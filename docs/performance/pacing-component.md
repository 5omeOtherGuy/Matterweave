# Frame pacing component design (`matterweave-pacing`)

Status: implemented component, host-tested only. **No device measurement was
performed in producing this document or the crate.** Every number below is a
definition, a configuration constant or a host test result, never a device
observation.

## Context

The [roadmap](../ROADMAP.md) records that "production frame-loop scheduling
remains open" and that the unintegrated 30/60 Hz frame-cap experiment "requires
reassessment as engine pacing work". That experiment was app-level logic in the
explorer; this crate extracts the reusable, engine-level substance: a decision
component and honest frame-timing statistics that any engine frame loop (and
any future sample) can call. Integration into the explorer, the renderer or
the Android frame loop is deliberately out of scope of this component.

## Model

- Time enters only as explicit caller-supplied `u64` nanosecond parameters:
  a frame timestamp and a measured frame cost per completed frame. The core
  **never** reads a clock, sleeps, allocates after construction, spawns
  threads, logs or performs I/O. This is a hard contract, stated in the crate
  docs, and it is what makes every behaviour host-testable with synthetic
  timestamps.
- The display is characterized by one nominal refresh period
  (`refresh_period_ns`), supplied by the caller.
- State is bounded: two fixed rings of 64 `u64` samples (frame intervals and
  frame costs) plus counters. No input sequence can grow memory; overflow
  overwrites the oldest sample.

## Policy

`PacingPolicy` offers three modes; the effective target present interval is:

- `Uncapped`: the display refresh period.
- `FixedCap`: `max(configured cap interval, refresh period)` — a cap faster
  than the display is still bounded by the display.
- `Adaptive`: the smallest cadence `k ∈ 1..=4` (present every `k` refresh
  periods) whose recent median frame cost fits with headroom.

The adaptive policy decides on the **median** of the most recent 16 frame
costs, not a mean or maximum, so an isolated spike cannot move the decision
(proved by `isolated_spike_does_not_swing_decision`). Boundaries are
expressed in integer per-mille arithmetic (deterministic, no floats):

- Saturated boundary for cadence `k`: `0.95 · k · refresh`.
- Recovery boundary for returning to `k − 1`: `0.75 · (k − 1) · refresh`.

## Hysteresis mechanism

Two independent mechanisms prevent boundary oscillation:

1. **Asymmetric dead band.** Between `0.75 · (k−1) · refresh` and
   `0.95 · k · refresh` no trigger is satisfied; the cadence holds. For a
   workload near the `k ↔ k+1` boundary the recovery threshold sits far below
   the saturation threshold, so a cost that justifies stepping down does not
   come close to justifying stepping back up.
2. **Sustained-trigger counter.** A trigger must hold for 4 consecutive
   decision windows before the cadence changes; a single crossing resets.

Additionally, the adaptive policy is gated behind an 8-frame warmup so
startup samples cannot commit a premature cadence
(`startup_with_fewer_samples_than_history_capacity`).

Proof obligation: `workload_parked_near_cadence_boundary_does_not_oscillate`
drives a workload that commits to cadence 2, then parks it alternating
between 15.0 ms and 15.5 ms around the 15.2 ms cadence-1 boundary for 40
frames and asserts every decision stays at cadence 2. The test was verified
to fail (decision oscillated) when the dead band was collapsed by mutation;
it also fails for a per-frame threshold design because the median alone
smooths the alternation.

## Statistics definitions

Over the last 64 valid present-to-present intervals:

- **Median frame time**: lower median `sorted[(n−1)/2]`.
- **High-percentile frame time**: 95th percentile, `sorted[ceil(0.95·n) − 1]`.
- **Jitter**: mean absolute deviation of intervals from their median, rounded
  to the nearest nanosecond.
- **Missed display deadlines**: a completed frame whose observed interval
  exceeded the *effective target interval then in force* (the decision
  computed from frames before it) by more than `deadline_tolerance_ns`
  (default `refresh/8`). A frame is judged by the pacing it was actually
  given; adapting to a slower cadence therefore stops the counter.
- **Clock anomalies**: frames whose timestamp repeated or went backwards.
  Their cost still counts; they produce no interval, no division by zero,
  and no interval measured across the anomaly — the previous good timestamp
  is retained so the next valid interval is measured from it.

`PacingSnapshot` exposes these plus the mode, target interval, median frame
cost, counters and history occupancy as plain data with no internal
representation attached.

## Verification (host only)

`cargo test -p matterweave-pacing --locked`: 12 tests, all passing, covering
steady pacing at both cadences, the boundary hysteresis proof, spike
robustness, startup, repeated/backwards timestamps, history bounding, both
non-adaptive policies, configuration validation and cadence saturation. Full
workspace commands and outputs are recorded in the engineering log of the
implementing session; see [STATUS.md](../STATUS.md) after lead integration.

## Limitations

- **No device measurement.** All thresholds (95 %/75 %, 16-sample window,
  4-frame trigger, 8-frame warmup, 64-sample history, `refresh/8` tolerance)
  are engineering defaults justified by construction and tests, not by
  measured device behaviour. They must be revisited against real frame-cost
  and compositor data during milestone M5 pacing work.
- **Frame cost semantics are the caller's responsibility.** The policy
  assumes `frame_cost_ns` approximates the per-frame work that competes with
  the cadence (for example CPU submit plus GPU completion). A wrong estimate
  yields a wrong cadence; the crate cannot detect this.
- **No catch-up or phase alignment.** The component recommends an interval;
  it does not schedule absolute present deadlines, align to actual vblank
  phases, or implement catch-up bursts. A future engine loop owns that.
- **Single display profile.** Refresh characteristics are static per pacer;
  a display mode switch requires constructing a new pacer (history is lost)
  or an explicit future API change.
- **`MAX_CADENCE = 4`.** Slower emergency cadences (long stall handling) are
  not modeled; sustained saturation beyond cadence 4 just holds cadence 4.
- **Percentile is window-local.** With 64 samples the 95th percentile is
  effectively "third worst frame in the last ~5 seconds at 60 Hz"; longer
  horizon analysis belongs in the performance log tooling, not the pacer.
