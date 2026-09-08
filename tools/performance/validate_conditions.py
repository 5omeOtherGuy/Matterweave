#!/usr/bin/env python3
"""Fail-closed read-only validator for TWO-MINUTE IDLE READINESS records.

This is NOT an app performance benchmark and certifies nothing about energy,
FPS or device speed. It checks that a readiness record (JSONL rows emitted by
the runtime collector: {"elapsed_s": <finite nonneg number>,
"data": {"battery": <raw dumpsys battery text>,
"thermalservice": <raw dumpsys thermalservice text>}}) shows a device in a
plausible idle, discharging, unthrottled state with thermally stable
battery/skin readings over at least 2 minutes.

Contract enforced (raise ValueError on any violation):
  - >= 5 samples; elapsed_s strictly increasing, finite, >= 0;
    total span >= 120 s; no consecutive gap > 35 s.
  - Per sample, battery dump: AC/USB/Wireless/Dock powered all false
    (all four fields required), status == 3 (discharging), level 0..100,
    finite plausible temperature (-20..80 C, parsed tenths C).
    If the dump indicates updates stopped or simulated/test mode -> reject.
  - Per sample, thermalservice dump: IsStatusOverride false,
    Thermal Status 0. The skin temperature is taken ONLY from the
    "Current temperatures from HAL:" section (terminated by
    "Current cooling devices from HAL:", both required). Cached skin values
    are never used. Exactly one finite skin entry required (-20..100 C).
  - Missing or malformed required data is rejected as unavailable; it is
    never treated as 0 C.
  - Stability across samples: battery range <= 1 C, skin range <= 2 C.

Limits: total input <= 1 MiB, each JSONL line <= 128 KiB.

Output summary (on success):
  {"sample_count", "elapsed_span_s", "battery_min_c", "battery_max_c",
   "skin_min_c", "skin_max_c", "evidence": "idle_readiness_only"}

The summary is evidence of the *reported data* only. It is not a hardware
calibration, a cryptographic authentication of the device, or a proof of
equal electrical power.

Python stdlib only (json, argparse, math, re). Read-only: never writes the
input, never spawns processes.
"""

import argparse
import json
import math
import re
import sys

MAX_TOTAL_BYTES = 1024 * 1024      # 1 MiB total input bound
MAX_LINE_BYTES = 128 * 1024        # 128 KiB per JSONL line bound

MIN_SAMPLES = 5
MIN_SPAN_S = 120.0
MAX_GAP_S = 35.0
BATTERY_TEMP_MIN_C = -20.0
BATTERY_TEMP_MAX_C = 80.0
SKIN_TEMP_MIN_C = -20.0
SKIN_TEMP_MAX_C = 100.0
MAX_BATTERY_RANGE_C = 1.0
MAX_SKIN_RANGE_C = 2.0

HAL_CURRENT_START = "Current temperatures from HAL:"
HAL_CURRENT_END = "Current cooling devices from HAL:"

POWERED_KEYS = ("AC powered", "USB powered", "Wireless powered", "Dock powered")

# Battery dump markers meaning the shown state is not live hardware state.
_BATTERY_MARKERS = re.compile(r"updates\s+stopped|test\s*mode|simulat", re.IGNORECASE)

_INT_RE = re.compile(r"^-?\d+$")
_TEMPERATURE_LINE_RE = re.compile(
    r"^Temperature\{mValue=(-?\d+(?:\.\d+)?), mType=(-?\d+), "
    r"mName=([^,]+), mStatus=(-?\d+)\}$")


def _reject_json_constant(name):
    raise ValueError("invalid JSON constant %r (NaN/Infinity not accepted)" % name)


def _reject_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key %r" % key)
        result[key] = value
    return result


def _finite_number(value, what):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("%s must be a number, got %r" % (what, value))
    try:
        value = float(value)
    except OverflowError:
        raise ValueError("%s is outside the finite numeric range" % what) from None
    if not math.isfinite(value):
        raise ValueError("%s must be finite, got %r" % (what, value))
    return value


def _parse_scalar_field(text, key):
    """Return the single required "key: value" occurrence in a dump text.

    Raises ValueError if the key is missing or duplicated.
    """
    needle = key + ":"
    found = None
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line.startswith(needle):
            continue
        if found is not None:
            raise ValueError("duplicate required field %r" % key)
        value = line[len(needle):].strip()
        if value == "":
            raise ValueError("empty value for required field %r" % key)
        found = value
    if found is None:
        raise ValueError("missing required field %r" % key)
    return found


def _parse_int_field(text, key):
    raw = _parse_scalar_field(text, key)
    if not _INT_RE.match(raw):
        raise ValueError("field %r is not an integer: %r" % (key, raw))
    return int(raw)


def parse_battery_state(text):
    """Parse one raw dumpsys battery text -> dict with temp_c, level.

    Raises ValueError for any contract violation.
    """
    if not isinstance(text, str) or not text.strip():
        raise ValueError("battery dump missing or empty")
    if _BATTERY_MARKERS.search(text):
        raise ValueError(
            "battery dump indicates updates stopped or simulated/test mode; "
            "reported charging state is not trustworthy")

    for key in POWERED_KEYS:
        raw = _parse_scalar_field(text, key)
        if raw != "false":
            raise ValueError("field %r must be false for idle readiness, "
                             "got %r" % (key, raw))

    status = _parse_int_field(text, "status")
    if status != 3:
        raise ValueError("battery status must be 3 (discharging), got %d" % status)

    level = _parse_int_field(text, "level")
    if not 0 <= level <= 100:
        raise ValueError("battery level out of range 0..100: %d" % level)

    tenths = _parse_int_field(text, "temperature")
    if not (BATTERY_TEMP_MIN_C * 10 <= tenths <= BATTERY_TEMP_MAX_C * 10):
        raise ValueError("battery temperature outside plausible -20..80 C range")

    return {"temp_c": tenths / 10.0, "level": level}


def parse_thermal_state(text):
    """Parse one raw dumpsys thermalservice text -> dict with skin_c.

    Skin is read ONLY from the "Current temperatures from HAL:" section
    (terminated by the required "Current cooling devices from HAL:" line);
    cached values are never used. Raises ValueError for any violation.
    """
    if not isinstance(text, str) or not text.strip():
        raise ValueError("thermalservice dump missing or empty")

    override = _parse_scalar_field(text, "IsStatusOverride")
    if override != "false":
        raise ValueError("IsStatusOverride must be false, got %r" % override)

    thermal_status = _parse_scalar_field(text, "Thermal Status")
    if thermal_status != "0":
        raise ValueError("Thermal Status must be 0 (no throttling), got %r"
                         % thermal_status)

    lines = text.splitlines()
    starts = [i for i, ln in enumerate(lines) if ln.strip() == HAL_CURRENT_START]
    ends = [i for i, ln in enumerate(lines) if ln.strip() == HAL_CURRENT_END]
    if len(starts) != 1:
        raise ValueError("expected exactly one %r section, found %d"
                         % (HAL_CURRENT_START, len(starts)))
    if len(ends) != 1:
        raise ValueError("expected exactly one %r boundary, found %d"
                         % (HAL_CURRENT_END, len(ends)))
    start, end = starts[0], ends[0]
    if end <= start:
        raise ValueError("%r boundary must come after %r"
                         % (HAL_CURRENT_END, HAL_CURRENT_START))

    skin_values = []
    for line in lines[start + 1:end]:
        stripped = line.strip()
        if "Temperature{" not in stripped:
            continue
        match = _TEMPERATURE_LINE_RE.match(stripped)
        if not match:
            raise ValueError("malformed Temperature entry in HAL current "
                             "section: %r" % stripped)
        value = float(match.group(1))
        name = match.group(3)
        if name == "skin":
            skin_values.append(value)

    if len(skin_values) == 0:
        raise ValueError("missing HAL current skin temperature (unavailable; "
                         "cached values are never substituted)")
    if len(skin_values) > 1:
        raise ValueError("duplicate HAL current skin temperature entries: %r"
                         % skin_values)

    skin_c = skin_values[0]
    if not math.isfinite(skin_c):
        raise ValueError("HAL skin temperature not finite")
    if not (SKIN_TEMP_MIN_C <= skin_c <= SKIN_TEMP_MAX_C):
        raise ValueError("skin temperature implausible: %.3f C" % skin_c)

    return {"skin_c": skin_c}


def _check_row_shape(index, row):
    if not isinstance(row, dict):
        raise ValueError("row %d: expected JSON object, got %s"
                         % (index, type(row).__name__))
    if "elapsed_s" not in row:
        raise ValueError("row %d: missing elapsed_s" % index)
    if "data" not in row:
        raise ValueError("row %d: missing data" % index)
    data = row["data"]
    if not isinstance(data, dict):
        raise ValueError("row %d: data must be an object" % index)
    for key in ("battery", "thermalservice"):
        if key not in data:
            raise ValueError("row %d: data.%s missing" % (index, key))
        if not isinstance(data[key], str):
            raise ValueError("row %d: data.%s must be raw text" % (index, key))


def validate_readiness(rows):
    """Validate a two-minute idle readiness record.

    Returns a summary dict on success; raises ValueError on any violation
    (fail-closed). Read-only: the input structure is never modified.
    """
    if not isinstance(rows, list):
        raise ValueError("rows must be a list of JSON objects")
    if len(rows) == 0:
        raise ValueError("no readiness samples provided")

    total = 0
    for row in rows:
        try:
            encoded = json.dumps(row)
        except (TypeError, ValueError) as exc:
            raise ValueError("row is not JSON-serializable: %s" % exc)
        total += len(encoded.encode("utf-8"))
        if len(encoded.encode("utf-8")) > MAX_LINE_BYTES:
            raise ValueError("row exceeds %d byte line limit"
                             % MAX_LINE_BYTES)
    if total > MAX_TOTAL_BYTES:
        raise ValueError("total input %d bytes exceeds %d byte limit"
                         % (total, MAX_TOTAL_BYTES))

    elapsed = []
    battery_states = []
    skin_states = []
    for index, row in enumerate(rows):
        _check_row_shape(index, row)
        value = _finite_number(row["elapsed_s"], "row %d elapsed_s" % index)
        if value < 0.0:
            raise ValueError("row %d elapsed_s must be >= 0, got %r"
                             % (index, value))
        if elapsed and value <= elapsed[-1]:
            raise ValueError("elapsed_s must strictly increase at row %d "
                             "(%r after %r)" % (index, value, elapsed[-1]))
        elapsed.append(value)
        battery_states.append(parse_battery_state(row["data"]["battery"]))
        skin_states.append(parse_thermal_state(row["data"]["thermalservice"]))

    if len(rows) < MIN_SAMPLES:
        raise ValueError("need at least %d samples, got %d"
                         % (MIN_SAMPLES, len(rows)))

    span = elapsed[-1] - elapsed[0]
    if span < MIN_SPAN_S:
        raise ValueError("total span %.1f s is below required %d s"
                         % (span, MIN_SPAN_S))
    for index in range(1, len(elapsed)):
        gap = elapsed[index] - elapsed[index - 1]
        if gap > MAX_GAP_S:
            raise ValueError("gap of %.1f s between rows %d and %d exceeds "
                             "%d s" % (gap, index - 1, index, MAX_GAP_S))

    battery_temps = [state["temp_c"] for state in battery_states]
    skin_temps = [state["skin_c"] for state in skin_states]
    battery_range = max(battery_temps) - min(battery_temps)
    skin_range = max(skin_temps) - min(skin_temps)
    if battery_range > MAX_BATTERY_RANGE_C:
        raise ValueError("battery temperature range %.3f C exceeds %.1f C"
                         % (battery_range, MAX_BATTERY_RANGE_C))
    if skin_range > MAX_SKIN_RANGE_C:
        raise ValueError("skin temperature range %.3f C exceeds %.1f C"
                         % (skin_range, MAX_SKIN_RANGE_C))

    return {
        "sample_count": len(rows),
        "elapsed_span_s": span,
        "battery_min_c": min(battery_temps),
        "battery_max_c": max(battery_temps),
        "skin_min_c": min(skin_temps),
        "skin_max_c": max(skin_temps),
        "evidence": "idle_readiness_only",
    }


def load_jsonl(path):
    """Read a JSONL readiness file read-only, enforcing size bounds.

    Raises ValueError/OSError on invalid input. Never writes the input.
    """
    with open(path, "rb") as fh:  # read-only, bounded even if the file grows
        raw = fh.read(MAX_TOTAL_BYTES + 1)
    if len(raw) > MAX_TOTAL_BYTES:
        raise ValueError("input exceeds %d byte limit" % MAX_TOTAL_BYTES)
    if not raw.strip():
        raise ValueError("input file is empty")
    if raw.endswith(b"\n"):
        raw = raw[:-1]
    lines = raw.split(b"\n")
    rows = []
    for number, raw_line in enumerate(lines, 1):
        if len(raw_line) > MAX_LINE_BYTES:
            raise ValueError("line %d exceeds %d byte limit"
                             % (number, MAX_LINE_BYTES))
        try:
            text = raw_line.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise ValueError("line %d is not valid UTF-8: %s" % (number, exc))
        if not text.strip():
            raise ValueError("line %d is empty" % number)
        try:
            row = json.loads(text,
                             parse_constant=_reject_json_constant,
                             object_pairs_hook=_reject_duplicate_keys)
        except ValueError as exc:
            raise ValueError("line %d is not valid readiness JSON: %s"
                             % (number, exc))
        if not isinstance(row, dict):
            raise ValueError("line %d: expected a JSON object" % number)
        rows.append(row)
    return rows


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Fail-closed validator for two-minute idle readiness "
                    "records (data-condition check, not a performance "
                    "benchmark). Prints a summary JSON on success.")
    parser.add_argument("health_jsonl",
                        help="JSONL readiness record (read-only)")
    args = parser.parse_args(argv)
    try:
        rows = load_jsonl(args.health_jsonl)
        summary = validate_readiness(rows)
    except (ValueError, OSError) as exc:
        print("invalid: %s" % exc, file=sys.stderr)
        return 1
    print(json.dumps(summary))
    return 0


if __name__ == "__main__":
    sys.exit(main())
