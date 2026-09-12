#!/usr/bin/env python3
"""Phone-free tests for the paired wetland collector's decision helpers.

These cover the gates that decide whether a trial may run at all: pairing
order, fixture validity, base-save invalidity (so the recovery slot is really
selected), the real scene load line, and the thermal matching bound.
"""
import copy
import json
import tempfile
import unittest
from pathlib import Path

from collect_wetland_pair import (
    EXPECTED_SCENE,
    FIXTURE_REMOTE_NAME,
    PROFILE_REQUEST_NAME,
    SAME_BUILD_LABEL,
    Ownership,
    TrialError,
    base_save_is_invalid_for,
    build_parser,
    capture_state_plan,
    check_args,
    check_fixture,
    check_identical_build,
    finish_and_pull,
    check_scene_counts,
    check_thermal_match,
    final_cleanup,
    parse_wetland_loaded,
    preflight_files,
    resolve_expected_scene,
    trial_plan,
)

BATTERY = """Current Battery Service state:
  AC powered: false
  USB powered: false
  Wireless powered: false
  Dock powered: false
  status: 3
  level: 71
  temperature: {tenths}
"""

THERMAL = """IsStatusOverride: false
Thermal Status: 0
Current temperatures from HAL:
\tTemperature{{mValue={skin}, mType=3, mName=skin, mStatus=0}}
Current cooling devices from HAL:
"""


def row(battery_tenths, skin_c):
    return {"elapsed_s": 0.0, "data": {
        "battery": BATTERY.format(tenths=battery_tenths),
        "thermalservice": THERMAL.format(skin=skin_c)}}


def fixture(**overrides):
    save = {
        "version": 1, "generator": 2, "seed": 20260908,
        "edits": [],
        "physics": {"version": 1, "eye": [46.0, 15.6, 66.0], "bodies": [
            {"position": [90.5, 18.52, 58.0]} for _ in range(6)]},
        "yaw": 0.0, "pitch": -0.08, "shadows": True,
    }
    save.update(copy.deepcopy(overrides))
    return json.dumps(save).encode()


class TrialPlanTest(unittest.TestCase):
    def test_three_pairs_alternate_ab_ba_ab(self):
        plan = trial_plan(3)
        self.assertEqual([t["variant"] for t in plan],
                         ["reference", "candidate", "candidate", "reference",
                          "reference", "candidate"])
        self.assertEqual([t["trial"] for t in plan], [1, 2, 3, 4, 5, 6])
        self.assertEqual(len({t["name"] for t in plan}), 6)

    def test_rejects_nonpositive(self):
        for bad in (0, -1, True, 1.0):
            with self.assertRaises(ValueError):
                trial_plan(bad)


class FixtureTest(unittest.TestCase):
    def test_accepts_expected_benchmark_session(self):
        info = check_fixture(fixture())
        self.assertEqual(info["generator"], 2)
        self.assertEqual(info["body_count"], 6)
        self.assertEqual(info["eye"], [46.0, 15.6, 66.0])
        self.assertTrue(info["shadows"])
        self.assertEqual(len(info["sha256"]), 64)

    def test_rejects_wrong_generator_and_body_count(self):
        with self.assertRaises(ValueError):
            check_fixture(fixture(generator=1))
        five = json.loads(fixture())
        five["physics"]["bodies"].pop()
        with self.assertRaises(ValueError):
            check_fixture(json.dumps(five).encode())

    def test_rejects_malformed_input(self):
        for bad in (b"", b"not json", b"[]", fixture(version=2), fixture(shadows="yes"),
                    fixture(pitch="x")):
            with self.assertRaises(ValueError):
                check_fixture(bad)


class BaseSaveTest(unittest.TestCase):
    def test_generation_1_save_is_invalid_for_generator_2(self):
        self.assertTrue(base_save_is_invalid_for(fixture(generator=1), 2))

    def test_missing_base_does_not_reach_recovery_slots(self):
        # load() returns Ok(None) for a missing base: the app starts a fresh
        # session and never scans recoveries, so the fixture would be ignored.
        self.assertFalse(base_save_is_invalid_for(None, 2))

    def test_corrupt_save_is_invalid(self):
        self.assertTrue(base_save_is_invalid_for(b"{oops", 2))
        self.assertTrue(base_save_is_invalid_for(b"\xff\xfe", 2))

    def test_matching_generator_blocks_the_run(self):
        self.assertFalse(base_save_is_invalid_for(fixture(), 2))


class StrictFixtureTest(unittest.TestCase):
    def test_rejects_non_finite_numbers(self):
        for bad in (b'{"version":1,"generator":2,"seed":20260908,"edits":[1],'
                    b'"physics":{"version":1,"eye":[NaN,1,1],"bodies":[1,2,3,4,5,6]},'
                    b'"yaw":0.0,"pitch":-0.08,"shadows":true}',
                    b'{"version":1,"generator":2,"seed":20260908,"edits":[1],'
                    b'"physics":{"version":1,"eye":[1,1,1],"bodies":[1,2,3,4,5,6]},'
                    b'"yaw":Infinity,"pitch":-0.08,"shadows":true}'):
            with self.assertRaises(ValueError):
                check_fixture(bad)

    def test_rejects_out_of_range_pitch_and_eye(self):
        with self.assertRaises(ValueError):
            check_fixture(fixture(pitch=2.0))
        far = json.loads(fixture())
        far["physics"]["eye"] = [1e9, 0.0, 0.0]
        with self.assertRaises(ValueError):
            check_fixture(json.dumps(far).encode())

    def test_rejects_wrong_seed_and_nonempty_edits(self):
        with self.assertRaises(ValueError):
            check_fixture(fixture(seed=7))
        with self.assertRaises(ValueError):
            check_fixture(fixture(edits=[{"instance": "i_shell_5_8_3", "cell": [24, 3, 13], "material": 0}]))


class ExpectedSceneTest(unittest.TestCase):
    def test_small_map_is_rejected(self):
        with self.assertRaises(TrialError):
            check_scene_counts({"cells": 10, "instances": 2}, EXPECTED_SCENE)
        with self.assertRaises(TrialError):
            check_scene_counts(None, EXPECTED_SCENE)

    def test_exact_frozen_counts_accepted(self):
        self.assertEqual(check_scene_counts(dict(EXPECTED_SCENE), EXPECTED_SCENE),
                         EXPECTED_SCENE)

    def test_manifest_override_and_unknown_composition(self):
        known = {"composition_hash": "f458591e7b345546"}
        self.assertEqual(resolve_expected_scene({"candidate": dict(known),
                                                 "reference": dict(known)}),
                         EXPECTED_SCENE)
        with self.assertRaises(SystemExit):
            resolve_expected_scene({"candidate": {"composition_hash": "deadbeef"},
                                    "reference": {"composition_hash": "deadbeef"}})
        scene = {"cells": 5, "instances": 2}
        self.assertEqual(resolve_expected_scene(
            {"candidate": {"composition_hash": "x", "expected_scene": dict(scene)},
             "reference": {"composition_hash": "x", "expected_scene": dict(scene)}}), scene)


COMMON_ARGS = ["--adb", "/bin/true", "--serial", "phone:1",
               "--fixture", "/tmp/fixture.json", "--out", "/tmp/out"]
PAIRED_ARGS = COMMON_ARGS + ["--candidate-apk", "/tmp/c.apk", "--reference-apk", "/tmp/r.apk"]
ONOFF_ARGS = COMMON_ARGS + ["--mode", "capture-on-off", "--apk", "/tmp/one.apk"]


def parse(argv):
    return check_args(build_parser().parse_args(argv))


class ArgumentBoundsTest(unittest.TestCase):
    def test_accepts_defaults(self):
        args = parse(PAIRED_ARGS)
        self.assertEqual(args.mode, "paired")
        self.assertIsNone(args.apk)

    def test_rejects_bad_timing_and_windows(self):
        for extra in (["--warmup", "0"], ["--measure", "nan"],
                      ["--load-timeout", "inf"], ["--measure", "-1"],
                      ["--max-observations", "62"], ["--max-observations", "0"],
                      ["--sample-interval", "5"], ["--sample-interval", "40"],
                      ["--pairs", "0"]):
            with self.assertRaises(SystemExit, msg=extra):
                parse(PAIRED_ARGS + extra)


class CaptureModeArgumentTest(unittest.TestCase):
    def test_capture_on_off_takes_a_single_build(self):
        args = parse(ONOFF_ARGS)
        self.assertEqual(args.mode, "capture-on-off")
        self.assertEqual(str(args.apk), "/tmp/one.apk")
        self.assertIsNone(args.candidate_apk)

    def test_rejects_mixed_or_missing_build_arguments(self):
        for argv in (COMMON_ARGS + ["--mode", "capture-on-off"],
                     ONOFF_ARGS + ["--candidate-apk", "/tmp/c.apk"],
                     ONOFF_ARGS + ["--reference-apk", "/tmp/r.apk"],
                     PAIRED_ARGS + ["--apk", "/tmp/one.apk"],
                     COMMON_ARGS + ["--candidate-apk", "/tmp/c.apk"],
                     COMMON_ARGS):
            with self.assertRaises(SystemExit, msg=argv):
                parse(argv)


class CaptureStatePlanTest(unittest.TestCase):
    def test_states_alternate_within_pairs_and_share_one_build(self):
        plan = capture_state_plan(3)
        self.assertEqual([t["capture_state"] for t in plan],
                         ["off", "on", "on", "off", "off", "on"])
        self.assertEqual({t["variant"] for t in plan}, {SAME_BUILD_LABEL})
        self.assertEqual([t["trial"] for t in plan], [1, 2, 3, 4, 5, 6])
        self.assertEqual(len({t["name"] for t in plan}), 6)
        self.assertEqual(plan[0]["name"], "pair1-1-capture-off")

    def test_paired_plan_states_capture_on_explicitly(self):
        self.assertEqual({t["capture_state"] for t in trial_plan(2)}, {"on"})

    def test_rejects_nonpositive(self):
        for bad in (0, -1, True, 2.0):
            with self.assertRaises(ValueError):
                capture_state_plan(bad)


class IdenticalBuildTest(unittest.TestCase):
    def install(self, installed, expected=None):
        return {"installed_sha256": installed, "expected_sha256": expected or installed}

    def test_same_installed_bytes_accepted(self):
        self.assertEqual(check_identical_build([self.install("a" * 64)] * 2), "a" * 64)

    def test_rejects_differing_or_mislabeled_builds(self):
        for installs in ([],
                         [self.install("a" * 64), self.install("b" * 64)],
                         [self.install("a" * 64, "b" * 64)]):
            with self.assertRaises(TrialError, msg=installs):
                check_identical_build(installs)


class FakeDevice:
    """In-memory app-private file store with fault injection."""

    def __init__(self, files=None, fail_on_push=False):
        self.package = "dev.matterweave.explorer"
        self.files = dict(files or {})
        self.fail_on_push = fail_on_push
        self.force_stopped = 0
        self.calls = []

    def list_files(self, timeout=20):
        return set(self.files)

    def pull_private(self, name, timeout=60):
        return self.files.get(name)

    def push_private(self, name, data, timeout=60):
        self.calls.append(("push", name))
        if self.fail_on_push:
            self.files[name] = data[: len(data) // 2]  # partial write, then failure
            raise TrialError("injected write failure")
        self.files[name] = data

    def run_as(self, *args, timeout=20, check=True):
        self.calls.append(("run_as", args))
        if args[:2] == ("rm", "-f"):
            self.files.pop(args[2].removeprefix("files/"), None)
        return ""

    def shell(self, *args, timeout=20, check=True):
        self.calls.append(("shell", args))
        if args[:2] == ("am", "force-stop"):
            self.force_stopped += 1
        return ""


USER_BASE = fixture(generator=1)          # the user's old generation-1 session
USER_RECOVERY = b'{"user recovery 1 experiment"}'


class OwnershipCleanupTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.out = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def cleanup(self, device, ownership):
        return final_cleanup(device, self.out, ownership, log=lambda _m: None)

    def test_preexisting_fixture_rejected_and_preserved(self):
        device = FakeDevice({"wetland-session.json": USER_BASE,
                             FIXTURE_REMOTE_NAME: b"someone else's slot 127"})
        ownership = Ownership()
        with self.assertRaises(TrialError):
            preflight_files(device, self.out, fixture(), ownership)
        self.assertEqual(ownership.owned, [])
        status = self.cleanup(device, ownership)
        self.assertEqual(status["removed"], [])
        self.assertEqual(device.files[FIXTURE_REMOTE_NAME], b"someone else's slot 127")
        self.assertEqual(device.files["wetland-session.json"], USER_BASE)

    def test_preexisting_profile_request_rejected_and_preserved(self):
        device = FakeDevice({"wetland-session.json": USER_BASE,
                             PROFILE_REQUEST_NAME: b"500\n"})
        ownership = Ownership()
        with self.assertRaises(TrialError):
            preflight_files(device, self.out, fixture(), ownership)
        self.cleanup(device, ownership)
        self.assertEqual(device.files[PROFILE_REQUEST_NAME], b"500\n")

    def test_error_before_creation_removes_nothing(self):
        device = FakeDevice({"wetland-session.json": USER_BASE,
                             "detail-gallery.txt": b"tile source\n",
                             "wetland-session.json.recovery-1.json": USER_RECOVERY})
        ownership = Ownership()
        with self.assertRaises(TrialError):
            preflight_files(device, self.out, fixture(), ownership)
        status = self.cleanup(device, ownership)
        self.assertEqual(status["removed"], [])
        self.assertEqual(sorted(device.files), sorted(
            ["wetland-session.json", "detail-gallery.txt",
             "wetland-session.json.recovery-1.json"]))

    def test_partial_write_is_owned_and_removed_user_files_kept(self):
        device = FakeDevice({"wetland-session.json": USER_BASE,
                             "world.json": b"user world",
                             "wetland-session.json.recovery-1.json": USER_RECOVERY},
                            fail_on_push=True)
        ownership = Ownership()
        with self.assertRaises(TrialError):
            preflight_files(device, self.out, fixture(), ownership)
        self.assertEqual(ownership.owned, [FIXTURE_REMOTE_NAME])
        self.assertIn(FIXTURE_REMOTE_NAME, device.files)  # half written
        status = self.cleanup(device, ownership)
        self.assertEqual(status["removed"], ["files/" + FIXTURE_REMOTE_NAME])
        self.assertNotIn(FIXTURE_REMOTE_NAME, device.files)
        self.assertEqual(device.files["world.json"], b"user world")
        self.assertEqual(device.files["wetland-session.json.recovery-1.json"], USER_RECOVERY)
        self.assertEqual(device.files["wetland-session.json"], USER_BASE)

    def test_cleanup_force_stops_app_before_deleting(self):
        device = FakeDevice({"wetland-session.json": USER_BASE})
        ownership = Ownership()
        preflight_files(device, self.out, fixture(), ownership)
        status = self.cleanup(device, ownership)
        self.assertEqual(device.force_stopped, 1)
        order = [c for c in device.calls if c[0] == "shell" or c[1][:2] == ("rm", "-f")]
        self.assertEqual(order[0][1][:2], ("am", "force-stop"))
        self.assertEqual(status["removed"], ["files/" + FIXTURE_REMOTE_NAME])

    def test_missing_base_and_outranking_slot_rejected(self):
        for files in ({}, {"wetland-session.json": USER_BASE,
                           "wetland-session.json.recovery-128.json": b"{}"}):
            device = FakeDevice(files)
            ownership = Ownership()
            with self.assertRaises(TrialError):
                preflight_files(device, self.out, fixture(), ownership)
            self.assertEqual(ownership.owned, [])

    def test_second_trial_may_rewrite_its_own_fixture(self):
        device = FakeDevice({"wetland-session.json": USER_BASE})
        ownership = Ownership()
        preflight_files(device, self.out, fixture(), ownership)
        device.files[FIXTURE_REMOTE_NAME] = b"stale pose from trial 1"
        preflight_files(device, self.out, fixture(), ownership)
        self.assertEqual(device.files[FIXTURE_REMOTE_NAME], fixture())
        self.assertEqual(ownership.owned, [FIXTURE_REMOTE_NAME])


class CaptureOffFinishTest(unittest.TestCase):
    """An OFF trial must expect no capture and must never touch foreign files."""

    def setUp(self):
        from unittest.mock import patch
        self.tmp = tempfile.TemporaryDirectory()
        self.out = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        sleep = patch("collect_wetland_pair.time.sleep", lambda _s: None)
        sleep.start()
        self.addCleanup(sleep.stop)

    def device(self, extra=None):
        files = {"wetland-session.json": USER_BASE, FIXTURE_REMOTE_NAME: fixture()}
        files.update(extra or {})
        return FakeDevice(files), set(files)

    def finish(self, device, before, ownership, state="off"):
        return finish_and_pull(device, self.out, before, fixture(), ownership, state)

    def test_off_trial_expects_no_capture_and_no_request(self):
        device, before = self.device()
        result = self.finish(device, before, Ownership())
        self.assertEqual(result["capture_state"], "off")
        self.assertEqual(result["captures"], [])
        self.assertEqual(result["capture_count"], 0)
        self.assertFalse(result["profile_request_created"])
        self.assertFalse(result["profile_request_present_after"])
        # Absence of a capture is never reported as a consumed request.
        self.assertIsNone(result["profile_request_consumed"])

    def test_unexpected_capture_rejects_the_trial_and_is_left_on_device(self):
        device, before = self.device()
        device.files["frame-profile-v2-20260912.csv"] = b"header\n"
        ownership = Ownership()
        with self.assertRaises(TrialError):
            self.finish(device, before, ownership)
        self.assertEqual(ownership.owned, [])
        self.assertIn("frame-profile-v2-20260912.csv", device.files)
        self.assertIn("unexpected_captures", json.loads(
            (self.out / "unexpected-captures.json").read_text()))

    def test_foreign_profile_request_is_rejected_never_consumed_or_deleted(self):
        device, before = self.device()
        device.files[PROFILE_REQUEST_NAME] = b"500\n"   # written by someone else
        ownership = Ownership()
        with self.assertRaises(TrialError):
            self.finish(device, before, ownership)
        final_cleanup(device, self.out, ownership, log=lambda _m: None)
        self.assertEqual(device.files[PROFILE_REQUEST_NAME], b"500\n")
        self.assertEqual(device.files["wetland-session.json"], USER_BASE)

    def test_on_trial_still_requires_exactly_one_capture(self):
        device, before = self.device()
        with self.assertRaises(TrialError):
            self.finish(device, before, Ownership(), state="on")

    def test_unknown_capture_state_rejected(self):
        device, before = self.device()
        with self.assertRaises(TrialError):
            self.finish(device, before, Ownership(), state="maybe")


class LoadLineTest(unittest.TestCase):
    def test_parses_last_real_load_line(self):
        text = ("I Matterweave: WETLAND LOADED: 10 cells / 2 placed objects; collision Ok; 1.0s\n"
                "I Matterweave: WETLAND LOADED: 918273 cells / 4211 placed objects; "
                "collision Ok(1); 6.512s\n")
        self.assertEqual(parse_wetland_loaded(text),
                         {"cells": 918273, "instances": 4211})

    def test_absent_line_returns_none(self):
        self.assertIsNone(parse_wetland_loaded("I Matterweave: Wetland graphics: ...\n"))


class ThermalMatchTest(unittest.TestCase):
    def test_accepts_within_one_degree_battery_and_two_skin(self):
        deltas = check_thermal_match(row(300, 32.0), row(310, 33.9))
        self.assertAlmostEqual(deltas["battery_delta_c"], 1.0, places=6)
        self.assertLessEqual(deltas["skin_delta_c"], 2.0)

    def test_rejects_warmer_start_than_reference(self):
        with self.assertRaises(ValueError):
            check_thermal_match(row(300, 32.0), row(312, 32.0))
        with self.assertRaises(ValueError):
            check_thermal_match(row(300, 32.0), row(300, 34.5))


if __name__ == "__main__":
    unittest.main()


class ActualReadinessTimeTest(unittest.TestCase):
    def gate(self, now):
        from unittest.mock import patch
        from collect_wetland_pair import gate_before_launch
        window = []
        for index in range(5):
            observation = row(300, 30.0)
            observation.update(elapsed_s=index * 30.0, host_monotonic_s=880.0 + index * 30.0)
            window.append(observation)
        class Device:
            def shell(self, command, service, **kwargs):
                return row(300, 30.0)["data"][service]
        with tempfile.TemporaryDirectory() as directory, patch(
                "collect_wetland_pair.time.monotonic", return_value=now):
            return gate_before_launch(Device(), Path(directory),
                {"accepted_monotonic_s": 1000.0}, window, None, None)

    def test_new_observation_keeps_actual_elapsed_time(self):
        gate = self.gate(1001.25)
        self.assertEqual(gate["raw"]["elapsed_s"], 121.25)
        self.assertEqual(gate["window"]["sample_count"], 6)
        self.assertEqual(gate["window"]["elapsed_span_s"], 121.25)

    def test_real_gap_cannot_be_replaced_with_invented_thirty_seconds(self):
        with self.assertRaises(TrialError):
            self.gate(1040.0)
