# Engine frame pacing

`crates/matterweave-pacing` is the reusable engine component that decides a target present
interval (vsync cadence) from observed frame cost and reports honest frame-timing statistics.
It is a pure recommendation engine: the caller owns the clock, the wait and the present. The
core is deterministic and clock-free. Time enters only as explicit nanosecond values, it
never reads a clock, sleeps, allocates after construction, spawns threads or performs I/O,
and it uses no `unsafe` code. That contract is what makes the behaviour below reproducible in
a host test without a display. Source: [crate source](../../crates/matterweave-pacing/src/lib.rs).

## Model

Each presented frame contributes exactly one `FrameSample` carrying two independent numbers:

| Field | Meaning | Drives |
| --- | --- | --- |
| `present_ns` | Monotonic frame-boundary timestamp on the caller's clock | Presentation intervals, and through them every reported statistic |
| `work_ns` | Cost of producing the frame, **excluding** any pacing wait the caller performed | The adaptive cadence |

The core keeps two fixed rings of `HISTORY_CAPACITY` (120) samples, one per series, overwritten
in ring order. Both series share one sample count, so state is bounded under any input sequence.
`present_ns` values that are identical to or older than the previous one are clock anomalies:
the whole sample is discarded, the reference timestamp is left unchanged, and `anomaly_count`
increments saturatingly.

The two series are summarised separately, and the distinction is deliberate, not cosmetic:

- **Presentation intervals** (`present_ns[i] - present_ns[i-1]`) describe what the display
  actually showed:
  - `median_ns` — lower median of the sorted intervals (`index (n-1)/2`).
  - `p95_ns` — nearest-rank 95th percentile (`ceil(0.95 * n)`-th of the sorted intervals).
  - `jitter_ns` — mean absolute deviation from the median, rounded down.
  - `min_ns`, `max_ns`, `latest_ns` — smallest, largest and most recent interval.
  - `missed_deadlines` — stored intervals strictly greater than the current target plus
    `deadline_tolerance_ns` (default: one eighth of the refresh period).
- **Frame production cost** describes the load:
  - `work_median_ns`, `work_latest_ns` — same median rule, most recent value.
  - `work_p95_ns` — same nearest-rank rule. This is the robust cost, and the **only** input
    to the adaptive cadence.

`target_interval_ns`, `divisor` and `throttled` report the current recommendation. `Snapshot`
is `Copy`; reading it never disturbs the pacer.

## Why cost and interval are separate

A paced loop presents on the interval its own controller recommended, plus however much the
caller's wait overshot. The presentation interval of a healthy frame is therefore
`max(work_ns, recommendation + overshoot)`: it contains the controller's output. Deriving load
from that number closes a positive-feedback loop. Each overshoot reads as extra cost, the
recommendation grows, the longer interval overshoots again, and the cadence ratchets upward
regardless of real work.

The lead measured exactly that defect in the two earlier candidates, which both decided
cadence from the presentation interval. In a simulated loop with a 200 microsecond wait
overshoot, the cadence ratcheted to the slowest setting within three frames and stayed there:
a 5 ms workload on a 120 Hz display was paced down to 30 Hz. With zero overshoot the same
loop held the correct cadence, which isolates the feedback loop as the cause. That is the
lead's measurement, not this component's, and it is recorded in the commit that introduced
the cost/interval split.

`work_ns` is measured by the caller on the production path and excludes the wait, so the
pacer's own output cannot appear in it. The decision therefore cannot reinforce itself. The
[closed-loop suite](../../crates/matterweave-pacing/tests/closed_loop.rs) makes that an
executable guard: it drives a clock-free paced loop, and it fails if the adaptive policy is
reconnected to the interval series.

## Policies

- `Uncapped` — present immediately (`interval_ns == 0`, `throttled == false`). Statistics are
  still measured against the refresh period.
- `Fixed { interval_ns }` — always recommend the configured interval, regardless of cost.
- `Adaptive { max_divisor, settle_frames, margin_ns }` — recommend the smallest vsync multiple
  of the refresh period covering the robust cost, with hysteresis.

For the adaptive policy the desired divisor is the least `d` in `1..=max_divisor` with
`d * period >= work_p95_ns`, so it never recommends more refreshes than the robust cost
requires and never fewer than one.

### Hysteresis

- Stepping to a **slower** cadence is immediate. As soon as the robust cost exceeds the
  current interval, the pacer jumps straight to the divisor that covers it.
- Stepping to a **faster** cadence is guarded twice. The robust cost plus `margin_ns` must fit
  the next-faster interval, and that condition must hold for `settle_frames` consecutive
  observations. It then steps down exactly one divisor and re-confirms before stepping
  further. Either guard failing resets the confirmation counter.

`margin_ns` defaults to one eighth of the refresh period and `settle_frames` to 30. A workload
parked within the margin of a cadence boundary therefore holds its cadence instead of
oscillating.

The asymmetry is the right shape for a frame scheduler because the two directions have
asymmetric costs. Choosing a slower cadence is a cheap, self-correcting bet: it costs a little
smoothness, and a frame that genuinely misses its deadline shows up as visible jank, so there
is no reason to delay the reaction to real load. Choosing a faster cadence on a noisy sample
is the failure mode: too-eager steps up and down produce oscillating cadence and repeated
deadline misses. Requiring a sustained, margin-guarded signal to speed up trades a little
latency for stability. Retreat fast, advance slowly.

## Limitations

- **Host-only evidence.** The behaviour below is proven by deterministic host tests. No
  device or emulator measurement was performed by the pacing work; real wait overshoot,
  display/vsync behaviour, GPU and CPU scheduling, thermals and surface lifecycle are not
  modelled. Frame-loop integration and the device gates are lead-owned.
- **The model simplifies the caller's wait.** Tests use a constant overshoot. A real caller's
  wait is jittery and occasionally much larger, and a single large overrun is not a cadence
  signal.
- **The pacer recommends; it does not present.** It never sleeps and never enforces its own
  interval. Enforcement, presentation and any missed-deadline response are the caller's.
- **Detection latency is deliberate.** The robust cost is a p95 over a 120-sample ring. An
  isolated spike is absorbed by design, and a genuine load *drop* only moves the statistic
  after enough cheaper samples have displaced the expensive tail of the ring; recovery after a
  load drop is correspondingly bounded but not immediate.
- **No power or thermal input.** The adaptive policy reacts to frame cost only. There is no
  energy-aware or thermal-aware policy in this component.
- **Integer nanoseconds.** All arithmetic is integer and saturating; there is no sub-nanosecond
  resolution, and configurations must use non-zero refresh and fixed intervals.
- **Policy changes clear history.** Samples taken under one policy are not evidence about
  another, so `set_policy` resets the rings and the adaptive state.

## Verification

All checks below were run on this host by the pacing work. None is a device measurement.

- `cargo test -p matterweave-pacing --offline` with
  `CARGO_TARGET_DIR=/mnt/bench/matterweave-dev/targets/pacing-deepseek`: exit 0,
  `5 passed; 0 failed` for the [closed-loop suite](../../crates/matterweave-pacing/tests/closed_loop.rs),
  `15 passed; 0 failed` for the [unit-level suite](../../crates/matterweave-pacing/tests/pacing.rs),
  and `1 passed; 0 failed` doctest.
- `cargo fmt --all -- --check`: exit 0, no output.
- `cargo clippy -p matterweave-pacing --all-targets --offline -- -D warnings`: exit 0, zero
  warnings.
- `python3 tools/check_docs.py`: exit 0.
- **Mutation check.** Temporarily reconnecting the adaptive policy to the presentation
  interval (`self.p95_ns()` in place of `self.work_p95_ns()` in `update_adaptive`) made all
  five closed-loop tests fail, with the ratchet guard observing divisors
  `[1, 2, 3, 4, 4, ...]` on a 5 ms workload. The file was restored exactly
  (`git diff --stat` empty) and the suite re-ran green.

The [performance campaign README](README.md) and [development guide](../DEVELOPMENT.md) hold
the lead-owned execution record and build/test instructions.
