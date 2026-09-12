# E2 measurement qualification — worker log, 2026-09-12

Task: close the missing OFFLINE qualification step for same-build noise and
capture-overhead evidence with a tested executable tool. Base commit `704bb4a`.
Worktree `/mnt/bench/matterweave-dev/worktrees/e2-measurement-20260912`.
Actual device measurement remains lead-owned and is **NOT RUN**.

## Actions taken

1. Read the existing surface before writing anything:
   [`analyze_wetland_pairs.py`](../../../tools/performance/analyze_wetland_pairs.py),
   [`collect_wetland_pair.py`](../../../tools/performance/collect_wetland_pair.py),
   [`validate_frame_profile.py`](../../../tools/performance/validate_frame_profile.py),
   [`validate_conditions.py`](../../../tools/performance/validate_conditions.py),
   [schema v2](../measurement-v2.md), [BENCHMARKS](../../BENCHMARKS.md) and — after
   lead steering — the executed
   [2026-09-08 P01 overhead evidence](../../evidence/2026-09-08-p01-overhead.json)
   and its [prose summary](../../evidence/2026-09-08-p01-overhead.md).
2. Implemented `tools/performance/qualify_measurements.py`: a fail-closed
   offline analyzer that takes one JSON document of already measured runs and
   either produces a qualification report or exits `2` with
   `NOT QUALIFIED: <reason>`.
3. Implemented `tools/performance/test_qualify_measurements.py`: 45 tests —
   known arithmetic fixtures, real-evidence reproduction, adversarial
   rejections, ordering/sign mutations and CLI behaviour.
4. Wrote the input contract
   [`docs/performance/measurement-qualification.md`](../measurement-qualification.md)
   with per-field provenance from the collector artifacts and runnable commands.
5. Verified: full performance suite, three deliberate source mutations, docs
   check, and both CLI paths (qualified synthetic document, rejected historical
   file).

## Issues and friction

- **The obvious data source does not exist for half the comparison.** A
  capture-OFF run writes no `frame-profile-v2-*.csv` at all (schema v2,
  "Requesting a capture"), so every CSV column is ON-only. Any overhead figure
  built from the CSV would silently be an ON-vs-nothing comparison. The tool
  therefore requires an explicitly declared timing source that exists in both
  states and rejects capture-derived kinds outright.
- **A second parallel evidence format was nearly created.** The first draft
  invented a `trials[]`/`metrics{}` shape. Lead steering pointed at
  `2026-09-08-p01-overhead.json`, which already defines runs, pairs,
  `on_vs_off_percent`, `range_over_median_percent` and `limitations`. The draft
  was rewritten onto that shape.
- **One accurate top-level metadata block proves nothing.** The first draft let
  a run inherit the top-level build/conditions when it omitted them, which makes
  a mismatched run undetectable by construction. Corrected: every gate field is
  per run and is compared against the expectation and against the other runs.
- **Thermal/power and missing samples were initially unqualified.** They were
  neither required nor checked, so a throttled or plugged-in run would have been
  analysed silently. Corrected by requiring per-run `observed` and `samples`
  blocks with explicit rejection rules.
- The historical evidence file has no machine-readable context, so it cannot be
  qualified as a new candidate. Rather than weaken the gate, that outcome is
  asserted as a test.

## Decisions and rationale

| Decision | Rationale |
| --- | --- |
| Reuse the 2026-09-08 run/pair/percentage shape and definitions verbatim | The historical step stays valid and is not repeated; the arithmetic is regression-tested against its own recorded numbers, which is stronger evidence than any invented fixture. |
| Explicit input document rather than reading a capture directory | The OFF side has no capture artifacts to read. Directory scraping would have to infer the OFF state from absence, which is exactly the substitution the task forbids. |
| Per-run restatement of build, conditions, power/thermal state, sample accounting and artifact digest | A single declaration cannot detect a run that differed. Restating and cross-checking can. |
| Reject rather than annotate: live power, nonzero thermal status, unobserved thermal state, missing observations, unsupported gaps, shared artifact digests | These are disqualifying conditions for a noise/overhead comparison, not caveats to be printed under a number. |
| Noise and overhead computed and reported by separate functions and separate report sections | The task requires them distinct; merging them would let capture cost hide inside repeat spread. |
| `resolution` compares mean overhead to the larger same-state range and labels it "not a significance test" | Useful and honest; three pairs cannot support a confidence interval. |
| Defaults: 3 alternating pairs, 1000 intervals per run, tolerances 1.0 °C battery / 2.0 °C skin | Taken from BENCHMARKS ("at least three repeatable passes", alternated order), the executed 120 s runs (~5500 intervals), and the collector's existing paired thermal tolerances. All overridable except the tolerances. |
| Reuse `validate_conditions._finite_number`, `_reject_json_constant`, `_reject_duplicate_keys` | Strict JSON handling already exists in this directory; re-deriving it would be a second contract to keep in sync. |
| No edits to the collector, the renderer or any existing source file | Out of scope and unowned. |

## Solutions applied

- Timing-source gate: `independent_of_capture` must be `true` and `kind` must
  not be in `CAPTURE_DERIVED_SOURCES`, with the reason spelled out in the error.
- No-zero-substitution: `_require_positive` rejects `null`, `0`, negatives and
  non-finite values with messages that name the rule; a metric absent from any
  run rejects the whole document.
- Matching: `_check_matching` compares each run's own build/conditions against
  the expectation, requires one shared active display mode, requires distinct
  artifact digests and bounds the battery/skin start spread across all runs.
- Sequencing: `_sequence` requires unique names, an even run count, strict
  alternation in recorded order and at least `--min-pairs` pairs.
- Reporting: `same_build_noise` per state, `capture_overhead` per pair with the
  historical `on_vs_off_percent` definition, `range_over_median_percent` in the
  historical shape, and a `limitations` list that always includes the tool's own
  non-claims plus any declared by the document.

## Verification

| Check | Command | Result |
| --- | --- | --- |
| Full performance Python suite | `python3 -m unittest discover -s tools/performance -p 'test_*.py'` | 218 tests, OK |
| New tests alone | `python3 -m unittest discover -s tools/performance -p 'test_qualify_measurements.py'` | 45 tests, OK |
| Docs | `python3 tools/check_docs.py` | PASS: 160 Markdown files, 540 local links, 16 ADRs, 20 requirements |
| Qualified synthetic document | `python3 tools/performance/qualify_measurements.py --input /tmp/qualification-example.json` | exit 0, report printed |
| Historical file as a new candidate | `python3 tools/performance/qualify_measurements.py --input docs/evidence/2026-09-08-p01-overhead.json` | exit 2, `NOT QUALIFIED: input document is missing build, conditions, metrics, schema, timing_source` |

Mutation checks (source mutated, suite rerun, source restored and verified
identical to the pre-mutation copy):

| Mutation | Suite result |
| --- | --- |
| `on_vs_off_percent` denominator changed from `off` to `on` | 2 failures |
| Same-build noise pooled across both capture states | 4 failures |
| Missing metric coerced to `0.0` instead of rejected | 2 failures |

Real-evidence reproduction: the analyzer's `capture_overhead` and
`same_build_noise` reproduce all twelve recorded `on_vs_off_percent` values and
both recorded `range_over_median_percent` blocks of
`docs/evidence/2026-09-08-p01-overhead.json` to 12 decimal places.

## Insights

- The strongest available fixture was the repository's own executed evidence.
  Reproducing its recorded numbers pins the arithmetic to something that was
  actually measured, and it costs nothing to keep as a regression test.
- Qualification is mostly about what must be *present*, not what must be
  computed. The analysis is a dozen lines; the contract that makes it meaningful
  is the rest of the file.
- Absence-shaped data is the trap in this gate: an OFF run is defined by
  artifacts that do not exist. Every such absence has to be turned into an
  explicit declared field or an explicit rejection, never a default.
- Inheriting metadata is a quiet correctness bug in measurement tooling: it
  makes the mismatch check structurally incapable of failing.

## Limitations and what remains NOT RUN

- No Android device work, no APK build, no phone access. Nothing here is a
  measurement.
- E2 is **not** closed. The tool qualifies data; the qualified data does not
  exist yet for the current build.
- The tool trusts the per-run `observed`/`samples` fields as transcribed from
  verified collector artifacts. It does not re-parse raw `dumpsys` output;
  `validate_conditions.py` and `collect_wetland_pair.py` own that.
- No automated transcription from an `analyze_wetland_pairs.py` output directory
  into a qualification document. That is a plausible follow-up and was left out
  to avoid touching the collector/analyzer contracts without steering.
- Three pairs remain descriptive evidence. The tool says so in every report.

## Next actions for the lead

1. Collect alternating same-build ON/OFF trials on the reserved OnePlus 13 using
   `collect_wetland_pair.py`, per the documented contract.
2. Reduce with `analyze_wetland_pairs.py`, transcribe the per-run fields, and run
   `qualify_measurements.py`.
3. If it qualifies, record the report as evidence and update the E2 gate. If it
   does not, record the rejection reason; that is also a result.
