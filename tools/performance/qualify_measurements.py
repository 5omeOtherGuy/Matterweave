#!/usr/bin/env python3
"""Fail-closed qualification of same-build noise and capture on/off overhead.

Scope: this tool consumes *already measured* run metadata plus timings and
decides whether that set qualifies as an OFFLINE noise / capture-overhead
comparison for a **new** candidate build. It performs no device access,
launches nothing, and never invents, imputes or zero-fills a missing
measurement. It is an analyzer, not a collector: `collect_wetland_pair.py`
remains the collector and `validate_frame_profile.py` remains the capture-CSV
validator.

Evidence contract: the run/pair shape is the one already used by
`docs/evidence/2026-09-08-p01-overhead.json` — `runs[]` with `name`,
`profiling` ("on"/"off"), `interval_count` and flat metric fields; `pairs[]`
with `on`, `off` and `on_vs_off_percent`; `range_over_median_percent` per
capture state; `limitations`. The arithmetic here reproduces that file's own
recorded numbers (regression-tested against it), so this is the same reporting
step made executable and strict, not a second parallel format. The historical
run does not have to be redone.

What is *added* for new candidates, and what the historical file lacks, is the
qualification gate. Every gate field is **per run**, transcribed from that
run's own verified artifact, and then compared: a single top-level declaration
cannot detect a run that actually differed, so top-level blocks are treated as
the expectation and each run must restate its own build identity, conditions,
observed power/thermal state, sample accounting and artifact digest. Missing
samples, unsupported gaps, a nonzero thermal status, any live power source, an
unmatched active display mode or start temperatures outside the collector's
paired tolerances all yield NOT QUALIFIED. Nothing is defaulted.

Why an explicit input document instead of reading a capture directory: a
capture-OFF run writes **no** `frame-profile-v2-*.csv` at all (see
docs/performance/measurement-v2.md, "Requesting a capture"). Any overhead
number derived from the app CSV would therefore only ever describe the ON side.
The timing series compared here must come from a source that exists in both
states and is not produced by the instrumentation under test — SurfaceFlinger
presentation intervals and `/proc/<pid>/stat` process CPU, both already
collected by `collect_wetland_pair.py` and summarised by
`analyze_wetland_pairs.py`. The input document must declare that source; a
capture-derived source is rejected.

Two questions are answered separately and never merged:

1. same-build noise: spread of each metric across repeats *within one capture
   state* of one build. This is the floor below which no difference means
   anything.
2. capture overhead: the paired ON-minus-OFF difference over alternating
   repeats of that same build.

Input contract: docs/performance/measurement-qualification.md.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import statistics
import sys
from pathlib import Path

# Reuse the existing strict JSON scalar/duplicate-key rejections rather than
# re-deriving them; same directory, no import side effects.
from validate_conditions import (_finite_number, _reject_duplicate_keys,
                                 _reject_json_constant)

SCHEMA = "matterweave-qualification-v1"

MAX_INPUT_BYTES = 4 * 1024 * 1024

# Build identity: every run must agree on all of these.
BUILD_KEYS = ("source_commit", "apk_sha256", "composition_hash", "generator")
# Declared run conditions: every run must agree on all of these.
CONDITION_KEYS = ("quality", "scene", "seed", "replay", "output_resolution",
                  "refresh_hz")
CAPTURE_STATES = ("on", "off")

TIMING_SOURCE_KEYS = ("kind", "tool", "independent_of_capture")
# Sources that exist only while the instrumentation under test is enabled.
# An OFF run produces none of these, so they cannot time both states.
CAPTURE_DERIVED_SOURCES = frozenset({
    "frame_profile_csv", "frame-profile-csv", "frame_profile_v2",
    "frame-profile-v2", "app_capture_csv", "metrics_rs", "main_wall_ms",
    "main_cpu_busy_ms",
})

# Every run restates its own identity/conditions/state; nothing is inherited.
# Other historical keys (long_intervals, thermal, profiles, content_manifest,
# ...) are recorded context, carried through and never analysed.
RUN_REQUIRED_KEYS = ("name", "profiling", "interval_count", "artifact_sha256",
                     "build", "conditions", "observed", "samples")

OBSERVED_KEYS = ("power_sources_live", "thermal_status_samples",
                 "battery_start_c", "skin_start_c", "active_display_mode")
SAMPLE_KEYS = ("missing_observation_count", "unsupported_gap_count")

# Paired tolerances already used by the collector and validate_conditions.
MAX_BATTERY_RANGE_C = 1.0
MAX_SKIN_RANGE_C = 2.0

_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")

TOP_REQUIRED_KEYS = ("schema", "build", "conditions", "timing_source",
                     "metrics", "runs")
TOP_OPTIONAL_KEYS = ("evidence", "purpose", "note", "limitations")

DEFAULT_MIN_PAIRS = 3
# A floor against trivially short runs. 1000 presentation intervals is about
# 17 s at 60 Hz; the 2026-09-08 runs recorded ~5500 over a 120 s window.
DEFAULT_MIN_SAMPLES = 1000

# Fixed limitations of this analysis. Input-declared limitations are appended,
# never replaced.
TOOL_LIMITATIONS = [
    "Statistical significance, confidence intervals and distribution models "
    "are not established: a handful of alternating repeats is descriptive "
    "evidence only.",
    "No energy, power, battery or thermal claim.",
    "No sustained-workload claim; nothing here extends to a 20-minute run.",
    "Whole-process CPU rate is not per-frame instrumentation cost.",
    "No GPU-only overhead, driver-internal wait or scanout/presentation "
    "identity claim.",
    "Only the difference between the declared capture states is measured. The "
    "baseline clock readings both states pay (docs/performance/"
    "measurement-v2.md, Timings) stay unmeasured.",
    "No visual-quality, temporal-stability or gameplay-equivalence claim.",
    "A display-bound interval can hide real CPU overhead; compare the metric "
    "against the declared refresh rate before concluding there is none.",
    "Per-run power, thermal, display-mode and sample-accounting fields are "
    "transcribed from each run's verified collector artifact and checked for "
    "consistency here. This tool does not re-parse the raw device dumps; "
    "validate_conditions.py and collect_wetland_pair.py own that check.",
    "Ambient and case conditions are unobserved.",
]


class QualificationError(ValueError):
    """Input rejected: the data does not qualify. Never downgraded to a value."""


# ---------------------------------------------------------------------------
# Strict field checks
# ---------------------------------------------------------------------------

def _require_mapping(value, what):
    if not isinstance(value, dict):
        raise QualificationError("%s must be a JSON object, got %s"
                                 % (what, type(value).__name__))
    return value


def _require_keys(mapping, required, optional, what):
    present = set(mapping)
    missing = [key for key in required if key not in present]
    if missing:
        raise QualificationError("%s is missing %s"
                                 % (what, ", ".join(sorted(missing))))
    unknown = sorted(present - set(required) - set(optional))
    if unknown:
        raise QualificationError("%s has unknown key(s) %s"
                                 % (what, ", ".join(unknown)))


def _require_scalar(value, what):
    """A declared identity/condition: a non-empty string, number or boolean."""
    if value is None:
        raise QualificationError("%s is null; an unrecorded value must not be "
                                 "declared" % what)
    if isinstance(value, str):
        if not value.strip():
            raise QualificationError("%s is empty" % what)
        return value
    if isinstance(value, bool) or isinstance(value, int):
        return value
    if isinstance(value, float):
        return _finite_number(value, what)
    raise QualificationError("%s must be a string, number or boolean, got %s"
                             % (what, type(value).__name__))


def _require_positive(value, what):
    if value is None:
        raise QualificationError(
            "%s is null; a missing measurement disqualifies the run and is "
            "never substituted with zero" % what)
    number = _finite_number(value, what)
    if number <= 0:
        raise QualificationError(
            "%s must be greater than zero, got %r; a missing measurement is "
            "never reported as zero" % (what, value))
    return number


def _require_positive_int(value, what):
    if isinstance(value, bool) or not isinstance(value, int):
        raise QualificationError("%s must be an integer, got %r" % (what, value))
    if value <= 0:
        raise QualificationError("%s must be greater than zero, got %d"
                                 % (what, value))
    return value


def load_document(path):
    """Read the qualification document read-only, bounded, strictly parsed."""
    with open(path, "rb") as source:
        raw = source.read(MAX_INPUT_BYTES + 1)
    if len(raw) > MAX_INPUT_BYTES:
        raise QualificationError("input exceeds %d byte limit" % MAX_INPUT_BYTES)
    if not raw.strip():
        raise QualificationError("input file is empty")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise QualificationError("input is not valid UTF-8: %s" % exc) from None
    try:
        document = json.loads(text, parse_constant=_reject_json_constant,
                              object_pairs_hook=_reject_duplicate_keys)
    except ValueError as exc:
        raise QualificationError("input is not valid qualification JSON: %s"
                                 % exc) from None
    return document, hashlib.sha256(raw).hexdigest()


def _check_build(build, what):
    _require_mapping(build, what)
    _require_keys(build, BUILD_KEYS, (), what)
    return {key: _require_scalar(build[key], "%s.%s" % (what, key))
            for key in BUILD_KEYS}


def _check_conditions(conditions, what):
    _require_mapping(conditions, what)
    _require_keys(conditions, CONDITION_KEYS, (), what)
    checked = {key: _require_scalar(conditions[key], "%s.%s" % (what, key))
               for key in CONDITION_KEYS}
    checked["refresh_hz"] = _require_positive(conditions["refresh_hz"],
                                              "%s.refresh_hz" % what)
    if not isinstance(conditions["replay"], (str, bool)):
        raise QualificationError(
            "%s.replay must name the replay/input sequence or be a boolean "
            "declaration, got %r" % (what, conditions["replay"]))
    return checked


def _check_timing_source(source):
    _require_mapping(source, "timing_source")
    _require_keys(source, TIMING_SOURCE_KEYS, ("note",), "timing_source")
    kind = _require_scalar(source["kind"], "timing_source.kind")
    tool = _require_scalar(source["tool"], "timing_source.tool")
    if source["independent_of_capture"] is not True:
        raise QualificationError(
            "timing_source.independent_of_capture must be true; a timing "
            "series produced by the instrumentation under test cannot measure "
            "its own overhead")
    if str(kind).strip().lower() in CAPTURE_DERIVED_SOURCES:
        raise QualificationError(
            "timing_source.kind %r is produced only while capture is on, so "
            "no capture-off timing exists for it; use a source present in "
            "both states (SurfaceFlinger presentation intervals or "
            "/proc/<pid>/stat process CPU)" % kind)
    result = {"kind": kind, "tool": tool, "independent_of_capture": True}
    if "note" in source:
        result["note"] = _require_scalar(source["note"], "timing_source.note")
    return result


def _check_metric_names(names):
    if not isinstance(names, list) or not names:
        raise QualificationError("metrics must be a non-empty list of metric "
                                 "names")
    checked = []
    for index, name in enumerate(names):
        if not isinstance(name, str) or not name.strip():
            raise QualificationError("metrics[%d] must be a non-empty string"
                                     % index)
        if name in RUN_REQUIRED_KEYS:
            raise QualificationError("metrics[%d] %r collides with a reserved "
                                     "run key" % (index, name))
        if name in checked:
            raise QualificationError("metrics lists %r twice" % name)
        checked.append(name)
    return checked


def _check_observed(raw, what):
    """Per-run power/thermal state, transcribed from that run's own artifact."""
    _require_mapping(raw, what)
    _require_keys(raw, OBSERVED_KEYS, ("note",), what)
    live = raw["power_sources_live"]
    if live is not False:
        raise QualificationError(
            "%s.power_sources_live must be false; a run with AC/USB/wireless/"
            "dock power live is not comparable and is NOT QUALIFIED (got %r)"
            % (what, live))
    samples = raw["thermal_status_samples"]
    if not isinstance(samples, list) or not samples:
        raise QualificationError(
            "%s.thermal_status_samples must be a non-empty list of observed "
            "thermal statuses; an unobserved thermal state is NOT QUALIFIED"
            % what)
    statuses = []
    for position, value in enumerate(samples):
        if isinstance(value, bool) or not isinstance(value, int):
            raise QualificationError("%s.thermal_status_samples[%d] must be an "
                                     "integer, got %r" % (what, position, value))
        if value != 0:
            raise QualificationError(
                "%s.thermal_status_samples[%d] is %d; a throttled run is NOT "
                "QUALIFIED for comparison" % (what, position, value))
        statuses.append(value)
    battery = _require_positive(raw["battery_start_c"], "%s.battery_start_c" % what)
    skin = _require_positive(raw["skin_start_c"], "%s.skin_start_c" % what)
    mode = _require_scalar(raw["active_display_mode"],
                           "%s.active_display_mode" % what)
    return {"power_sources_live": False, "thermal_status_samples": statuses,
            "battery_start_c": battery, "skin_start_c": skin,
            "active_display_mode": mode}


def _check_samples(raw, what):
    """Per-run accounting for observations that were not obtained."""
    _require_mapping(raw, what)
    _require_keys(raw, SAMPLE_KEYS, ("note",), what)
    counts = {}
    for key in SAMPLE_KEYS:
        value = raw[key]
        if isinstance(value, bool) or not isinstance(value, int) or value < 0:
            raise QualificationError("%s.%s must be a non-negative integer, "
                                     "got %r" % (what, key, value))
        if value > 0:
            raise QualificationError(
                "%s.%s is %d; missing observations are NOT QUALIFIED and are "
                "never treated as zero-cost intervals" % (what, key, value))
        counts[key] = value
    return counts


def _check_run(raw, index, metric_names, build, conditions, min_samples):
    """Validate one measured run. Extra recorded context is carried, not used."""
    what = "runs[%d]" % index
    _require_mapping(raw, what)
    missing = [key for key in RUN_REQUIRED_KEYS if key not in raw]
    if missing:
        raise QualificationError("%s is missing %s"
                                 % (what, ", ".join(sorted(missing))))
    name = _require_scalar(raw["name"], "%s.name" % what)
    what = "run %r" % name
    if raw["profiling"] not in CAPTURE_STATES:
        raise QualificationError(
            "%s.profiling must be declared as 'on' or 'off', got %r; an "
            "undeclared capture state disqualifies the run"
            % (what, raw["profiling"]))
    samples = _require_positive_int(raw["interval_count"],
                                    "%s.interval_count" % what)
    if samples < min_samples:
        raise QualificationError(
            "%s.interval_count %d is below the required floor %d"
            % (what, samples, min_samples))

    digest = raw["artifact_sha256"]
    if not isinstance(digest, str) or not _SHA256_RE.match(digest):
        raise QualificationError(
            "%s.artifact_sha256 must be a lowercase 64-hex digest of the "
            "verified collector artifact for this run, got %r" % (what, digest))
    run_build = _check_build(raw["build"], "%s.build" % what)
    run_conditions = _check_conditions(raw["conditions"], "%s.conditions" % what)
    observed = _check_observed(raw["observed"], "%s.observed" % what)
    accounting = _check_samples(raw["samples"], "%s.samples" % what)

    absent = [key for key in metric_names if key not in raw]
    if absent:
        raise QualificationError(
            "%s is missing declared metric(s) %s; a metric absent from one run "
            "cannot be compared and is never filled with zero"
            % (what, ", ".join(sorted(absent))))
    metrics = {key: _require_positive(raw[key], "%s.%s" % (what, key))
               for key in metric_names}
    context = sorted(set(raw) - set(RUN_REQUIRED_KEYS) - set(metric_names))
    return {"name": name, "profiling": raw["profiling"],
            "interval_count": samples, "artifact_sha256": digest,
            "build": run_build, "conditions": run_conditions,
            "observed": observed, "samples": accounting, "metrics": metrics,
            "context_keys": context}


def _check_matching(runs, build, conditions):
    """Every run must be the same build under the same measured conditions.

    Each run's own restated values are compared against the expectation and
    against the other runs, so one accurate top-level block cannot hide a run
    that actually differed.
    """
    mismatches = []
    for run in runs:
        for key in BUILD_KEYS:
            if run["build"][key] != build[key]:
                mismatches.append("%s: build.%s %r != declared %r"
                                  % (run["name"], key, run["build"][key],
                                     build[key]))
        for key in CONDITION_KEYS:
            if run["conditions"][key] != conditions[key]:
                mismatches.append("%s: conditions.%s %r != declared %r"
                                  % (run["name"], key, run["conditions"][key],
                                     conditions[key]))
    modes = {run["observed"]["active_display_mode"] for run in runs}
    if len(modes) > 1:
        mismatches.append("active display mode differs across runs: %s"
                          % ", ".join(sorted(repr(mode) for mode in modes)))
    digests = [run["artifact_sha256"] for run in runs]
    if len(set(digests)) != len(digests):
        mismatches.append("two runs cite the same artifact_sha256; each run "
                          "needs its own verified artifact")
    for field, bound in (("battery_start_c", MAX_BATTERY_RANGE_C),
                         ("skin_start_c", MAX_SKIN_RANGE_C)):
        values = [run["observed"][field] for run in runs]
        span = max(values) - min(values)
        if span > bound:
            mismatches.append("%s spread %.3f C across runs exceeds %.1f C"
                              % (field, span, bound))
    if mismatches:
        raise QualificationError(
            "runs are not the same build under matched conditions: "
            + "; ".join(mismatches))


def _sequence(runs, min_pairs):
    """Require opposite capture states within each chronological pair.

    Pair order may reverse to counterbalance drift (ON/OFF, OFF/ON, ON/OFF).
    Never reorder measured runs or require opposite states across pair boundaries.
    """
    names = [run["name"] for run in runs]
    if len(set(names)) != len(names):
        raise QualificationError("run names must be unique")
    if len(runs) % 2 != 0:
        raise QualificationError(
            "an alternating comparison needs complete pairs; got %d run(s)"
            % len(runs))
    pairs = list(zip(runs[0::2], runs[1::2]))
    for first, second in pairs:
        if first["profiling"] == second["profiling"]:
            raise QualificationError(
                "capture states must alternate within each pair; %r and %r are "
                "both %r" % (first["name"], second["name"], first["profiling"]))
    if len(pairs) < min_pairs:
        raise QualificationError(
            "%d alternating pair(s) is below the required %d"
            % (len(pairs), min_pairs))
    return pairs


# ---------------------------------------------------------------------------
# Analysis. Definitions match docs/evidence/2026-09-08-p01-overhead.json.
# ---------------------------------------------------------------------------

def _spread(values):
    low, high = min(values), max(values)
    median = statistics.median(values)
    span = high - low
    return {
        "repeats": len(values),
        "min": low,
        "max": high,
        "mean": statistics.mean(values),
        "median": median,
        "range": span,
        # Same definition as the historical range_over_median_percent.
        "range_over_median_percent": 100.0 * span / median,
    }


def same_build_noise(runs, metric_names):
    """Spread within each capture state. The two states stay separate."""
    noise = {}
    for state in CAPTURE_STATES:
        members = [run for run in runs if run["profiling"] == state]
        noise[state] = {
            "runs": [run["name"] for run in members],
            "metrics": {name: _spread([run["metrics"][name]
                                       for run in members])
                        for name in metric_names},
        }
    return noise


def capture_overhead(pairs, metric_names):
    """Paired on-minus-off differences, never merged with the noise figures.

    `on_vs_off_percent` is 100*(on-off)/off, the historical definition.
    """
    entries = []
    for first, second in pairs:
        on = first if first["profiling"] == "on" else second
        off = second if first["profiling"] == "on" else first
        entries.append({
            "on": on["name"],
            "off": off["name"],
            "on_vs_off_absolute": {
                name: on["metrics"][name] - off["metrics"][name]
                for name in metric_names},
            "on_vs_off_percent": {
                name: 100.0 * (on["metrics"][name] - off["metrics"][name])
                / off["metrics"][name]
                for name in metric_names},
        })
    summary = {}
    for name in metric_names:
        absolute = [entry["on_vs_off_absolute"][name] for entry in entries]
        percent = [entry["on_vs_off_percent"][name] for entry in entries]
        summary[name] = {
            "pair_count": len(entries),
            "mean_on_vs_off_absolute": statistics.mean(absolute),
            "min_on_vs_off_absolute": min(absolute),
            "max_on_vs_off_absolute": max(absolute),
            "mean_on_vs_off_percent": statistics.mean(percent),
            "min_on_vs_off_percent": min(percent),
            "max_on_vs_off_percent": max(percent),
        }
    return {"pairs": entries, "summary": summary}


def resolution(noise, overhead, metric_names):
    """Compare each mean overhead against the larger same-state spread.

    A separated descriptive verdict, not a significance test: it only says
    whether the paired difference is distinguishable from the observed
    same-build repeat spread.
    """
    verdicts = {}
    for name in metric_names:
        floor = max(noise[state]["metrics"][name]["range"]
                    for state in CAPTURE_STATES)
        mean_delta = overhead["summary"][name]["mean_on_vs_off_absolute"]
        verdicts[name] = {
            "same_build_noise_range": floor,
            "mean_on_vs_off_absolute": mean_delta,
            "abs_mean_over_noise_range": (abs(mean_delta) / floor
                                          if floor > 0 else None),
            "verdict": ("no observed difference" if mean_delta == 0
                        else "below same-build noise range"
                        if abs(mean_delta) <= floor
                        else "exceeds same-build noise range"),
            "note": "Descriptive comparison of two observed spreads. Not a "
                    "significance test and not a per-frame cost model.",
        }
    return verdicts


def qualify(document, input_sha256, min_pairs=DEFAULT_MIN_PAIRS,
            min_samples=DEFAULT_MIN_SAMPLES):
    """Validate the document and return the qualification report.

    Raises QualificationError on anything that disqualifies the data set.
    """
    _require_mapping(document, "input document")
    _require_keys(document, TOP_REQUIRED_KEYS, TOP_OPTIONAL_KEYS,
                  "input document")
    if document["schema"] != SCHEMA:
        raise QualificationError("unsupported schema %r, expected %r"
                                 % (document["schema"], SCHEMA))
    build = _check_build(document["build"], "build")
    conditions = _check_conditions(document["conditions"], "conditions")
    timing_source = _check_timing_source(document["timing_source"])
    metric_names = _check_metric_names(document["metrics"])
    raw_runs = document["runs"]
    if not isinstance(raw_runs, list):
        raise QualificationError("runs must be a list")
    if not raw_runs:
        raise QualificationError("runs is empty: NOT RUN, nothing to qualify")
    runs = [_check_run(raw, index, metric_names, build, conditions, min_samples)
            for index, raw in enumerate(raw_runs)]
    _check_matching(runs, build, conditions)
    pairs = _sequence(runs, min_pairs)
    noise = same_build_noise(runs, metric_names)
    overhead = capture_overhead(pairs, metric_names)
    declared = document.get("limitations", [])
    if not isinstance(declared, list) or not all(isinstance(item, str)
                                                 for item in declared):
        raise QualificationError("limitations must be a list of strings")
    return {
        "tool": Path(__file__).name,
        "tool_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "input_sha256": input_sha256,
        "schema": SCHEMA,
        "qualified": True,
        "evidence": document.get("evidence"),
        "build": build,
        "conditions": conditions,
        "timing_source": timing_source,
        "metrics": metric_names,
        "min_pairs_required": min_pairs,
        "min_interval_count_required": min_samples,
        "runs": [{"name": run["name"], "profiling": run["profiling"],
                  "interval_count": run["interval_count"],
                  "artifact_sha256": run["artifact_sha256"],
                  "observed": run["observed"], "samples": run["samples"],
                  "metrics": run["metrics"],
                  "recorded_context_keys": run["context_keys"]}
                 for run in runs],
        "pair_count": len(pairs),
        "same_build_noise": noise,
        "range_over_median_percent": {
            state: {name: noise[state]["metrics"][name]
                    ["range_over_median_percent"] for name in metric_names}
            for state in CAPTURE_STATES},
        "capture_overhead": overhead,
        "resolution": resolution(noise, overhead, metric_names),
        "limitations": TOOL_LIMITATIONS + list(declared),
    }


def render_markdown(report):
    conditions = report["conditions"]
    lines = ["# Measurement qualification", "",
             "Same-build noise and capture on/off overhead, reported "
             "separately. Descriptive evidence only.", "",
             "- Build: commit `%s`, APK `%s`"
             % (report["build"]["source_commit"], report["build"]["apk_sha256"]),
             "- Conditions: quality `%s`, scene `%s`, seed `%s`, replay `%s`, "
             "%s at %s Hz" % (conditions["quality"], conditions["scene"],
                              conditions["seed"], conditions["replay"],
                              conditions["output_resolution"],
                              conditions["refresh_hz"]),
             "- Timing source: `%s` via `%s`, independent of the capture under "
             "test" % (report["timing_source"]["kind"],
                       report["timing_source"]["tool"]),
             "- Alternating pairs: %d" % report["pair_count"], "",
             "## Same-build noise (within one capture state)", "",
             "| Capture | Metric | Repeats | Min | Max | Range | "
             "Range % of median |",
             "| --- | --- | ---: | ---: | ---: | ---: | ---: |"]
    for state in CAPTURE_STATES:
        for name in report["metrics"]:
            stats = report["same_build_noise"][state]["metrics"][name]
            lines.append("| %s | %s | %d | %.4f | %.4f | %.4f | %.4f |"
                         % (state, name, stats["repeats"], stats["min"],
                            stats["max"], stats["range"],
                            stats["range_over_median_percent"]))
    lines += ["", "## Capture on/off overhead (paired, on minus off)", "",
              "| Metric | Pairs | Mean | Min | Max | Mean % | "
              "Against noise range |",
              "| --- | ---: | ---: | ---: | ---: | ---: | --- |"]
    for name in report["metrics"]:
        summary = report["capture_overhead"]["summary"][name]
        lines.append("| %s | %d | %.4f | %.4f | %.4f | %.4f | %s |"
                     % (name, summary["pair_count"],
                        summary["mean_on_vs_off_absolute"],
                        summary["min_on_vs_off_absolute"],
                        summary["max_on_vs_off_absolute"],
                        summary["mean_on_vs_off_percent"],
                        report["resolution"][name]["verdict"]))
    lines += ["", "## Limitations and non-claims", ""]
    lines += ["- " + item for item in report["limitations"]]
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Qualify same-build noise and capture on/off overhead from "
                    "an explicit measured-run document.")
    parser.add_argument("--input", required=True, type=Path,
                        help="qualification JSON document (read-only)")
    parser.add_argument("--out", type=Path,
                        help="write the JSON report to this new file")
    parser.add_argument("--out-markdown", type=Path,
                        help="write the Markdown summary to this new file")
    parser.add_argument("--min-pairs", type=int, default=DEFAULT_MIN_PAIRS,
                        help="minimum alternating on/off pairs (default %d)"
                             % DEFAULT_MIN_PAIRS)
    parser.add_argument("--min-samples", type=int, default=DEFAULT_MIN_SAMPLES,
                        help="minimum interval_count per run (default %d)"
                             % DEFAULT_MIN_SAMPLES)
    args = parser.parse_args(argv)
    try:
        if args.min_pairs < 1 or args.min_samples < 1:
            raise QualificationError("--min-pairs and --min-samples must be at "
                                     "least 1")
        document, digest = load_document(args.input)
        report = qualify(document, digest, args.min_pairs, args.min_samples)
    except QualificationError as error:
        print("NOT QUALIFIED: %s" % error, file=sys.stderr)
        return 2
    except OSError as error:
        print("input unreadable: %s" % error, file=sys.stderr)
        return 2
    text = json.dumps(report, indent=2, allow_nan=False) + "\n"
    if args.out:
        with open(args.out, "x", encoding="utf-8") as sink:
            sink.write(text)
    markdown = render_markdown(report)
    if args.out_markdown:
        with open(args.out_markdown, "x", encoding="utf-8") as sink:
            sink.write(markdown)
    print(markdown, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
