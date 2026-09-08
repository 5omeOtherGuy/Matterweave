"""Phone-free contract tests for the two independent stationary experiments."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import collect_wetland_modes as modes
from test_collect_wetland_pair import fixture, row


def gen3(**changes):
    value = json.loads(fixture(generator=3))
    value.update(changes)
    return json.dumps(value).encode()


class FakeDevice:
    package = modes.core.PACKAGE

    def __init__(self, extra=None):
        self.files = {modes.core.BASE_SAVE_NAME: b'{invalid', **(extra or {})}
        self.calls = []

    def list_files(self):
        return set(self.files)

    def pull_private(self, name, **kwargs):
        return self.files.get(name)

    def push_private(self, name, data):
        self.calls.append(('write', name))
        self.files[name] = data

    def shell(self, *args, **kwargs):
        self.calls.append(args)
        return ''


class ModesTest(unittest.TestCase):
    def test_order_and_single_factor(self):
        for experiment, expected in [('profiling', [(True, 60), (False, 60)]),
                                     ('frame-cap', [(True, 60), (True, 30)])]:
            plan = modes.mode_plan(experiment, 3)
            self.assertEqual([p['variant'] for p in plan], list('ABBAAB'))
            self.assertEqual([(p['profile'], p['frame_rate']) for p in plan[:2]], expected)
            self.assertEqual(len({p['name'] for p in plan}), 6)
            self.assertTrue(all(experiment in p['name'] for p in plan))
            self.assertEqual(len(modes.mode_plan(experiment, 1)), 2)
        for bad in (0, -1, True, 1.5):
            with self.assertRaises(ValueError):
                modes.mode_plan('profiling', bad)
        with self.assertRaises(ValueError):
            modes.mode_plan('combined', 3)

    def test_fixture_default_staging_and_identity(self):
        original = gen3()
        a = modes.stage_fixture(original, 60)
        b = modes.stage_fixture(original, 30)
        self.assertEqual(json.loads(a)['frame_rate'], 60)
        self.assertEqual(json.loads(b)['frame_rate'], 30)
        self.assertEqual(modes.check_fixture(a)['comparison_sha256'],
                         modes.check_fixture(b)['comparison_sha256'])
        self.assertNotEqual(modes.check_fixture(a)['sha256'], modes.check_fixture(b)['sha256'])
        self.assertNotIn('frame_rate', json.loads(original))
        for bad in (gen3(generator=2), gen3(seed=1), gen3(edits=[{}]),
                    gen3(frame_rate=59), gen3(frame_rate=True), gen3(pitch=float('nan'))):
            with self.assertRaises(ValueError):
                modes.check_fixture(bad)

    def test_build_source_and_hash(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)/'build-manifest.json'
            apk = Path(tmp)/'app.apk'
            apk.write_bytes(b'frozen')
            data = {'apk': 'app.apk', 'apk_sha256': modes.core.sha256_file(apk),
                    'source_commit': 'a'*40, 'scene': dict(modes.SOURCE)}
            path.write_text(json.dumps(data))
            self.assertEqual(modes.read_build(path)['apk_sha256'], data['apk_sha256'])
            for key, bad in [('generator', 2), ('seed', 3), ('composition_hash', 'wrong')]:
                broken = copy.deepcopy(data)
                broken['scene'][key] = bad
                path.write_text(json.dumps(broken))
                with self.assertRaises(ValueError):
                    modes.read_build(path)
            path.write_text(json.dumps(data))
            apk.write_bytes(b'changed')
            with self.assertRaises(ValueError):
                modes.read_build(path)

    def test_preexisting_files_rejected_without_ownership(self):
        for name in modes.FORBIDDEN:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                device = FakeDevice({name: b'unowned'})
                owned = modes.core.Ownership()
                with self.assertRaises(modes.core.TrialError):
                    modes.preflight(device, Path(tmp), gen3(), owned)
                self.assertEqual(owned.owned, [])
                self.assertEqual(device.files[name], b'unowned')
                self.assertEqual(device.calls, [])

    def test_base_must_already_be_invalid_and_stage_verified(self):
        for base in (None, gen3()):
            with tempfile.TemporaryDirectory() as tmp:
                device = FakeDevice()
                if base is None:
                    device.files.clear()
                else:
                    device.files[modes.core.BASE_SAVE_NAME] = base
                with self.assertRaises(modes.core.TrialError):
                    modes.preflight(device, Path(tmp), gen3(), modes.core.Ownership())
                self.assertEqual(device.calls, [])
        with tempfile.TemporaryDirectory() as tmp:
            device, owned = FakeDevice({'wetland-session.json.recovery-1.json': b'invalid'}), modes.core.Ownership()
            modes.preflight(device, Path(tmp), gen3(), owned)
            self.assertEqual(owned.owned, [modes.core.FIXTURE_REMOTE_NAME])
            self.assertEqual(device.files['wetland-session.json.recovery-1.json'], b'invalid')

    def test_capture_expectations_and_stale_request(self):
        modes.check_captures([], False, True)
        valid = [{'name': 'frame-profile-v2-1.csv', 'profile': {'row_count': 20}}]
        modes.check_captures(valid, True, True)
        for captures, enabled, consumed in [(valid, False, True), ([], True, True),
                                           (valid*2, True, True), (valid, True, False),
                                           ([], False, False), ([{'name': 'bad.csv'}], True, True),
                                           ([{'profile': {'row_count': modes.PROFILE_ROWS}}], True, True)]:
            with self.assertRaises(modes.core.TrialError):
                modes.check_captures(captures, enabled, consumed)

    def test_saved_mode_and_camera_must_match(self):
        raw = modes.stage_fixture(gen3(), 30)
        modes.check_saved(raw, raw, 30)
        for changed in (gen3(frame_rate=60), gen3(frame_rate=30, yaw=.1),
                        gen3(frame_rate=30, shadows=False)):
            with self.assertRaises(modes.core.TrialError):
                modes.check_saved(changed, raw, 30)
        changed = json.loads(raw)
        changed['physics']['eye'][0] += .011
        with self.assertRaises(modes.core.TrialError):
            modes.check_saved(json.dumps(changed).encode(), raw, 30)
        changed.pop('frame_rate')
        with self.assertRaises(modes.core.TrialError):
            modes.check_saved(json.dumps(changed).encode(), raw, 30)

    def test_pair_conditions_match_actual_first_not_shared_reference(self):
        a = {'environment': {'settings': {'brightness': '100'}},
             'apk_sha256': 'same', 'fixture': modes.check_fixture(gen3()),
             'session_after': modes.check_fixture(gen3()),
             'gate': {'raw': row(300, 32)}, **modes.mode_plan('profiling', 1)[0]}
        b = {**copy.deepcopy(a), **modes.mode_plan('profiling', 1)[1]}
        modes.check_pair(a, b)
        for key, bad in [('apk_sha256', 'other'), ('environment', {}),
                         ('frame_rate', 30), ('name', 'wrong-label'),
                         ('gate', {'raw': row(320, 36)})]:
            broken = {**b, key: bad}
            with self.assertRaises((ValueError, modes.core.TrialError)):
                modes.check_pair(a, broken)

    @patch.object(modes.core.time, 'sleep')
    def test_finish_off_keeps_metrics_missing_and_rejects_csv(self, sleep):
        with tempfile.TemporaryDirectory() as tmp:
            staged = modes.stage_fixture(gen3(), 60)
            device = FakeDevice({modes.core.FIXTURE_REMOTE_NAME: staged})
            before = device.list_files()
            result = modes.finish(device, Path(tmp), before, staged, False, 60)
            self.assertEqual(result['captures'], [])
            device.files['frame-profile-v2-new.csv'] = b'invalid'
            with self.assertRaises(modes.core.TrialError):
                modes.finish(device, Path(tmp), before, staged, False, 60)
            self.assertTrue((Path(tmp)/'frame-profile-v2-new.csv').exists())


class OrchestrationTest(unittest.TestCase):
    def args(self, out):
        return modes.build_parser().parse_args(['--adb', '/no/adb', '--serial', 'fake',
            '--build', str(out/'build-manifest.json'), '--fixture', str(out/'input.json'),
            '--out', str(out/'run'), '--experiment', 'profiling'])

    def inputs(self, root):
        (root/'input.json').write_bytes(gen3())
        (root/'app.apk').write_bytes(b'frozen')
        build = {'apk': 'app.apk', 'apk_sha256': modes.core.sha256_file(root/'app.apk'),
                 'source_commit': 'a'*40, 'scene': dict(modes.SOURCE)}
        modes.write_json(root/'build-manifest.json', build)
        return modes.read_build(root/'build-manifest.json')

    def test_main_resets_first_member_for_each_pair(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.inputs(root)
            args = self.args(root)
            references = []
            def trial(device, spec, build, raw, out, first, owned, settings):
                references.append(first['name'] if first else None)
                return spec
            with patch.object(modes, 'run_trial', side_effect=trial), \
                    patch.object(modes, 'cleanup') as cleanup:
                modes.main(['--adb', '/no/adb', '--serial', 'fake', '--build', str(args.build),
                    '--fixture', str(args.fixture), '--out', str(args.out),
                    '--experiment', 'profiling'])
            plan = modes.mode_plan('profiling', 3)
            self.assertEqual(references, [None, plan[0]['name'], None, plan[2]['name'], None, plan[4]['name']])
            cleanup.assert_called_once()
            self.assertTrue((args.out/'run-complete.json').exists())
            with self.assertRaises(FileExistsError):
                modes.main(['--adb', '/no/adb', '--serial', 'fake', '--build', str(args.build),
                    '--fixture', str(args.fixture), '--out', str(args.out), '--experiment', 'profiling'])

    def test_main_failure_is_retained_and_cleanup_runs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.inputs(root)
            args = self.args(root)
            with patch.object(modes, 'run_trial', side_effect=RuntimeError('fake failure')), \
                    patch.object(modes, 'cleanup') as cleanup, self.assertRaises(RuntimeError):
                modes.main(['--adb', '/no/adb', '--serial', 'fake', '--build', str(args.build),
                    '--fixture', str(args.fixture), '--out', str(args.out), '--experiment', 'profiling'])
            cleanup.assert_called_once()
            self.assertTrue((args.out/'failure.json').exists())
            self.assertFalse((args.out/'run-complete.json').exists())

    def test_run_trial_on_and_off_and_saved_write_proof(self):
        for profile, rewritten in [(True, True), (False, True), (True, False)]:
            with self.subTest(profile=profile, rewritten=rewritten), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                build, args = self.inputs(root), self.args(root)
                spec = modes.mode_plan('profiling', 1)[0 if profile else 1]
                device = FakeDevice()
                device.app_layer = Mock(return_value='fake-layer')
                device.run_as = Mock(side_effect=['inode:before', 'inode:after' if rewritten else 'inode:before'])
                logcat, log_file = Mock(), Mock()
                def enter(*unused):
                    device.files.pop(modes.core.PROFILE_REQUEST_NAME, None)
                    if profile:
                        device.files['frame-profile-v2-fake.csv'] = b'fake CSV; validator mocked'
                    return '123', modes.SCENE, logcat, log_file
                with patch.object(modes.core, 'install_and_verify'), \
                        patch.object(modes, 'environment', return_value={'same': True}), \
                        patch.object(modes.core, 'record_idle_window', return_value=({}, [])), \
                        patch.object(modes.core, 'gate_before_launch', return_value={'raw': row(300, 32)}), \
                        patch.object(modes.core, 'enter_wetland', side_effect=enter), \
                        patch.object(modes.core, 'collect_window', return_value={'elapsed_s': 240}), \
                        patch.object(modes, 'validate_profile', return_value={'row_count': 20}), \
                        patch.object(modes, 'summarize'), patch.object(modes.core.time, 'sleep'):
                    if rewritten:
                        result = modes.run_trial(device, spec, build, gen3(), root/spec['name'],
                                                 None, modes.core.Ownership(), args)
                        self.assertEqual(result['profile'], profile)
                    else:
                        with self.assertRaisesRegex(modes.core.TrialError, 'did not rewrite'):
                            modes.run_trial(device, spec, build, gen3(), root/spec['name'],
                                            None, modes.core.Ownership(), args)
                logcat.terminate.assert_called_once()
                logcat.wait.assert_called_once()
                log_file.close.assert_called_once()

    def test_summary_off_and_missing_sf_remain_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            args = self.args(root)
            modes.summarize(root, {'captures': []}, args)
            summary = json.loads((root/'raw-summary.json').read_text())
            self.assertIsNone(summary['app_whole_capture'])
            self.assertIsNone(summary['presentation'])
            self.assertIn('presentation_missing_reason', summary)
            samples = [{'elapsed_s': 121., 'exit': 0, 'raw': '16666667\n0 1000000000 0\n0 1016000000 0'}]
            modes.write_json(root/'process-stat.json', {})
            with patch.object(modes, 'read_jsonl', return_value=samples), \
                    patch.object(modes, 'process_cpu', return_value={'cpu_seconds': 1}), \
                    patch.object(modes, 'app_profile', return_value={'whole': True}):
                modes.summarize(root, {'captures': [{'name': 'fake.csv'}]}, args)
            summary = json.loads((root/'raw-summary.json').read_text())
            self.assertEqual(summary['presentation']['supported']['interval_count'], 1)
            self.assertEqual(summary['app_whole_capture'], {'whole': True})

    def test_cleanup_requires_successful_stop(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            device = FakeDevice()
            owned = modes.core.Ownership()
            owned.claim(modes.core.FIXTURE_REMOTE_NAME)
            device.shell = Mock(side_effect=RuntimeError('stop failed'))
            with patch.object(modes.core, 'final_cleanup') as cleanup, self.assertRaises(RuntimeError):
                modes.cleanup(device, root, owned)
            cleanup.assert_not_called()
            self.assertTrue((root/'cleanup-blocked.json').exists())
            device.shell = Mock(return_value='')
            with patch.object(modes.core, 'final_cleanup', return_value={'errors': []}) as cleanup:
                modes.cleanup(device, root, owned)
            cleanup.assert_called_once()
            with patch.object(modes.core, 'final_cleanup', return_value={'errors': ['failed']}), \
                    self.assertRaises(modes.core.TrialError):
                modes.cleanup(device, root, owned)

    def test_rejects_stale_owned_profile_request(self):
        with tempfile.TemporaryDirectory() as tmp:
            owned = modes.core.Ownership()
            owned.claim(modes.core.PROFILE_REQUEST_NAME)
            device = FakeDevice({modes.core.PROFILE_REQUEST_NAME: b'240000'})
            with self.assertRaises(modes.core.TrialError):
                modes.preflight(device, Path(tmp), gen3(), owned)


if __name__ == '__main__':
    unittest.main()
