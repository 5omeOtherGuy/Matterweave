#!/usr/bin/env python3
"""Run a normal Android wetland route using an exclusively owned recovery fixture.

The lead must own the idle phone. This functional check does not establish a
matched or sustained performance result. It preserves all pre-existing saves.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import subprocess
import time
from types import SimpleNamespace

from collect_wetland_pair import (Device, Ownership, base_save_is_invalid_for,
                                 enter_wetland, final_cleanup, install_and_verify,
                                 record_environment, sha256_file)
from validate_frame_profile import validate_profile

FIXTURE = 'wetland-session.json.recovery-127.json'
REQUEST = 'wetland-replay.json'
PROFILE = 'profile-frames.txt'


def validate_report(report, route, points, expected, interrupted):
    assert report['request'] == {'version': 1, 'route': route}, report
    assert report['route_point_count'] == len(points), report
    assert report['outcome'] == expected, report
    assert report['physics_step_count'] > 0, report
    if expected == 'CANCEL':
        assert interrupted, 'route cancelled before the intended interruption'
    else:
        assert report['next_waypoint'] == len(points), report
        assert report['max_index'] == len(points) - 1, report
        eye, target = report['actual_eye'], points[-1]
        assert math.dist([eye[0], eye[2]], [target[0], target[2]]) <= .201, report
        assert abs(eye[1] - target[1] - 1.7) <= 1.001, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--adb', type=Path, required=True)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--build', type=Path, required=True, help='frozen build-manifest.json')
    parser.add_argument('--fixture', type=Path, required=True, help='six-body source session; eye set to route start')
    parser.add_argument('--scene', type=Path, required=True, help='recorded full scene manifest')
    parser.add_argument('--route', choices=['ground', 'elevated'], required=True)
    parser.add_argument('--interrupt', choices=['touch', 'home'])
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    build = json.loads(args.build.read_text())
    scene = json.loads(args.scene.read_text())
    assert scene['generator_version'] == build['scene']['generator'] == 3
    assert scene['composition_hash'] == 'dfb9f40519a3c151' and scene['seed'] == 20260908
    assert scene['composition_hash'] == build['scene']['composition_hash']
    points = scene['navigation'][args.route + '_route_m']
    fixture = json.loads(args.fixture.read_text())
    assert fixture['generator'] == scene['generator_version']
    assert fixture['seed'] == scene['seed'] and fixture['edits'] == []
    assert len(fixture['physics']['bodies']) == 6
    fixture['physics']['eye'] = [points[0][0], points[0][1] + 1.7, points[0][2]]
    fixture['yaw'], fixture['pitch'], fixture['shadows'] = 0., -.08, True
    data = (json.dumps(fixture, allow_nan=False) + '\n').encode()
    (args.out/'fixture.json').write_bytes(data)
    apk = args.build.parent/build['apk']
    assert sha256_file(apk) == build['apk_sha256']
    settings = SimpleNamespace(install_timeout=600., launch_timeout=60., load_timeout=180.,
                               chooser_delay=6., settle_delay=1., tap=(850, 780))
    device = Device(args.adb, args.serial, 'dev.matterweave.explorer')
    owned = Ownership()
    logcat = log_file = None
    record = {'build': build, 'route': args.route, 'interrupt': args.interrupt,
              'fixture_sha256': hashlib.sha256(data).hexdigest(),
              'scene_manifest_sha256': sha256_file(args.scene),
              'script_sha256': sha256_file(Path(__file__)), 'result': 'NOT COMPLETED'}
    try:
        install_and_verify(device, apk, build['apk_sha256'], args.out, settings)
        device.shell('am', 'force-stop', device.package)
        record['environment'] = record_environment(device, args.out)
        before = device.list_files()
        forbidden = {FIXTURE, REQUEST, PROFILE, 'detail-gallery.txt',
                     'wetland-session.json.recovery-128.json'} & before
        if forbidden:
            raise RuntimeError(f'pre-existing control/fixture files: {sorted(forbidden)}')
        base = device.pull_private('wetland-session.json')
        if base is None or not base_save_is_invalid_for(base, fixture['generator']):
            raise RuntimeError('fixture would not be selected; primary must already be incompatible')
        (args.out/'preflight.json').write_text(json.dumps({'files': sorted(before),
            'base_sha256': hashlib.sha256(base).hexdigest()}, indent=2)+'\n')
        for name, content in [(FIXTURE, data), (PROFILE, b'90000\n'),
                              (REQUEST, json.dumps({'version': 1, 'route': args.route}).encode())]:
            owned.claim(name)
            device.push_private(name, content)
            assert device.pull_private(name) == content
        pid, loaded, logcat, log_file = enter_wetland(device, args.out, settings,
                                                     {'cells': 34716467, 'instances': 8302})
        record.update(pid=pid, scene_loaded=loaded)
        started = time.monotonic()
        deadline = started + (650 if args.route == 'ground' else 150)
        last_health = last_image = -100.
        interrupted = False
        terminal = None
        while time.monotonic() < deadline:
            elapsed = time.monotonic() - started
            reports = sorted(n for n in device.list_files() - before
                             if n.startswith('wetland-replay-result-') and n.endswith('.json'))
            if len(reports) > 1:
                raise RuntimeError('multiple unexpected replay reports')
            if reports:
                raw = device.pull_private(reports[0])
                report = json.loads(raw)
                with (args.out/'replay-observations.jsonl').open('a') as out:
                    out.write(json.dumps({'host_elapsed_s': elapsed, 'report': report})+'\n')
                print(f"{elapsed:.1f}s {report['outcome']} waypoint {report['next_waypoint']}", flush=True)
                if report['outcome'] != 'RUNNING':
                    terminal = report
                    (args.out/reports[0]).write_bytes(raw)
                    break
                if args.interrupt and not interrupted and elapsed >= 5.:
                    if args.interrupt == 'home':
                        device.shell('input', 'keyevent', 'KEYCODE_HOME')
                    else:
                        device.shell('input', 'tap', '410', '1160')
                    interrupted = True
                    record['interruption_host_elapsed_s'] = elapsed
            if not interrupted and elapsed - last_health >= 30.:
                health = {'elapsed_s': elapsed, 'data': {service: device.shell('dumpsys', service)
                           for service in ['battery', 'thermalservice']}}
                health['data']['meminfo'] = device.shell('dumpsys', 'meminfo', device.package)
                battery = health['data']['battery']
                assert all(f'{power} powered: false' in battery for power in ['AC','USB','Wireless','Dock'])
                assert device.foreground()
                with (args.out/'health.jsonl').open('a') as out:
                    out.write(json.dumps(health)+'\n')
                last_health = elapsed
            if not interrupted and elapsed - last_image >= 30.:
                device.screenshot(args.out/f'route-{int(elapsed):04d}.png')
                last_image = elapsed
            time.sleep(5.)
        if terminal is None:
            raise RuntimeError('no terminal report before supervisor deadline')
        expected = 'CANCEL' if args.interrupt else 'PASS'
        validate_report(terminal, args.route, points, expected, interrupted)
        record['terminal'] = terminal
        if not interrupted:
            device.screenshot(args.out/'terminal.png')
        device.shell('input', 'keyevent', 'KEYCODE_HOME')
        time.sleep(2.)
        device.shell('am', 'force-stop', device.package)
        session = device.pull_private(FIXTURE)
        (args.out/'session-after.json').write_bytes(session)
        saved = json.loads(session)
        assert saved['edits'] == [] and saved['generator'] == fixture['generator']
        captures = sorted(n for n in device.list_files() - before
                          if n.startswith('frame-profile-v2-') and n.endswith('.csv'))
        assert len(captures) == 1, captures
        capture = args.out/captures[0]
        capture.write_bytes(device.pull_private(captures[0]))
        record['capture'] = {'name': capture.name, 'sha256': sha256_file(capture),
                             'validation': validate_profile(capture)}
        record['result'] = 'PASS'
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        if logcat is not None:
            logcat.terminate()
            try:
                logcat.wait(timeout=10)
            except subprocess.TimeoutExpired:
                logcat.kill(); logcat.wait(timeout=5)
        if log_file is not None:
            log_file.close()
        record['cleanup'] = final_cleanup(device, args.out, owned)
        if record['cleanup']['errors']:
            record['result'] = 'FAIL_CLEANUP'
        (args.out/'result.json').write_text(json.dumps(record, indent=2, allow_nan=False)+'\n')
        if record['cleanup']['errors']:
            raise RuntimeError(f"owned fixture cleanup failed: {record['cleanup']['errors']}")


if __name__ == '__main__':
    main()
