"""Phone-free contract tests for the two independent stationary experiments."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

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


if __name__ == '__main__':
    unittest.main()
