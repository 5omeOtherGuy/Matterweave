#!/usr/bin/env python3
"""Collect separate profiling-overhead or 60/30Hz stationary pairs on one gen3 APK.

Raw evidence only: no optimization verdict and no exact app/SurfaceFlinger join.
Requires exclusive lead ownership of an unplugged phone and an already-invalid
base journal. Never backs up, overwrites or restores user saves.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import time

import collect_wetland_pair as core
from analyze_wetland_pairs import app_profile, presentation_intervals, process_cpu, read_jsonl
from validate_frame_profile import validate_profile

SOURCE = {'generator': 3, 'seed': 20260908, 'composition_hash': 'dfb9f40519a3c151'}
SCENE = {'cells': 34716467, 'instances': 8302}
PROFILE_ROWS = 240000
FORBIDDEN = {core.FIXTURE_REMOTE_NAME, core.PROFILE_REQUEST_NAME,
             core.GALLERY_MARKER_NAME, 'wetland-replay.json', *core.OUTRANKING_SLOTS}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + '\n')


def mode_plan(experiment, pairs):
    if experiment not in ('profiling', 'frame-cap'):
        raise ValueError('experiment must be profiling or frame-cap')
    plan = core.trial_plan(pairs)
    for trial in plan:
        variant = 'A' if trial['variant'] == 'reference' else 'B'
        profile = experiment != 'profiling' or variant == 'A'
        rate = 30 if experiment == 'frame-cap' and variant == 'B' else 60
        trial.update(experiment=experiment, variant=variant, profile=profile, frame_rate=rate)
        trial['name'] = (f"{experiment}-pair{trial['pair']}-{trial['position']}-{variant}-"
                         f"profile-{'on' if profile else 'off'}-{rate}hz")
    return plan


def check_fixture(raw):
    if len(raw) > 2 * 1024 * 1024:
        raise ValueError('fixture exceeds 2MiB')
    save = core._strict_json(raw, 'gen3 fixture')
    if not isinstance(save, dict):
        raise ValueError('fixture must be an object')
    if set(save) - {'version', 'generator', 'seed', 'edits', 'physics', 'yaw', 'pitch',
                    'shadows', 'frame_rate'}:
        raise ValueError('unknown fixture field')
    if (type(save.get('version')) is not int or save['version'] != 1
            or save.get('generator') != SOURCE['generator']
            or save.get('seed') != SOURCE['seed'] or save.get('edits') != []):
        raise ValueError('fixture must be version1/gen3/seed20260908 with empty edits')
    rate = save.get('frame_rate', 60)
    if type(rate) is not int or rate not in (30, 60):
        raise ValueError('frame_rate must be 30 or 60')
    physics = save.get('physics')
    if not isinstance(physics, dict) or physics.get('version') != 1:
        raise ValueError('physics snapshot must be version1')
    bodies, eye = physics.get('bodies'), physics.get('eye')
    if not isinstance(bodies, list) or len(bodies) != 6 or not all(isinstance(b, dict) for b in bodies):
        raise ValueError('fixture must hold six bodies')
    if not isinstance(eye, list) or len(eye) != 3:
        raise ValueError('eye must have three components')
    eye = [core._finite(v, 'eye', core.MAX_EYE_ABS) for v in eye]
    yaw = core._finite(save.get('yaw'), 'yaw')
    pitch = core._finite(save.get('pitch'), 'pitch', core.MAX_PITCH)
    if not isinstance(save.get('shadows'), bool):
        raise ValueError('shadows must be boolean')
    comparable = {k: v for k, v in save.items() if k != 'frame_rate'}
    canonical = json.dumps(comparable, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
    return {**SOURCE, 'sha256': hashlib.sha256(raw).hexdigest(),
            'comparison_sha256': hashlib.sha256(canonical).hexdigest(),
            'eye': eye, 'yaw': yaw, 'pitch': pitch, 'shadows': save['shadows'],
            'body_count': len(bodies), 'frame_rate': rate}


def stage_fixture(raw, frame_rate):
    check_fixture(raw)
    save = json.loads(raw)
    save['frame_rate'] = frame_rate
    staged = (json.dumps(save, allow_nan=False) + '\n').encode()
    check_fixture(staged)
    return staged


def read_build(path):
    build = core._strict_json(path.read_bytes(), 'build manifest')
    if not isinstance(build, dict) or not isinstance(build.get('scene'), dict):
        raise ValueError('build manifest must declare scene')
    if any(build['scene'].get(k) != v for k, v in SOURCE.items()):
        raise ValueError('build source must match frozen gen3 seed/composition')
    if 'expected_scene' in build and build['expected_scene'] != SCENE:
        raise ValueError('build scene counts differ')
    if not re.fullmatch(r'[0-9a-f]{40}', build.get('source_commit', '')):
        raise ValueError('build requires full source_commit')
    name = build.get('apk')
    if not isinstance(name, str) or Path(name).name != name or not name.endswith('.apk'):
        raise ValueError('apk must be a filename alongside build-manifest.json')
    apk = path.parent / name
    if core.sha256_file(apk) != build.get('apk_sha256'):
        raise ValueError('frozen APK hash differs from manifest')
    return {**build, 'apk_path': str(apk.resolve()), 'manifest_sha256': core.sha256_file(path)}


def preflight(device, out, staged, ownership):
    before = device.list_files()
    # Even an owned request left from a prior trial is stale, never reusable.
    blocked = FORBIDDEN & before
    if ownership.owns(core.FIXTURE_REMOTE_NAME):
        blocked.discard(core.FIXTURE_REMOTE_NAME)
    if blocked:
        raise core.TrialError(f'pre-existing control/fixture files: {sorted(blocked)}')
    base = device.pull_private(core.BASE_SAVE_NAME)
    if not core.base_save_is_invalid_for(base, SOURCE['generator']):
        raise core.TrialError('base must already be provably invalid for gen3; never overwritten')
    existing = device.pull_private(core.FIXTURE_REMOTE_NAME)
    if existing is not None and not ownership.owns(core.FIXTURE_REMOTE_NAME):
        raise core.TrialError('unowned recovery127 must not be overwritten')
    write_json(out/'preflight.json', {'files_before': sorted(before),
               'base_sha256': hashlib.sha256(base).hexdigest(),
               'base_invalid_for_generator': 3})
    ownership.claim(core.FIXTURE_REMOTE_NAME)
    device.push_private(core.FIXTURE_REMOTE_NAME, staged)
    if device.pull_private(core.FIXTURE_REMOTE_NAME) != staged:
        raise core.TrialError('staged fixture bytes differ')
    return before


def check_captures(captures, profile, consumed):
    if not consumed:
        raise core.TrialError('stale/unconsumed profile request invalidates trial')
    if len(captures) != int(profile):
        raise core.TrialError(f'profile={profile}: expected {int(profile)} new CSV, got {len(captures)}')
    if profile:
        valid = captures[0].get('profile', {})
        if not 0 < valid.get('row_count', 0) < PROFILE_ROWS:
            raise core.TrialError('CSV invalid or frame request exhausted before capture ended')


def check_camera(a, b):
    if math.dist(a['eye'], b['eye']) > .01:
        raise core.TrialError('stationary endpoint differs by more than0.01m')
    if any(abs(a[k] - b[k]) > 1e-6 for k in ('yaw', 'pitch')) or a['shadows'] != b['shadows']:
        raise core.TrialError('camera orientation or shadows differ')


def check_saved(raw, staged, rate):
    info = check_fixture(raw)
    if json.loads(raw).get('frame_rate') != rate:
        raise core.TrialError('app did not persist requested frame_rate explicitly')
    check_camera(info, check_fixture(staged))
    return info


def finish(device, out, before, staged, profile, rate):
    device.shell('input', 'keyevent', 'KEYCODE_HOME', timeout=20)
    time.sleep(3)
    device.shell('am', 'force-stop', device.package, timeout=30)
    after = device.list_files()
    captures = []
    for name in sorted(n for n in after - before if n.startswith('frame-profile')):
        if Path(name).name != name:
            raise core.TrialError('unsafe capture filename')
        raw = device.pull_private(name, timeout=180)
        if raw is None:
            raise core.TrialError(f'cannot pull {name}')
        target = out/name
        target.write_bytes(raw)
        entry = {'name': name, 'sha256': hashlib.sha256(raw).hexdigest(), 'bytes': len(raw)}
        try:
            entry['profile'] = validate_profile(target)
        except (ValueError, OSError) as error:
            entry['validation_error'] = str(error)
        captures.append(entry)
    session = device.pull_private(core.FIXTURE_REMOTE_NAME)
    if session is None:
        raise core.TrialError('owned session disappeared')
    (out/'session-after.json').write_bytes(session)
    result = {'captures': captures, 'files_after': sorted(after),
              'session_after_sha256': hashlib.sha256(session).hexdigest(),
              'profile_request_consumed': core.PROFILE_REQUEST_NAME not in after}
    write_json(out/'capture-result.json', result)  # retain even rejected outcomes
    check_captures(captures, profile, result['profile_request_consumed'])
    if (FORBIDDEN - {core.FIXTURE_REMOTE_NAME}) & after:
        raise core.TrialError('control/competing recovery appeared during capture')
    result['session_after'] = check_saved(session, staged, rate)
    return result


def check_pair(a, b):
    if a['experiment'] != b['experiment'] or a['pair'] != b['pair']:
        raise core.TrialError('different experiment or pair')
    expected = [p for p in mode_plan(a['experiment'], a['pair']) if p['pair'] == a['pair']]
    for actual, spec in zip((a, b), expected):
        if any(actual[k] != spec[k] for k in ('name', 'variant', 'profile', 'frame_rate', 'position')):
            raise core.TrialError('trial label or mode differs from planned single factor')
    if a['apk_sha256'] != b['apk_sha256'] or a['environment'] != b['environment']:
        raise core.TrialError('APK or environment differs across pair')
    if a['fixture']['comparison_sha256'] != b['fixture']['comparison_sha256']:
        raise core.TrialError('fixture differs beyond the frame_rate factor')
    check_camera(a['session_after'], b['session_after'])
    core.check_thermal_match(b['gate']['raw'], a['gate']['raw'])


def environment(device, out):
    result = core.record_environment(device, out)
    result['display_geometry'] = {key: device.shell('wm', key, timeout=20).strip()
                                  for key in ('size', 'density')}
    return result


def summarize(out, result, args):
    summary = {'app_whole_capture': None,
               'limits': ['No exact app/SF clock join; app includes startup/warmup.',
                          'Missing app/GPU metrics are not zero; no verdict.']}
    for key, operation in (
        ('presentation', lambda: presentation_intervals(read_jsonl(out/'surface-samples.jsonl'),
                                                       args.warmup, args.warmup + args.measure)),
        ('process_cpu', lambda: process_cpu(json.loads((out/'process-stat.json').read_text())))):
        try:
            summary[key] = operation()
        except (ValueError, KeyError, OSError, IndexError) as error:
            summary[key] = None
            summary[key + '_missing_reason'] = str(error)
    if result['captures']:
        summary['app_whole_capture'] = app_profile(out/result['captures'][0]['name'])
    else:
        summary['app_missing_reason'] = 'profiling disabled; no app CSV or GPU timings'
    write_json(out/'raw-summary.json', summary)


def run_trial(device, spec, build, raw, out, first, owned, args):
    out.mkdir()
    staged = stage_fixture(raw, spec['frame_rate'])
    (out/'fixture.json').write_bytes(staged)
    record = {**spec, 'fixture': check_fixture(staged), 'build': build,
              'apk_sha256': build['apk_sha256'], 'source': SOURCE, 'expected_scene': SCENE}
    write_json(out/'trial-request.json', record)
    core.install_and_verify(device, Path(build['apk_path']), build['apk_sha256'], out, args)
    record['environment'] = environment(device, out)
    if first and record['environment'] != first['environment']:
        raise core.TrialError('paired settings/display/device differ before launch')
    reference = first['gate']['raw'] if first else None
    readiness, window = core.record_idle_window(device, out, reference, args)
    before = preflight(device, out, staged, owned)
    stamp_before = device.run_as('stat', '-c', '%i:%y', 'files/' + core.FIXTURE_REMOTE_NAME).strip()
    if not stamp_before:
        raise core.TrialError('cannot establish fixture write identity')
    record['gate'] = core.gate_before_launch(device, out, readiness, window, reference, args)
    # Recheck requests immediately before launch, including a stale owned request.
    if (FORBIDDEN - {core.FIXTURE_REMOTE_NAME}) & device.list_files():
        raise core.TrialError('control request appeared before launch')
    if spec['profile']:
        owned.claim(core.PROFILE_REQUEST_NAME)
        request = f'{PROFILE_ROWS}\n'.encode()
        device.push_private(core.PROFILE_REQUEST_NAME, request)
        if device.pull_private(core.PROFILE_REQUEST_NAME) != request:
            raise core.TrialError('profile request bytes differ')
    logcat = log_file = None
    try:
        pid, loaded, logcat, log_file = core.enter_wetland(device, out, args, SCENE)
        record['scene_loaded'] = loaded
        record['collection'] = core.collect_window(device, out, pid, device.app_layer(), args)
    finally:
        if logcat is not None:
            logcat.terminate()
            try:
                logcat.wait(timeout=15)
            except subprocess.TimeoutExpired:
                logcat.kill()
                logcat.wait(timeout=15)
        if log_file is not None:
            log_file.close()
    result = finish(device, out, before, staged, spec['profile'], spec['frame_rate'])
    stamp_after = device.run_as('stat', '-c', '%i:%y', 'files/' + core.FIXTURE_REMOTE_NAME).strip()
    write_json(out/'fixture-write-observation.json', {'before': stamp_before, 'after': stamp_after})
    if not stamp_after or stamp_after == stamp_before:
        raise core.TrialError('app did not rewrite owned fixture; recovery selection unproven')
    if core.PROFILE_REQUEST_NAME in owned.owned:
        owned.owned.remove(core.PROFILE_REQUEST_NAME)  # verified consumed
    record.update(result=result, session_after=result['session_after'])
    end = out/'environment-after'
    end.mkdir()
    if environment(device, end) != record['environment']:
        raise core.TrialError('settings/display/device changed during trial')
    if first:
        check_pair(first, record)
    summarize(out, result, args)
    write_json(out/'trial.json', record)
    write_json(out/'collection-complete.json', {'name': spec['name'], **record['collection']})
    return record


def cleanup(device, out, owned):
    # Existing helper is best-effort: establish a successful stop before allowing
    # it to delete anything. On transport/stop failure preserve owned files too.
    try:
        device.shell('am', 'force-stop', device.package, timeout=30)
    except Exception as error:
        write_json(out/'cleanup-blocked.json', {'error': repr(error), 'retained': owned.owned})
        raise
    result = core.final_cleanup(device, out, owned)
    if result['errors']:
        raise core.TrialError(f"cleanup incomplete: {result['errors']}")


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('adb', 'build', 'fixture', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--experiment', choices=('profiling', 'frame-cap'), required=True)
    parser.add_argument('--pairs', type=int, default=3, help='AB/BA/AB; 1 is exploratory')
    parser.add_argument('--warmup', type=float, default=120.)
    parser.add_argument('--measure', type=float, default=120.)
    parser.set_defaults(max_observations=61, sample_interval=30.05, install_timeout=600.,
                        launch_timeout=60., load_timeout=180., chooser_delay=6.,
                        settle_delay=5., tap=(850, 780))
    return parser


def main(argv=None):
    args = core.check_args(build_parser().parse_args(argv))
    build, raw = read_build(args.build), args.fixture.read_bytes()
    fixture = check_fixture(raw)
    plan = mode_plan(args.experiment, args.pairs)
    args.out.mkdir(parents=True, exist_ok=False)
    write_json(args.out/'manifest.json', {'build': build, 'fixture': fixture, 'plan': plan,
               'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
               'reuse_sha256': {name: core.sha256_file(Path(__file__).with_name(name)) for name in
                               ('collect_wetland_pair.py', 'validate_conditions.py',
                                'validate_frame_profile.py', 'analyze_wetland_pairs.py')},
               'source': SOURCE, 'expected_scene': SCENE, 'serial': args.serial,
               'warmup_s': args.warmup, 'measure_s': args.measure,
               'readiness': {'observations_max': 61, 'interval_s': 30.05,
                             'reference': 'actual first-member pre-launch reading of EACH pair'},
               'collector_sha256': core.sha256_file(Path(__file__)),
               'claims': 'raw evidence only; experiments never combined into one verdict'})
    device, owned = core.Device(args.adb, args.serial, core.PACKAGE), core.Ownership()
    records, first = [], None
    try:
        for spec in plan:
            if spec['position'] == 1:
                first = None  # no thermal reference leaks from the preceding pair
            print(spec['name'], flush=True)
            record = run_trial(device, spec, build, raw, args.out/spec['name'], first, owned, args)
            records.append(record)
            if spec['position'] == 1:
                first = record
    except BaseException as error:
        write_json(args.out/'failure.json', {'error': repr(error),
                   'completed_trials': [r['name'] for r in records]})
        raise
    finally:
        cleanup(device, args.out, owned)
    write_json(args.out/'run-complete.json', {'trials': [r['name'] for r in records],
               'claims': 'collection only; interpretation and image review remain lead-owned'})
    return 0


if __name__ == '__main__':
    sys.exit(main())
