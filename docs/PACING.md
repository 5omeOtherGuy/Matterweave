# Engine frame pacing (`matterweave-pacing`)

Reusable engine-level frame pacing component. It decides how the engine should pace
presentation and reports honest frame-timing statistics. This is engine code: it has no
dependency on rendering, physics, windowing, `winit`, Vulkan, Android or game types
(std only, no `unsafe`).

This work reassesses the unintegrated app-level frame-cap experiment (a 30/60 Hz wetland
toggle, recorded in `docs/performance/showcase/frame-cap.md` on branch
`codex/completion-frame-cap`) as reusable engine pacing work, per the roadmap. The
experiment's fixed-cap idea survives here as one policy (`Policy::Fixed`); its app wiring
is untouched and out of scope.

## Model

Time enters the core only as explicit nanosecond timestamps supplied by the caller to
`Pacer::observe` (one per presented frame). The core never reads a clock, sleeps,
allocates after construction, spawns threads, logs or performs I/O. The caller owns the
clock and any sleeping; a `0` decision interval (uncapped policy) means "present
immediately".

State is bounded: frame intervals live in a fixed ring of `HISTORY_CAPACITY` (120)
samples; old samples are overwritten in ring order. Statistics are computed over the
stored samples with these precise meanings:

- `median_ns`: lower median of the sorted samples (index `(n-1)/2`).
- `p95_ns`: nearest-rank 95th percentile (`ceil(0.95*n)`-th of the sorted samples). This
  is the robust frame cost driving the adaptive policy.
- `jitter_ns`: mean absolute deviation from the median (integer division, rounded down).
- `missed_deadlines`: count of stored intervals strictly greater than the current target
  reference (the decision interval; the refresh period for the uncapped policy).
- `anomaly_count`: timestamps discarded because they were identical to or older than the
  previous one. Discarded samples leave the clock reference unchanged.

A `Snapshot` value exposes these numbers (plus the current decision) for a HUD or
performance log without leaking internal representation. Reading it never disturbs the
pacer.

## Policy

- `Uncapped`: always recommend presenting immediately. Statistics still track the display
  refresh period as their reference.
- `Fixed { interval_ns }`: always recommend the configured interval (e.g. 33,333,333 ns
  for a 30 Hz cap).
- `Adaptive { max_divisor, settle_frames, margin_ns }`: recommend the smallest vsync
  multiple of the refresh period (up to `max_divisor`, at most `MAX_DIVISOR` = 4) whose
  interval covers the p95 frame cost. A zero `margin_ns` selects the default of one
  eighth of the refresh period.

## Hysteresis mechanism

Stepping to a *slower* cadence (larger divisor) is immediate: as soon as p95 exceeds the
current cadence interval, the pacer jumps directly to the smallest covering divisor.
Stepping to a *faster* cadence is guarded twice: p95 plus the margin must fit the
next-faster cadence interval, and that condition must hold for `settle_frames`
consecutive observations; the pacer then steps down exactly one divisor and re-confirms
before stepping further. Either guard failing resets the confirmation counter.

The asymmetry is deliberate: missing a deadline (too fast) is the visible failure, so
the pacer retreats eagerly and advances cautiously. A workload parked within the margin
of a cadence boundary therefore holds its cadence instead of oscillating frame to frame,
at the price of staying one cadence slower than strictly necessary near the boundary.
The `boundary_workload_does_not_oscillate` test proves this: it fails when the guards
are removed (verified during development) and passes with them, and it also proves the
controller is not stuck by stepping back down for a clearly cheaper workload.

## Limitations (honest)

- The pacer observes frame-boundary *intervals*, which include any caller-side sleeping
  and throttling — not pure rendering/work cost. It paces presentation; it does not
  diagnose where frame time goes.
- Cadences are integer vsync multiples only; fractional rates and variable-refresh
  displays are not modelled.
- The p95-over-120-samples cost estimate reacts over several frames, not instantly; a
  sudden sustained load steps up within a handful of frames, not the next one.
- No device measurement was performed for this component. All proof is host-level
  deterministic tests (`crates/matterweave-pacing/tests/pacing.rs`). Mobile frame timing,
  thermal behaviour and integration with the real frame loop remain open and belong to
  the lead's device testing.
