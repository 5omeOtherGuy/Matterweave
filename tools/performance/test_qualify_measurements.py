"""Adversarial checks for the measurement qualification analyzer.

Two kinds of fixture are used and never confused:

- REAL: docs/evidence/2026-09-08-p01-overhead.json, the executed 2026-09-08
  OnePlus 13 on/off pairs. It is read read-only to prove that this tool's
  arithmetic reproduces the numbers already recorded there, and that the same
  file does NOT qualify as a new candidate because it carries none of the
  per-run qualification fields.
- SYNTHETIC: `document()` below invents metadata and timings purely to exercise
  validation. Its numbers are arithmetic fixtures, not measurements, and no
  device claim follows from them.
"""
import copy
import io
import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import qualify_measurements as qm
from qualify_measurements import (QualificationError, capture_overhead, main,
                                  qualify, same_build_noise)

REPO_ROOT = Path(__file__).resolve().parents[2]
HISTORICAL = REPO_ROOT / "docs" / "evidence" / "2026-09-08-p01-overhead.json"

BUILD = {"source_commit": "704bb4a7c274d4cd33c8fe859ef15b18e90bbc06",
         "apk_sha256": "0c" * 32, "composition_hash": "cafe" * 8,
         "generator": 3}
CONDITIONS = {"quality": "shadows-2048", "scene": "wetland",
              "seed": 20260907, "replay": "stationary-120s",
              "output_resolution": "1440x3168", "refresh_hz": 60.000004}
TIMING_SOURCE = {"kind": "surfaceflinger_presentation_intervals",
                 "tool": "collect_wetland_pair.py + analyze_wetland_pairs.py",
                 "independent_of_capture": True}


def observed(battery=32.0, skin=31.0, mode="1440x3168@60.000004"):
    return {"power_sources_live": False, "thermal_status_samples": [0] * 8,
            "battery_start_c": battery, "skin_start_c": skin,
            "active_display_mode": mode}


def run(name, profiling, mean_ms, digest, cpu=73.9):
    return {"name": name, "profiling": profiling, "interval_count": 5400,
            "artifact_sha256": digest, "build": copy.deepcopy(BUILD),
            "conditions": copy.deepcopy(CONDITIONS), "observed": observed(),
            "samples": {"missing_observation_count": 0,
                        "unsupported_gap_count": 0},
            "mean_ms": mean_ms, "process_cpu_core_percent": cpu}


def document():
    """SYNTHETIC. Exact decimal means keep the expected arithmetic obvious."""
    return {"schema": qm.SCHEMA, "build": copy.deepcopy(BUILD),
            "conditions": copy.deepcopy(CONDITIONS),
            "timing_source": copy.deepcopy(TIMING_SOURCE),
            "metrics": ["mean_ms"],
            "runs": [run("on-01", "on", 21.0, "a" * 64),
                     run("off-01", "off", 20.0, "b" * 64),
                     run("on-02", "on", 22.0, "c" * 64),
                     run("off-02", "off", 20.0, "d" * 64),
                     run("on-03", "on", 21.0, "e" * 64),
                     run("off-03", "off", 20.0, "f" * 64)]}


def qualify_document(doc):
    return qualify(doc, "input-digest")


class ArithmeticTests(unittest.TestCase):
    """Known fixtures: every number below is checked by hand."""

    def test_noise_and_overhead_are_reported_separately(self):
        report = qualify_document(document())
        on = report["same_build_noise"]["on"]["metrics"]["mean_ms"]
        off = report["same_build_noise"]["off"]["metrics"]["mean_ms"]
        # ON means 21, 22, 21 -> median 21, range 1 -> 100*1/21.
        self.assertEqual(on["repeats"], 3)
        self.assertEqual(on["range"], 1.0)
        self.assertAlmostEqual(on["range_over_median_percent"], 100.0 / 21.0)
        # OFF means are all 20 -> zero spread, reported as zero, not omitted.
        self.assertEqual(off["range"], 0.0)
        self.assertEqual(off["range_over_median_percent"], 0.0)
        summary = report["capture_overhead"]["summary"]["mean_ms"]
        self.assertEqual(summary["pair_count"], 3)
        self.assertAlmostEqual(summary["mean_on_vs_off_absolute"], 4.0 / 3.0)
        self.assertAlmostEqual(summary["max_on_vs_off_percent"], 10.0)
        self.assertAlmostEqual(summary["min_on_vs_off_percent"], 5.0)

    def test_pairs_follow_run_order_and_name_both_sides(self):
        report = qualify_document(document())
        pairs = report["capture_overhead"]["pairs"]
        self.assertEqual([(p["on"], p["off"]) for p in pairs],
                         [("on-01", "off-01"), ("on-02", "off-02"),
                          ("on-03", "off-03")])

    def test_overhead_above_and_below_the_noise_range_is_labelled(self):
        report = qualify_document(document())
        verdict = report["resolution"]["mean_ms"]
        self.assertEqual(verdict["same_build_noise_range"], 1.0)
        self.assertEqual(verdict["verdict"], "exceeds same-build noise range")
        quiet = document()
        for entry, mean in zip(quiet["runs"], [20.1, 20.0, 20.4, 20.0, 20.1, 20.0]):
            entry["mean_ms"] = mean
        # ON range 0.3 exceeds the mean ON-OFF difference 0.2.
        self.assertEqual(qualify_document(quiet)["resolution"]["mean_ms"]
                         ["verdict"], "below same-build noise range")

    def test_off_side_is_never_swapped_when_off_runs_first(self):
        doc = document()
        doc["runs"] = doc["runs"][1:] + [run("on-04", "on", 21.0, "1" * 64)]
        report = qualify_document(doc)
        pair = report["capture_overhead"]["pairs"][0]
        self.assertEqual((pair["on"], pair["off"]), ("on-02", "off-01"))
        self.assertAlmostEqual(pair["on_vs_off_absolute"]["mean_ms"], 2.0)


class HistoricalEvidenceTests(unittest.TestCase):
    """REAL executed data: reuse the 2026-09-08 contract, do not redo the run."""

    @classmethod
    def setUpClass(cls):
        cls.evidence = json.loads(HISTORICAL.read_text())

    def _runs(self):
        return [{"name": entry["name"], "profiling": entry["profiling"],
                 "metrics": {key: entry[key] for key in
                             ("mean_ms", "p95_ms", "p99_ms",
                              "process_cpu_core_percent")}}
                for entry in self.evidence["runs"]]

    def test_recorded_pair_percentages_are_reproduced(self):
        runs = self._runs()
        metrics = sorted(runs[0]["metrics"])
        pairs = list(zip(runs[0::2], runs[1::2]))
        computed = capture_overhead(pairs, metrics)["pairs"]
        self.assertEqual(len(computed), len(self.evidence["pairs"]))
        for recorded, actual in zip(self.evidence["pairs"], computed):
            self.assertEqual((recorded["on"], recorded["off"]),
                             (actual["on"], actual["off"]))
            for metric, value in recorded["on_vs_off_percent"].items():
                self.assertAlmostEqual(actual["on_vs_off_percent"][metric],
                                       value, places=12)

    def test_recorded_same_build_ranges_are_reproduced(self):
        runs = self._runs()
        metrics = sorted(runs[0]["metrics"])
        noise = same_build_noise(runs, metrics)
        for state, recorded in self.evidence["range_over_median_percent"].items():
            for metric, value in recorded.items():
                self.assertAlmostEqual(
                    noise[state]["metrics"][metric]["range_over_median_percent"],
                    value, places=12)

    def test_historical_file_does_not_qualify_as_a_new_candidate(self):
        with self.assertRaises(QualificationError) as caught:
            qualify_document(copy.deepcopy(self.evidence))
        self.assertIn("missing", str(caught.exception))


class RejectionTests(unittest.TestCase):
    def assert_rejects(self, doc, fragment):
        with self.assertRaises(QualificationError) as caught:
            qualify_document(doc)
        self.assertIn(fragment, str(caught.exception))

    def test_capture_derived_timing_source_is_rejected(self):
        doc = document()
        doc["timing_source"]["kind"] = "frame_profile_csv"
        self.assert_rejects(doc, "produced only while capture is on")

    def test_timing_source_must_be_declared_independent(self):
        doc = document()
        doc["timing_source"]["independent_of_capture"] = False
        self.assert_rejects(doc, "independent_of_capture must be true")

    def test_missing_metric_is_never_zero_filled(self):
        doc = document()
        del doc["runs"][2]["mean_ms"]
        self.assert_rejects(doc, "never filled with zero")

    def test_null_metric_is_rejected(self):
        doc = document()
        doc["runs"][2]["mean_ms"] = None
        self.assert_rejects(doc, "never substituted with zero")

    def test_zero_metric_is_rejected(self):
        doc = document()
        doc["runs"][2]["mean_ms"] = 0
        self.assert_rejects(doc, "never reported as zero")

    def test_mismatched_per_run_build_is_detected(self):
        doc = document()
        doc["runs"][3]["build"]["apk_sha256"] = "ff" * 32
        self.assert_rejects(doc, "build.apk_sha256")

    def test_mismatched_per_run_condition_is_detected(self):
        doc = document()
        doc["runs"][1]["conditions"]["seed"] = 1
        self.assert_rejects(doc, "conditions.seed")

    def test_per_run_blocks_are_required_not_inherited(self):
        doc = document()
        del doc["runs"][4]["conditions"]
        self.assert_rejects(doc, "conditions")

    def test_mismatched_active_display_mode_is_detected(self):
        doc = document()
        doc["runs"][2]["observed"]["active_display_mode"] = "1080x2400@120.0"
        self.assert_rejects(doc, "active display mode differs")

    def test_live_power_source_is_not_qualified(self):
        doc = document()
        doc["runs"][0]["observed"]["power_sources_live"] = True
        self.assert_rejects(doc, "power_sources_live must be false")

    def test_throttled_thermal_status_is_not_qualified(self):
        doc = document()
        doc["runs"][5]["observed"]["thermal_status_samples"] = [0, 0, 2]
        self.assert_rejects(doc, "throttled run is NOT QUALIFIED")

    def test_unobserved_thermal_state_is_not_qualified(self):
        doc = document()
        doc["runs"][5]["observed"]["thermal_status_samples"] = []
        self.assert_rejects(doc, "unobserved thermal state")

    def test_start_temperature_spread_beyond_tolerance_is_rejected(self):
        doc = document()
        doc["runs"][4]["observed"]["battery_start_c"] = 34.0
        self.assert_rejects(doc, "battery_start_c spread")

    def test_missing_observations_are_not_qualified(self):
        doc = document()
        doc["runs"][1]["samples"]["missing_observation_count"] = 3
        self.assert_rejects(doc, "NOT QUALIFIED")

    def test_unsupported_gaps_are_not_qualified(self):
        doc = document()
        doc["runs"][1]["samples"]["unsupported_gap_count"] = 1
        self.assert_rejects(doc, "unsupported_gap_count")

    def test_sample_accounting_must_be_present(self):
        doc = document()
        del doc["runs"][1]["samples"]
        self.assert_rejects(doc, "samples")

    def test_non_alternating_sequence_is_rejected(self):
        doc = document()
        doc["runs"][1], doc["runs"][2] = doc["runs"][2], doc["runs"][1]
        self.assert_rejects(doc, "must alternate")

    def test_too_few_pairs_is_rejected(self):
        doc = document()
        doc["runs"] = doc["runs"][:4]
        self.assert_rejects(doc, "below the required 3")

    def test_odd_run_count_is_rejected(self):
        doc = document()
        doc["runs"] = doc["runs"][:5]
        self.assert_rejects(doc, "complete pairs")

    def test_empty_run_list_reports_not_run(self):
        doc = document()
        doc["runs"] = []
        self.assert_rejects(doc, "NOT RUN")

    def test_short_run_below_interval_floor_is_rejected(self):
        doc = document()
        doc["runs"][0]["interval_count"] = 12
        self.assert_rejects(doc, "below the required floor")

    def test_duplicate_run_name_is_rejected(self):
        doc = document()
        doc["runs"][4]["name"] = "on-01"
        self.assert_rejects(doc, "names must be unique")

    def test_shared_artifact_digest_is_rejected(self):
        doc = document()
        doc["runs"][4]["artifact_sha256"] = doc["runs"][0]["artifact_sha256"]
        self.assert_rejects(doc, "same artifact_sha256")

    def test_artifact_digest_must_look_like_a_digest(self):
        doc = document()
        doc["runs"][4]["artifact_sha256"] = "not-a-digest"
        self.assert_rejects(doc, "64-hex digest")

    def test_undeclared_capture_state_is_rejected(self):
        doc = document()
        doc["runs"][0]["profiling"] = "unknown"
        self.assert_rejects(doc, "must be declared as 'on' or 'off'")

    def test_unknown_top_level_key_is_rejected(self):
        doc = document()
        doc["fps"] = 60
        self.assert_rejects(doc, "unknown key(s) fps")

    def test_wrong_schema_is_rejected(self):
        doc = document()
        doc["schema"] = "matterweave-qualification-v2"
        self.assert_rejects(doc, "unsupported schema")

    def test_missing_condition_key_is_rejected(self):
        doc = document()
        del doc["conditions"]["refresh_hz"]
        for entry in doc["runs"]:
            entry["conditions"].pop("refresh_hz")
        self.assert_rejects(doc, "refresh_hz")

    def test_metric_name_colliding_with_a_reserved_run_key_is_rejected(self):
        doc = document()
        doc["metrics"] = ["interval_count"]
        self.assert_rejects(doc, "reserved run key")


class MutationTests(unittest.TestCase):
    """Meaningful mutations of the analysis must break a test above."""

    def test_swapping_the_paired_sign_changes_the_reported_overhead(self):
        report = qualify_document(document())
        forward = report["capture_overhead"]["summary"]["mean_ms"]
        swapped = document()
        for entry in swapped["runs"]:
            entry["profiling"] = "off" if entry["profiling"] == "on" else "on"
        reversed_report = qualify_document(swapped)
        self.assertLess(reversed_report["capture_overhead"]["summary"]
                        ["mean_ms"]["mean_on_vs_off_absolute"], 0)
        self.assertGreater(forward["mean_on_vs_off_absolute"], 0)

    def test_noise_uses_only_same_state_runs(self):
        doc = document()
        doc["runs"][0]["mean_ms"] = 30.0  # an ON outlier
        report = qualify_document(doc)
        self.assertEqual(report["same_build_noise"]["off"]["metrics"]
                         ["mean_ms"]["range"], 0.0)
        self.assertEqual(report["same_build_noise"]["on"]["metrics"]
                         ["mean_ms"]["max"], 30.0)


class CliTests(unittest.TestCase):
    def run_cli(self, doc, *extra):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text(json.dumps(doc))
            out = Path(directory) / "report.json"
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = main(["--input", str(path), "--out", str(out), *extra])
            report = json.loads(out.read_text()) if out.exists() else None
        return code, stdout.getvalue(), stderr.getvalue(), report

    def test_qualified_document_writes_a_report_and_a_markdown_summary(self):
        code, stdout, _, report = self.run_cli(document())
        self.assertEqual(code, 0)
        self.assertTrue(report["qualified"])
        self.assertEqual(report["pair_count"], 3)
        self.assertIn("Same-build noise", stdout)
        self.assertIn("Capture on/off overhead", stdout)
        self.assertIn("Limitations and non-claims", stdout)

    def test_report_carries_declared_limitations_and_never_drops_the_fixed_ones(self):
        doc = document()
        doc["limitations"] = ["Ambient temperature unrecorded."]
        _, _, _, report = self.run_cli(doc)
        self.assertIn("Ambient temperature unrecorded.", report["limitations"])
        for item in qm.TOOL_LIMITATIONS:
            self.assertIn(item, report["limitations"])

    def test_rejection_exits_nonzero_without_a_report_or_a_traceback(self):
        doc = document()
        doc["runs"][0]["observed"]["power_sources_live"] = True
        code, stdout, stderr, report = self.run_cli(doc)
        self.assertEqual(code, 2)
        self.assertIsNone(report)
        self.assertEqual(stdout, "")
        self.assertTrue(stderr.startswith("NOT QUALIFIED:"))
        self.assertNotIn("Traceback", stderr)

    def test_unreadable_input_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as directory:
            stderr = io.StringIO()
            with redirect_stderr(stderr):
                code = main(["--input", str(Path(directory) / "absent.json")])
        self.assertEqual(code, 2)
        self.assertIn("input unreadable", stderr.getvalue())

    def test_duplicate_json_keys_are_rejected_by_the_loader(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text('{"schema": "a", "schema": "b"}')
            stderr = io.StringIO()
            with redirect_stderr(stderr):
                code = main(["--input", str(path)])
        self.assertEqual(code, 2)
        self.assertIn("duplicate JSON key", stderr.getvalue())

    def test_nan_is_rejected_by_the_loader(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            doc = document()
            path.write_text(json.dumps(doc).replace("21.0", "NaN", 1))
            stderr = io.StringIO()
            with redirect_stderr(stderr):
                code = main(["--input", str(path)])
        self.assertEqual(code, 2)
        self.assertIn("NaN", stderr.getvalue())

    def test_existing_output_file_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "input.json"
            path.write_text(json.dumps(document()))
            out = Path(directory) / "report.json"
            out.write_text("keep me")
            with redirect_stdout(io.StringIO()):
                with self.assertRaises(FileExistsError):
                    main(["--input", str(path), "--out", str(out)])
            self.assertEqual(out.read_text(), "keep me")


if __name__ == "__main__":
    unittest.main()
