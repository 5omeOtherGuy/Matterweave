# Measurement qualification: same-build noise and capture overhead

`tools/performance/qualify_measurements.py` decides whether a set of already
collected trials **qualifies** as an offline comparison of

1. **same-build noise** — how much a metric moves between repeats of one build
   in one capture state, and
2. **capture on/off overhead** — the paired ON-minus-OFF difference for that
   same build.

The two are always reported separately. The tool reads one JSON document, does
no device access, and rejects anything it cannot qualify. It never fills a
missing value with zero and never derives an OFF number from an ON artifact.

This document is the input contract. It contains **no measured results**. The
Android qualification of the current build is **NOT RUN**; see
[Status of this gate](#status-of-this-gate).

## Relationship to the existing tools and evidence

| Piece | Owns |
| --- | --- |
| [`collect_wetland_pair.py`](../../tools/performance/collect_wetland_pair.py) | Collecting a trial on the device: install/identity checks, unplugged and foreground gating, SurfaceFlinger sampling, `/proc/<pid>/stat` CPU, health sampling, artifact hashing. `--mode capture-on-off` collects the same-build ON/OFF batch this contract needs; `--mode paired` (default) is the unchanged candidate/reference comparison. |
| [`analyze_wetland_pairs.py`](../../tools/performance/analyze_wetland_pairs.py) | Turning a collected batch into per-trial presentation/CPU/health statistics. |
| [`validate_frame_profile.py`](../../tools/performance/validate_frame_profile.py) | Strict format/identity validation of a schema-v2 capture CSV ([schema](measurement-v2.md)). |
| [`validate_conditions.py`](../../tools/performance/validate_conditions.py) | Idle-readiness/thermal/power validation of raw device dumps. |
| `qualify_measurements.py` (this contract) | Deciding whether a *set* of those trials is a legitimate noise / capture-overhead comparison, and reporting the two figures. |

The run and pair shape is the one already used by the executed
[2026-09-08 P01 overhead evidence](../evidence/2026-09-08-p01-overhead.json):
`runs[]` carrying `name`, `profiling`, `interval_count` and flat metric fields;
`pairs[]` carrying `on`, `off` and `on_vs_off_percent`; per-state
`range_over_median_percent`; `limitations`. The arithmetic is identical and is
regression-tested against that file's own recorded numbers, so the historical
step is reused, not repeated.

What is new is the qualification gate. The 2026-09-08 document records its
context in prose, so it cannot be machine-checked; consequently **it does not
qualify as a new candidate** and the tool says so. New candidates must restate
the gate fields per run.

## Why the timing source cannot be the capture CSV

A capture-OFF run writes no `frame-profile-v2-*.csv` at all: the capture is
requested by writing a frame count into `profile-frames.txt` and produces a file
only when active ([schema v2](measurement-v2.md)). There is therefore no OFF-side
value for `main_wall_ms`, `main_cpu_busy_ms` or any other CSV column, and no
honest overhead figure can be computed from them. Absence of the file is not a
zero and must not be treated as one.

The compared series must come from a source that exists in both states and is
not produced by the instrumentation under test:

- SurfaceFlinger presentation intervals (`dumpsys SurfaceFlinger --latency`),
  sampled by the collector and reduced by `analyze_wetland_pairs.py`;
- whole-process CPU from `/proc/<pid>/stat`, independently host-timed.

`timing_source.independent_of_capture` must be `true`, and a capture-derived
`kind` (`frame_profile_csv`, `metrics_rs`, `main_wall_ms`, …) is rejected.

Both series are collected identically in both capture states: the collector
samples `dumpsys SurfaceFlinger --latency` and `/proc/<pid>/stat` in an OFF
trial exactly as in an ON trial, and an OFF trial that produced no valid
presentation history is rejected rather than reduced.

## Collecting a same-build ON/OFF batch

`--mode capture-on-off` measures ONE build in both capture states. Compared
with the default paired mode it:

- takes a single `--apk` (with its frozen `*-build.json` manifest) and rejects
  `--candidate-apk`/`--reference-apk`;
- plans alternating OFF/ON, ON/OFF, OFF/ON trials, so device drift cannot line
  up with one capture state;
- re-checks after every trial that the *installed* APK hash is byte-identical
  to the previous trials, and aborts the batch otherwise;
- writes the profile request `files/profile-frames.txt` only in an ON trial. An
  OFF trial creates no request, therefore expects nothing to be consumed, and
  a `profile-frames.txt` that appears anyway is treated as another worker's
  file: it is neither read as ours nor deleted, and the trial is rejected;
- expects **zero** new `frame-profile*` files in an OFF trial. Any that appear
  are recorded in `unexpected-captures.json`, left on the device, and fail the
  trial, because the OFF state was not actually established;
- records `capture_state` explicitly in `trial.json`,
  `collection-complete.json`, `manifest.json` and `run-complete.json`, in both
  modes (every paired-mode trial is `"on"`). No consumer infers the state from
  the presence of a file.

Everything else is shared with the paired mode and unchanged: readiness
validation, thermal matching against the first trial, the recovery-slot fixture
and its ownership rules, foreground/unplugged gating, and the final cleanup
that removes only files this run created.

```sh
python3 tools/performance/collect_wetland_pair.py \
  --mode capture-on-off \
  --adb /absolute/platform-tools/adb --serial 192.168.178.93:42373 \
  --apk /absolute/frozen/explorer-candidate.apk \
  --fixture /absolute/frozen/wetland-session.json \
  --pairs 3 \
  --out /absolute/overhead-batch
```

The run fails closed and preserves artifacts; it makes no overhead claim. Only
`qualify_measurements.py` decides whether the batch qualifies.

## Document schema (`matterweave-qualification-v1`)

Top level. Unknown keys are rejected.

| Key | Required | Meaning |
| --- | --- | --- |
| `schema` | yes | Exactly `matterweave-qualification-v1`. |
| `build` | yes | Expected build identity: `source_commit`, `apk_sha256`, `composition_hash`, `generator`. |
| `conditions` | yes | Expected run conditions: `quality`, `scene`, `seed`, `replay`, `output_resolution`, `refresh_hz`. |
| `timing_source` | yes | `kind`, `tool`, `independent_of_capture` (must be `true`), optional `note`. |
| `metrics` | yes | Non-empty list of the metric names compared, e.g. `["mean_ms", "p95_ms", "p99_ms", "process_cpu_core_percent"]`. |
| `runs` | yes | The measured runs, in the order they were executed. |
| `evidence`, `purpose`, `note`, `limitations` | no | Free text; `limitations` is a list of strings appended to the tool's own list, never replacing it. |

Each entry of `runs` must carry all of the following. The top-level `build` and
`conditions` are the *expectation*; each run restates its own values so a run
that actually differed is detected. Nothing is inherited or defaulted.

| Key | Meaning | Source in the collected trial |
| --- | --- | --- |
| `name` | Unique run name. | Trial directory name. |
| `profiling` | Declared capture state: `"on"` or `"off"`. | The trial's own capture request; ON trials have exactly one capture CSV, OFF trials none. |
| `interval_count` | Number of qualified timing observations. | `summary.json` → `trials[].presentation.supported.interval_count`. |
| `capture_state` context | The collector's recorded state for the run, which must equal `profiling`. | `trial.json` → `capture_state`, echoed by `summary.json` → `trials[].capture_state`. |
| `artifact_sha256` | Digest of the verified artifact for *this* run; must be unique across runs. | `analyze_wetland_pairs.py` → `trials[].input_trial_sha256`, or the batch's own content manifest digest. |
| `build` | This run's `source_commit`, `apk_sha256`, `composition_hash`, `generator`. | `trial.json` → `apk` (frozen build manifest, verified against the installed APK by the collector). |
| `conditions` | This run's `quality`, `scene`, `seed`, `replay`, `output_resolution`, `refresh_hz`. | `trial.json` → `scene_loaded`, `fixture.seed`, shadow/quality settings from `session-after.json`, active SF mode from `display.txt`. `replay` names the input sequence; a stationary run with no injected input must say so explicitly (e.g. `"stationary-120s-no-input"`). |
| `observed` | Measured state, see below. | Health and environment artifacts. |
| `samples` | Missing-observation accounting, see below. | Collection and analysis artifacts. |
| declared metrics | One numeric field per name in `metrics`, all finite and greater than zero. | `presentation.supported.*`; `process_cpu_core_percent` is `process_cpu.logical_core_equivalent_utilization × 100`. |

Any other key (`long_intervals`, `thermal`, `profiles`, `content_manifest`, …)
is carried through as recorded context and listed in the report under
`recorded_context_keys`. It is never analysed.

### `observed`

| Key | Rule |
| --- | --- |
| `power_sources_live` | Must be `false`: no AC/USB/wireless/dock power live in any health sample. The collector already aborts a trial that sees external power. |
| `thermal_status_samples` | Non-empty list of the thermal statuses actually observed during the run. Every value must be `0`. An empty list means the thermal state was unobserved and the run is NOT QUALIFIED. |
| `battery_start_c`, `skin_start_c` | Start temperatures, from `trial.json` → `pre_launch_gate`. Across all runs the spread must stay within 1.0 °C (battery) and 2.0 °C (skin), the same tolerances the collector applies to a pair. |
| `active_display_mode` | The active SurfaceFlinger mode, e.g. `1440x3168@60.000004`. Must be identical across all runs. |

### `samples`

| Key | Rule |
| --- | --- |
| `missing_observation_count` | Observations recorded in the measurement window that were discarded. Must be `0`. Transcribe `summary.json` → `trials[].sampling.compositor_dumps_discarded`; do not count anything by hand. |
| `unsupported_gap_count` | `len(presentation.unsupported_gaps)` for this run. Must be `0`. |

## Rejection rules

The tool exits `2` with `NOT QUALIFIED: <reason>` on stderr and writes no
report when any of the following holds.

- Unknown, missing, null, empty, non-finite or non-positive fields anywhere in
  the contract; duplicate JSON keys; `NaN`/`Infinity`; input over 4 MiB.
- A metric absent from one run, or present as `null` or `0`. Missing is never
  zero.
- A run whose restated build identity or conditions differ from the expectation.
- An active display mode that differs across runs, a shared `artifact_sha256`,
  or start temperatures outside the paired tolerances.
- Live power, a nonzero thermal status, an unobserved thermal state, missing
  observations or unsupported gaps.
- A capture-derived or non-independent timing source.
- An undeclared capture state, a pair containing the same state twice, an odd number of
  runs, fewer than `--min-pairs` (default 3) alternating pairs, a run with
  fewer than `--min-samples` (default 1000) intervals, duplicate run names, or
  an empty `runs` list (reported as NOT RUN, nothing to qualify).
- An existing `--out`/`--out-markdown` path: outputs are created with `x` mode
  and never overwritten.

The defaults follow the protocol already in
[BENCHMARKS](../BENCHMARKS.md#comparison-procedure) — at least three repeatable
passes with alternated order. Each chronological pair contains one ON and one OFF
run; reversing the order between pairs is allowed and preserves counterbalancing.
Measured runs are never sorted to satisfy the validator. `--min-samples 1000` is roughly 17 s of 60 Hz
intervals; the executed 2026-09-08 runs recorded about 5500 over a 120 s window.

## Running it

Real data, once a qualified batch exists (paths are examples; the batch is the
lead's):

```sh
# 1. Collect alternating same-build ON/OFF trials (see the section above).
#    Never mix builds, scenes, seeds or display modes within one batch.
python3 tools/performance/collect_wetland_pair.py --mode capture-on-off \
  --adb /absolute/platform-tools/adb --serial 192.168.178.93:42373 \
  --apk /absolute/frozen/explorer-candidate.apk \
  --fixture /absolute/frozen/wetland-session.json \
  --pairs 3 --out /absolute/overhead-batch
# 2. Reduce the batch.
python3 tools/performance/analyze_wetland_pairs.py \
  --input /absolute/overhead-batch --out /absolute/overhead-analysis
# 3. Transcribe the per-run fields above into one qualification document
#    (see "What has no automatic source" for the fields no tool can supply).
# 4. Qualify.
python3 tools/performance/qualify_measurements.py \
  --input /absolute/overhead-analysis/qualification.json \
  --out /absolute/overhead-analysis/qualification-report.json \
  --out-markdown /absolute/overhead-analysis/qualification-report.md
```

Contract self-check without any device, using the synthetic document built by
the tests (its numbers are invented arithmetic fixtures, not measurements):

```sh
python3 - <<'PY'
import json, sys
sys.path.insert(0, "tools/performance")
from test_qualify_measurements import document
json.dump(document(), open("/tmp/qualification-example.json", "w"), indent=2)
PY
python3 tools/performance/qualify_measurements.py --input /tmp/qualification-example.json
```

Tests:

```sh
python3 -m unittest discover -s tools/performance -p 'test_*.py'
```

### What has no automatic source

The qualification document is transcribed by hand, and nothing in it may be
invented to satisfy a required field. Only these come straight from the tools:

- `interval_count`, the declared metrics and `unsupported_gap_count` from
  `summary.json` → `trials[].presentation` / `trials[].process_cpu`;
- `missing_observation_count` from `trials[].sampling.compositor_dumps_discarded`.
  The collector polls best effort and schedules no fixed number of compositor
  dumps, so there is no scheduled-minus-obtained figure to report; the analyzer
  rejects any dump with a nonzero exit code or a non-monotonic host time, so a
  trial that was reduced at all discarded none. A trial the analyzer rejected
  produces no row and cannot be transcribed as a qualified run at all;
- `artifact_sha256` from `trials[].input_trial_sha256`;
- `build` from `trials[].apk`, `observed.battery_start_c`/`skin_start_c` from
  `trials[].pre_launch_gate`, `thermal_status_samples` from
  `trials[].health_measurement.metrics.thermal_status` and
  `health_measurement.thermal_status_counts`;
- `profiling` from the collector's recorded `capture_state`.

`conditions.replay`, `conditions.quality`, `conditions.output_resolution` and
`conditions.refresh_hz` are **not** derived by any tool. `output_resolution`
and `refresh_hz`/`active_display_mode` must be read out of the run's own
`display.txt`, and `replay` must describe the input sequence that was actually
performed (a stationary run with no injected input says so). If a value cannot
be supported by an artifact of that run, the batch is not transcribed: an
unsupported field is a NOT RUN, never a plausible default.

## What a qualified report does not establish

Repeated in every report under `limitations`: no statistical significance or
confidence interval; no energy, power, battery or thermal claim; no sustained
(20-minute) behaviour; whole-process CPU rate is not per-frame instrumentation
cost; no GPU-only overhead, driver-internal wait or scanout identity; no
visual-quality or temporal-stability claim; ambient and case conditions
unobserved. Only the difference between the two declared capture states is
measured — the baseline clock readings both states pay
([schema v2, Timings](measurement-v2.md#timings)) stay unmeasured. A
display-bound interval can hide real CPU overhead, so compare the metric
against the declared refresh rate before concluding there is none.

`resolution` in the report compares each mean overhead against the larger
same-state range. That is a descriptive comparison of two observed spreads, not
a significance test.

## Status of this gate

- Tool, contract, ON/OFF collection mode and tests: implemented and passing on
  the host. The collection mode has never been executed against a phone.
- Same-build noise and capture-overhead qualification for the current build on
  the OnePlus 13: **NOT RUN**. No device work was performed for this change and
  no new measurement is claimed. The executed
  [2026-09-08 P01 check](../evidence/2026-09-08-p01-overhead.md) covers an older
  instrumentation candidate on a short stationary workload and does not qualify
  the current build.
