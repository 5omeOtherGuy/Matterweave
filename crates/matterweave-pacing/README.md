# matterweave-pacing

Deterministic, clock-free frame pacer: it decides a target present interval from measured
frame cost and reports honest frame-timing statistics.

## What it provides

The crate is a pure recommendation engine. The caller owns the clock, the wait and the
present. Each presented frame contributes one `FrameSample` carrying two independent
numbers:

| Field | Meaning | Drives |
| --- | --- | --- |
| `present_ns` | Monotonic frame-boundary timestamp on the caller's clock | Presentation intervals, and through them every reported statistic |
| `work_ns` | Cost of producing the frame, excluding any pacing wait the caller performed | The adaptive cadence |

`Pacer::observe` records one sample and returns a `Decision` (`interval_ns`, `divisor`,
`throttled`). `Pacer::decision` reads the current recommendation without recording.
`Pacer::snapshot` returns `Snapshot` telemetry; it is `Copy`, so reading it never disturbs
the pacer. `set_policy` and `reset` are the only state transitions, and both clear history.

Both series live in fixed rings of `HISTORY_CAPACITY` samples and are overwritten in ring
order. A `present_ns` that is identical to or older than the previous one is a clock
anomaly: the whole sample is discarded, the reference timestamp is unchanged, and
`anomaly_count` increments saturatingly.

### Cost and interval are separate, deliberately

A paced loop sleeps until its own recommendation, so the presentation interval of a healthy
frame is `max(work_ns, recommendation + overshoot)`: it contains the pacer's own output.
Deriving load from that interval closes a positive-feedback loop — every overshoot reads as
extra cost, the recommendation grows, and the cadence ratchets to its slowest setting
regardless of real work. `work_ns` is measured on the production path and excludes the wait,
so the recommendation cannot reinforce itself. `tests/closed_loop.rs` is the executable guard
and fails if the adaptive policy is reconnected to the interval series.
[Frame pacing](../../docs/performance/frame-pacing.md) holds the measured defect, the
frame-loop gate and the executed device result.

### Policies

| Policy | Recommendation |
| --- | --- |
| `Uncapped` | Present immediately (`interval_ns == 0`, `throttled == false`); statistics still use the refresh period as the missed-deadline reference. |
| `Fixed { interval_ns }` | Always recommend the configured interval, regardless of cost. |
| `Adaptive { max_divisor, settle_frames, margin_ns }` | The smallest vsync multiple of the refresh period covering the robust cost, with hysteresis. |

For `Adaptive`, the desired divisor is the least `d` in `1..=max_divisor` with
`d * period >= work_p95_ns`. Stepping to a **slower** cadence is immediate and jumps straight
to the covering divisor. Stepping to a **faster** cadence needs the robust cost plus
`margin_ns` to fit the next-faster interval for `settle_frames` consecutive observations,
then steps down exactly one divisor and re-confirms. `margin_ns == 0` resolves at construction
to one eighth of the refresh period; `DEFAULT_SETTLE_FRAMES` is 30.

`Config` validates where an invalid value enters: refresh period non-zero, fixed interval
non-zero, `max_divisor` in `1..=MAX_DIVISOR`. A built `Config` is always valid, so
`Pacer::new` cannot fail. `ConfigError` covers the three rejection cases.

## Public surface

| Item | Notes |
| --- | --- |
| `FrameSample` | `present_ns`, `work_ns`. |
| `Policy` | `Uncapped`, `Fixed`, `Adaptive`. |
| `Config`, `ConfigError` | Validated configuration; `new`, `uncapped`, `fixed`, `adaptive_default`, `adaptive`, `with_deadline_tolerance_ns`, accessors. |
| `Decision` | `interval_ns` (0 means present immediately), `divisor` (0 when not a vsync multiple), `throttled`. |
| `Snapshot` | `sample_count`, `median_ns`, `p95_ns`, `jitter_ns`, `min_ns`, `max_ns`, `latest_ns`, `work_median_ns`, `work_p95_ns`, `work_latest_ns`, `missed_deadlines`, `anomaly_count`, `target_interval_ns`, `divisor`, `throttled`. |
| `Pacer` | `new`, `config`, `set_policy`, `observe`, `decision`, `sample_count`, `is_empty`, `anomaly_count`, `median_ns`, `p95_ns`, `jitter_ns`, `work_median_ns`, `work_p95_ns`, `target_interval_ns`, `missed_deadlines`, `snapshot`, `reset`. |
| `HISTORY_CAPACITY` | 120 samples per series. |
| `MAX_DIVISOR` | 4 (the adaptive cadence ceiling). |
| `DEFAULT_SETTLE_FRAMES` | 30. |

Statistics use the lower median (`index (n-1)/2`), a nearest-rank 95th percentile
(`ceil(0.95 * n)`-th of the sorted samples) and jitter as the mean absolute deviation from
the median, rounded down. `missed_deadlines` counts stored intervals strictly greater than
the current target plus `deadline_tolerance_ns` (default one eighth of the refresh period).

## Invariants and guarantees

- Deterministic and clock-free: time enters only as explicit nanosecond values in
  `FrameSample`. The core never reads a clock, never sleeps, never spawns threads and does no
  logging or I/O.
- No allocation after construction: all state lives in fixed-capacity inline arrays on the
  caller's stack plus `Pacer`'s own arrays. State stays bounded under any input sequence.
- std only, no external dependencies, and `#![forbid(unsafe_code)]`.
- Arithmetic is integer and saturating. `Pacer::new` takes a validated `Config` and cannot
  fail.
- A policy change clears history: samples taken under a different policy are not evidence
  about the new one. `reset` additionally clears anomalies and re-arms the clock reference on
  the next `observe`, which is the lifecycle-recovery path after returning from background or
  recreating the renderer.

## Limits and what it does not do

- It recommends; it does not present. It never enforces its own interval; the wait,
  presentation and missed-deadline response are the caller's.
- Exactly one `work_ns` source is supported: frame production cost excluding the caller's
  wait. Feeding a display-paced block (for example FIFO acquire/fence stalls) into `work_ns`
  reintroduces the same feedback one layer down; the frame-loop gate subtracts measured
  blocking before feeding the pacer.
- No power or thermal input. The adaptive policy reacts to frame cost only.
- Detection latency is deliberate: the robust cost is a p95 over a 120-sample ring, so an
  isolated spike is absorbed and recovery after a load drop is bounded but not immediate.
- Integer nanoseconds only. Refresh and fixed intervals must be non-zero.
- Host-only crate evidence; real wait overshoot, display/vsync behaviour, GPU/CPU
  scheduling, thermals and surface lifecycle are not modelled here. The frame-loop
  integration and device gates are lead-owned and recorded in
  [frame pacing](../../docs/performance/frame-pacing.md).

## How it is tested

From the repository root:

```sh
cargo test -p matterweave-pacing --offline
```

- `tests/pacing.rs` (16 tests) drives hand-written sample sequences: steady cadence at high
  and low refresh, hand-computed jitter, fixed and uncapped policies, boundary hysteresis,
  isolated-spike absorption, partial history, non-monotonic timestamp rejection, bounded
  history under long runs, max-divisor saturation, configuration rejection, deadline
  tolerance and policy-change history clearing.
- `tests/closed_loop.rs` (5 tests) runs a clock-free simulated paced loop that feeds the
  pacer's own recommendation plus an overshoot back in. It is the guard for the
  cost/interval split, convergence to the definitional divisor, overshoot insensitivity,
  recovery after sustained load and boundary-crossing frames inside one p95 window.
- The crate-level example in `src/lib.rs` is one doctest.

`tests/closed_loop.rs` is deterministic: it uses no `std::time`, sleeps, threads or I/O, so
it reproduces on any host. None of these are mobile measurements; the executed host and
device gate results live in [frame pacing](../../docs/performance/frame-pacing.md).
