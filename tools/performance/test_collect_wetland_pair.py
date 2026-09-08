#!/usr/bin/env python3
"""Phone-free tests for the paired wetland collector's decision helpers.

These cover the gates that decide whether a trial may run at all: pairing
order, fixture validity, base-save invalidity (so the recovery slot is really
selected), the real scene load line, and the thermal matching bound.
"""
import copy
import json
import unittest

from collect_wetland_pair import (
    base_save_is_invalid_for,
    check_fixture,
    check_thermal_match,
    parse_wetland_loaded,
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
        "edits": [{"instance": "i_shell_5_8_3", "cell": [24, 3, 13], "material": 0}],
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

    def test_absent_or_corrupt_save_is_invalid(self):
        self.assertTrue(base_save_is_invalid_for(None, 2))
        self.assertTrue(base_save_is_invalid_for(b"{oops", 2))
        self.assertTrue(base_save_is_invalid_for(b"\xff\xfe", 2))

    def test_matching_generator_blocks_the_run(self):
        self.assertFalse(base_save_is_invalid_for(fixture(), 2))


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
