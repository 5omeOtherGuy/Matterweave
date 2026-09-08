"""Focused unittest suite for tools/performance/validate_conditions.py.

ALL fixtures in this file are explicitly SYNTHETIC — small hand-built
`dumpsys battery` / `dumpsys thermalservice` text templates and JSONL row
lists. No real device measurements appear here. Expected summary values are
computed independently by hand in the assertions.

Run: python3 -m unittest tools.performance.test_validate_conditions -v
(or: python3 tools/performance/test_validate_conditions.py)
"""

import copy
import json
import os
import subprocess
import sys
import unittest
from unittest.mock import patch

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import validate_conditions as vc  # noqa: E402

VALIDATOR = os.path.join(HERE, "validate_conditions.py")


# ---------------------------------------------------------------------------
# Synthetic fixture builders
# ---------------------------------------------------------------------------

def synthetic_battery_text(ac="false", usb="false", wireless="false", dock="false",
                           status="3", level="60", temperature="300", extra=""):
    """SYNTHETIC dumpsys battery text (shape mirrors Android output)."""
    lines = [
        "Current Battery Service state:",
        "  AC powered: %s" % ac,
        "  USB powered: %s" % usb,
        "  Wireless powered: %s" % wireless,
        "  Dock powered: %s" % dock,
        "  Max charging current: 0",
        "  Charge counter: 2612000",
        "  status: %s" % status,
        "  health: 2",
        "  present: true",
        "  level: %s" % level,
        "  scale: 100",
        "  voltage: 3997",
        "  temperature: %s" % temperature,
        "  technology: Li-ion",
    ]
    if extra:
        lines.append(extra)
    return "\n".join(lines) + "\n"


def synthetic_thermal_text(override="false", thermal_status="0",
                           current_skin="30.464", include_hal=True,
                           end_boundary=True, malformed_skin=False,
                           duplicate_skin=False, extra=""):
    """SYNTHETIC dumpsys thermalservice text.

    Always contains a Cached temperatures section with a stale skin value
    (45.979) so tests can prove the validator never reads cached skin data.
    """
    lines = [
        "IsStatusOverride: %s" % override,
        "ThermalEventListeners:",
        "\tcallbacks: 4",
        "\tkilled: false",
        "ThermalStatusListeners:",
        "\tcallbacks: 2",
        "Thermal Status: %s" % thermal_status,
        "Cached temperatures:",
        "\tTemperature{mValue=45.979, mType=3, mName=skin, mStatus=0}",
        "\tTemperature{mValue=30.0, mType=2, mName=battery, mStatus=0}",
    ]
    if include_hal:
        lines.append("Current temperatures from HAL:")
        lines.append("\tTemperature{mValue=40.0, mType=8, mName=socd, mStatus=0}")
        lines.append("\tTemperature{mValue=30.0, mType=2, mName=battery, mStatus=0}")
        if malformed_skin:
            lines.append("\tTemperature{mValue=not-a-number, mType=3, mName=skin, mStatus=0}")
        else:
            lines.append("\tTemperature{mValue=%s, mType=3, mName=skin, mStatus=0}" % current_skin)
        if duplicate_skin:
            lines.append("\tTemperature{mValue=99.9, mType=3, mName=skin, mStatus=0}")
        if end_boundary:
            lines.append("Current cooling devices from HAL:")
            lines.append("\tCoolingDevice{mValue=0, mType=3, mName=gpu}")
        lines.append("Temperature static thresholds from HAL:")
        lines.append("\tTemperatureThreshold{mType=3, mName=skin, "
                     "mHotThrottlingThresholds=[NaN, 48.0]}")
    if extra:
        lines.append(extra)
    return "\n".join(lines) + "\n"


def synthetic_row(elapsed_s, battery_text=None, thermal_text=None):
    """SYNTHETIC readiness row matching the runtime collector interface."""
    return {
        "elapsed_s": elapsed_s,
        "data": {
            "battery": battery_text if battery_text is not None
            else synthetic_battery_text(),
            "thermalservice": thermal_text if thermal_text is not None
            else synthetic_thermal_text(),
        },
    }


def synthetic_valid_rows():
    """SYNTHETIC valid 2-minute record: 5 samples, span 120s, gaps 30s.

    Hand-computed expectations:
      battery C: 30.0, 30.0, 29.9, 29.9, 29.9 -> min 29.9 max 30.0 (range 0.1)
      skin C:    30.5, 30.4, 30.3, 30.2, 30.1 -> min 30.1 max 30.5 (range 0.4)
      span = 120.0
    """
    battery = ["300", "300", "299", "299", "299"]
    skins = ["30.5", "30.4", "30.3", "30.2", "30.1"]
    elapsed = [0.0, 30.0, 60.0, 90.0, 120.0]
    return [synthetic_row(e,
                          battery_text=synthetic_battery_text(temperature=bt),
                          thermal_text=synthetic_thermal_text(current_skin=sk))
            for e, bt, sk in zip(elapsed, battery, skins)]


def write_jsonl(path, rows):
    with open(path, "w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row) + "\n")
    return path


def run_cli(path):
    return subprocess.run(
        [sys.executable, VALIDATOR, path],
        capture_output=True, text=True, timeout=120,
    )


# ---------------------------------------------------------------------------
# API: valid summary
# ---------------------------------------------------------------------------

class TestValidSummary(unittest.TestCase):
    def test_valid_fixture_summary_values(self):
        summary = vc.validate_readiness(synthetic_valid_rows())
        self.assertEqual(summary["sample_count"], 5)
        self.assertAlmostEqual(summary["elapsed_span_s"], 120.0, places=6)
        self.assertAlmostEqual(summary["battery_min_c"], 29.9, places=6)
        self.assertAlmostEqual(summary["battery_max_c"], 30.0, places=6)
        self.assertAlmostEqual(summary["skin_min_c"], 30.1, places=6)
        self.assertAlmostEqual(summary["skin_max_c"], 30.5, places=6)
        self.assertEqual(summary["evidence"], "idle_readiness_only")

    def test_summary_keys_exact(self):
        summary = vc.validate_readiness(synthetic_valid_rows())
        self.assertEqual(
            set(summary.keys()),
            {"sample_count", "elapsed_span_s", "battery_min_c", "battery_max_c",
             "skin_min_c", "skin_max_c", "evidence"},
        )

    def test_inputs_unchanged(self):
        rows = synthetic_valid_rows()
        snapshot = copy.deepcopy(rows)
        vc.validate_readiness(rows)
        self.assertEqual(rows, snapshot)


# ---------------------------------------------------------------------------
# API: structure
# ---------------------------------------------------------------------------

class TestStructure(unittest.TestCase):
    def test_empty_rows_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness([])

    def test_rows_not_list_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness({"elapsed_s": 0})

    def test_row_not_dict_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(["not a dict"])

    def test_missing_data_rejected(self):
        rows = synthetic_valid_rows()
        del rows[2]["data"]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_data_not_dict_rejected(self):
        rows = synthetic_valid_rows()
        rows[1]["data"] = "battery text"
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_missing_battery_rejected(self):
        rows = synthetic_valid_rows()
        del rows[0]["data"]["battery"]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_missing_thermalservice_rejected(self):
        rows = synthetic_valid_rows()
        del rows[0]["data"]["thermalservice"]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_data_value_not_str_rejected(self):
        rows = synthetic_valid_rows()
        rows[3]["data"]["battery"] = {"text": 1}
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_extra_data_keys_ignored(self):
        rows = synthetic_valid_rows()
        rows[2]["data"]["extra_stream"] = "ignored"
        vc.validate_readiness(rows)  # must not raise


# ---------------------------------------------------------------------------
# API: timing contract
# ---------------------------------------------------------------------------

class TestTiming(unittest.TestCase):
    def test_four_samples_rejected(self):
        rows = synthetic_valid_rows()[:4]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_span_below_120_rejected(self):
        # 5 samples, gaps 25s, span 100s
        rows = [synthetic_row(e) for e in (0.0, 25.0, 50.0, 75.0, 100.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_gap_above_35_rejected(self):
        rows = [synthetic_row(e) for e in (0.0, 30.0, 60.0, 96.0, 126.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_equal_elapsed_rejected(self):
        rows = [synthetic_row(e) for e in (0.0, 30.0, 30.0, 90.0, 120.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_decreasing_elapsed_rejected(self):
        rows = [synthetic_row(e) for e in (0.0, 30.0, 20.0, 90.0, 120.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_negative_elapsed_rejected(self):
        rows = [synthetic_row(e) for e in (-1.0, 30.0, 60.0, 90.0, 120.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_boolean_elapsed_rejected(self):
        rows = [synthetic_row(e) for e in (True, 30.0, 60.0, 90.0, 120.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_string_elapsed_rejected(self):
        rows = [synthetic_row(e) for e in ("0", 30.0, 60.0, 90.0, 120.0)]
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_nan_elapsed_rejected(self):
        rows = synthetic_valid_rows()
        rows[2]["elapsed_s"] = float("nan")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_infinite_elapsed_rejected(self):
        rows = synthetic_valid_rows()
        rows[2]["elapsed_s"] = float("inf")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)


# ---------------------------------------------------------------------------
# API: battery contract
# ---------------------------------------------------------------------------

class TestBattery(unittest.TestCase):
    def valid_row_with(self, **battery_kwargs):
        row = synthetic_row(0.0, battery_text=synthetic_battery_text(**battery_kwargs))
        return [row] + synthetic_valid_rows()[1:]

    def test_missing_ac_powered_rejected(self):
        rows = synthetic_valid_rows()
        rows[0]["data"]["battery"] = synthetic_battery_text().replace(
            "  AC powered: false\n", "")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_missing_dock_powered_rejected(self):
        rows = synthetic_valid_rows()
        rows[0]["data"]["battery"] = synthetic_battery_text().replace(
            "  Dock powered: false\n", "")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_ac_powered_true_rejected(self):
        rows = self.valid_row_with(ac="true")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_usb_powered_true_rejected(self):
        rows = self.valid_row_with(usb="true")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_wireless_powered_true_rejected(self):
        rows = self.valid_row_with(wireless="true")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_dock_powered_true_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(self.valid_row_with(dock="true"))

    def test_huge_temperature_is_a_validation_error(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(self.valid_row_with(temperature="9" * 400))

    def test_status_not_discharging_rejected(self):
        rows = self.valid_row_with(status="2")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_level_out_of_range_rejected(self):
        rows = self.valid_row_with(level="101")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_level_negative_rejected(self):
        rows = self.valid_row_with(level="-1")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_temperature_above_80c_rejected(self):
        rows = self.valid_row_with(temperature="801")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_temperature_below_minus_20c_rejected(self):
        rows = self.valid_row_with(temperature="-201")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_temperature_non_numeric_rejected(self):
        rows = self.valid_row_with(temperature="hot")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_duplicate_status_field_rejected(self):
        rows = synthetic_valid_rows()
        rows[1]["data"]["battery"] = synthetic_battery_text(
            extra="  status: 3")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_updates_stopped_marker_rejected(self):
        rows = self.valid_row_with(extra="  (UPDATES STOPPED)")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_simulated_marker_rejected(self):
        rows = self.valid_row_with(extra="  test mode: simulated battery")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)


# ---------------------------------------------------------------------------
# API: thermal contract
# ---------------------------------------------------------------------------

def thermal_rows(**thermal_kwargs):
    rows = synthetic_valid_rows()
    rows[2]["data"]["thermalservice"] = synthetic_thermal_text(**thermal_kwargs)
    return rows


class TestThermal(unittest.TestCase):
    def test_duplicate_and_inverted_hal_boundaries_rejected(self):
        start = "Current temperatures from HAL:"
        end = "Current cooling devices from HAL:"
        text = synthetic_thermal_text()
        malformed = [text.replace(start, start + "\n" + start),
                     text.replace(end, end + "\n" + end),
                     text.replace(start, "BOUNDARY").replace(end, start).replace("BOUNDARY", end)]
        for thermal in malformed:
            with self.subTest(thermal=thermal):
                rows = synthetic_valid_rows()
                rows[0]["data"]["thermalservice"] = thermal
                with self.assertRaises(ValueError):
                    vc.validate_readiness(rows)

    def test_is_status_override_true_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(override="true"))

    def test_thermal_status_nonzero_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(thermal_status="1"))

    def test_missing_hal_section_rejected_cached_skin_not_used(self):
        # HAL section absent entirely; only cached skin (45.979) exists.
        rows = thermal_rows(include_hal=False)
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_missing_end_boundary_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(end_boundary=False))

    def test_missing_hal_skin_rejected(self):
        rows = synthetic_valid_rows()
        rows[2]["data"]["thermalservice"] = synthetic_thermal_text().replace(
            "\tTemperature{mValue=30.464, mType=3, mName=skin, mStatus=0}\n", "")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_malformed_skin_line_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(malformed_skin=True))

    def test_duplicate_skin_entry_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(duplicate_skin=True))

    def test_skin_above_100_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(current_skin="100.1"))

    def test_skin_below_minus_20_rejected(self):
        with self.assertRaises(ValueError):
            vc.validate_readiness(thermal_rows(current_skin="-20.5"))

    def test_skin_non_finite_text_rejected(self):
        rows = synthetic_valid_rows()
        rows[2]["data"]["thermalservice"] = synthetic_thermal_text().replace(
            "mValue=30.464", "mValue=nan")
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)


# ---------------------------------------------------------------------------
# API: stability contract
# ---------------------------------------------------------------------------

class TestStability(unittest.TestCase):
    def test_battery_range_above_1c_rejected(self):
        # 29.0 .. 30.2 -> range 1.2 C
        rows = synthetic_valid_rows()
        temps = ["300", "290", "295", "300", "302"]
        for row, t in zip(rows, temps):
            row["data"]["battery"] = synthetic_battery_text(temperature=t)
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_skin_range_above_2c_rejected(self):
        # 30.0 .. 32.5 -> range 2.5 C
        rows = synthetic_valid_rows()
        skins = ["30.0", "30.6", "31.2", "31.8", "32.5"]
        for row, s in zip(rows, skins):
            row["data"]["thermalservice"] = synthetic_thermal_text(current_skin=s)
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_boundary_ranges_accepted(self):
        # battery range exactly 1.0 C, skin range exactly 2.0 C
        rows = synthetic_valid_rows()
        battery = ["290", "290", "290", "295", "300"]  # 29.0..30.0
        skins = ["30.0", "30.5", "31.0", "31.5", "32.0"]  # 30.0..32.0
        for row, b, s in zip(rows, battery, skins):
            row["data"]["battery"] = synthetic_battery_text(temperature=b)
            row["data"]["thermalservice"] = synthetic_thermal_text(current_skin=s)
        summary = vc.validate_readiness(rows)
        self.assertAlmostEqual(summary["battery_max_c"] - summary["battery_min_c"],
                               1.0, places=6)
        self.assertAlmostEqual(summary["skin_max_c"] - summary["skin_min_c"],
                               2.0, places=6)


# ---------------------------------------------------------------------------
# API: size limits
# ---------------------------------------------------------------------------

class TestSizeLimits(unittest.TestCase):
    def test_single_row_over_128kib_rejected(self):
        rows = synthetic_valid_rows()
        rows[2]["data"]["battery"] = "x" * (128 * 1024 + 1)
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)

    def test_total_over_1mib_rejected(self):
        # 12 rows of ~122 KiB each: > 1 MiB total, each line < 128 KiB
        rows = []
        pad = "x" * 120_000
        for i in range(12):
            rows.append(synthetic_row(float(i) * 15.0))
            rows[-1]["data"]["battery"] += pad
        with self.assertRaises(ValueError):
            vc.validate_readiness(rows)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

class TestCli(unittest.TestCase):
    def test_read_is_bounded_even_when_file_size_changes(self):
        p = write_jsonl(self.path("growing.jsonl"), synthetic_valid_rows())

        def bounded_read(size=-1):
            self.assertGreaterEqual(size, 0, "unbounded read ignores input cap")
            self.assertLessEqual(size, vc.MAX_TOTAL_BYTES + 1)
            return b"x" * (vc.MAX_TOTAL_BYTES + 1)

        with patch("builtins.open") as opened:
            opened.return_value.__enter__.return_value.read.side_effect = bounded_read
            with self.assertRaisesRegex(ValueError, "exceeds"):
                vc.load_jsonl(p)

    def test_huge_elapsed_is_rejected_without_traceback(self):
        rows = synthetic_valid_rows()
        rows[0]["elapsed_s"] = 10 ** 400
        p = write_jsonl(self.path("huge-elapsed.jsonl"), rows)
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def setUp(self):
        import tempfile
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def path(self, name):
        return os.path.join(self.tmp.name, name)

    def test_valid_file_exit_zero_summary_json(self):
        p = write_jsonl(self.path("ok.jsonl"), synthetic_valid_rows())
        with open(p, "rb") as fh:
            before = fh.read()
        proc = run_cli(p)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        summary = json.loads(proc.stdout)
        self.assertEqual(summary["sample_count"], 5)
        self.assertAlmostEqual(summary["elapsed_span_s"], 120.0, places=6)
        self.assertAlmostEqual(summary["battery_min_c"], 29.9, places=6)
        self.assertAlmostEqual(summary["battery_max_c"], 30.0, places=6)
        self.assertAlmostEqual(summary["skin_min_c"], 30.1, places=6)
        self.assertAlmostEqual(summary["skin_max_c"], 30.5, places=6)
        self.assertEqual(summary["evidence"], "idle_readiness_only")
        # input file untouched (read-only validator)
        with open(p, "rb") as fh:
            self.assertEqual(fh.read(), before)

    def test_bad_json_rejected_no_traceback(self):
        p = self.path("bad.jsonl")
        with open(p, "w") as fh:
            fh.write("{not json}\n")
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)
        self.assertNotEqual(proc.stderr.strip(), "")

    def test_empty_file_rejected(self):
        p = self.path("empty.jsonl")
        open(p, "w").close()
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_nan_literal_rejected(self):
        p = write_jsonl(self.path("nan.jsonl"), synthetic_valid_rows())
        with open(p, "w") as fh:
            fh.write('{"elapsed_s": NaN, "data": {}}\n')
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_duplicate_json_keys_rejected(self):
        p = self.path("dupkeys.jsonl")
        rows = synthetic_valid_rows()
        # duplicate elapsed_s key at top level (same value, still rejected)
        line = json.dumps(rows[0])
        doctored = '{"elapsed_s": 0, ' + line[1:] + "\n"
        with open(p, "w") as fh:
            fh.write(doctored)
            for row in rows[1:]:
                fh.write(json.dumps(row) + "\n")
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)
        self.assertIn("duplicate JSON key", proc.stderr)

    def test_overlarge_line_rejected(self):
        p = self.path("bigline.jsonl")
        rows = synthetic_valid_rows()
        rows[1]["data"]["battery"] += "x" * (128 * 1024 + 10)
        write_jsonl(p, rows)
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_overlarge_total_rejected(self):
        p = self.path("bigtotal.jsonl")
        rows = []
        pad = "x" * 120_000
        for i in range(12):
            rows.append(synthetic_row(float(i) * 15.0))
            rows[-1]["data"]["battery"] += pad
        write_jsonl(p, rows)
        proc = run_cli(p)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_missing_file_rejected(self):
        proc = run_cli(self.path("does-not-exist.jsonl"))
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_missing_argument_rejected(self):
        proc = subprocess.run([sys.executable, VALIDATOR],
                              capture_output=True, text=True, timeout=60)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)


if __name__ == "__main__":
    unittest.main()
