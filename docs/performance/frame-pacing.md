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
| `work_ns` | Cost of producing the frame, **excluding** pacing waits: the caller's own wait and any display-paced blocking the caller measured | The adaptive cadence |

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

### Display-paced blocking is not frame cost

The caller's own pacing wait is not the only wait that can hide inside `work_ns`. A renderer that
presents with FIFO (hard vsync) can block in `vkAcquireNextImageKHR` and on the submission fence
until the display's own cadence allows the next frame. That time is paced by the display, not by
the engine's work, but it sits in the production path; feeding it to the pacer as load is the same
feedback the cost/interval split exists to avoid, one layer down.

The native gate (`--pacing-check`) enables the renderer's draw diagnostics, sums the measured
upload fence waits, submission fence wait and swapchain acquire time, and feeds the pacer a
`work_ns` with that sum subtracted saturating at zero. `present_ms` is excluded because it times
queueing the present request, not waiting on scanout. Every phase line reports both the raw
measured mean frame cost and the blocked share (`work_raw_mean_ms`, `blocked_mean_ms`,
`blocked_share_pct`), and counts frames whose blocking the renderer did not measure
(`blocked_unavailable_frames`) instead of substituting a zero silently.

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

## Frame-loop gate

The `--pacing-check` gate drives this component against a real window, present and display
refresh, with a per-phase synthetic frame cost. Three properties of the gate matter when reading
its report:

- **Per-phase cadence evidence.** Every phase line reports `cadence_changes`, `divisor_min` and
  `divisor_max` over the phase's frames, not only the final `divisor`. The parked phase also
  requires zero cadence changes, so a cadence that oscillated and happened to end on 2x no longer
  passes, and its PASS text claims only what was checked.
- **Blocked-time correction.** `work_ns` is the measured frame cost minus the display-paced
  blocking the renderer measured (see above), and the line reports the raw mean and the blocked
  share so the size of the correction is visible.
- **Lifecycle interruption.** A suspend rebuilds the pacer with `Pacer::new`, whose history is
  empty and whose divisor starts at 1, while the gate's phase state survives. The gate therefore
  ends the run `INCONCLUSIVE` and names the interruption rather than judging later phases against
  state the suspend destroyed.

## Limitations

- **Host-only evidence.** The behaviour below is proven by deterministic host tests. No
  device or emulator measurement was performed by the pacing work; real wait overshoot,
  display/vsync behaviour, GPU and CPU scheduling, thermals and surface lifecycle are not
  modelled. Frame-loop integration and the device gates are lead-owned.
- **The blocked-time correction is unverified on a device.** The gate subtracts measured
  display-paced blocking from the cost it feeds the pacer, but no run in this record shows whether
  that correction changes a cadence decision on a real FIFO panel. Only the host-reported blocked
  share exists so far.
- **The gate samples the display period periodically.** The frame path rechecks the monitor's
  reported refresh period about once a second and re-arms pacing on a change larger than one
  percent; a genuine variable-refresh change is handled, but a change during the first second
  after a resize or a monitor move is not.
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

All checks below were re-run on this host when the review findings described above were applied.
None is a device measurement.

- `cargo test -p matterweave-pacing --offline`: exit 0; `5 passed; 0 failed` for the
  [closed-loop suite](../../crates/matterweave-pacing/tests/closed_loop.rs), `16 passed; 0 failed`
  for the [unit-level suite](../../crates/matterweave-pacing/tests/pacing.rs), and `1 passed;
  0 failed` doctest.
- `cargo test -p matterweave-explorer --offline`: exit 0; `89 passed; 0 failed; 1 ignored`. The
  ignored test is the opt-in full-map wetland integration check.
- `cargo fmt --all -- --check`: exit 0, no output.
- `cargo clippy -p matterweave-pacing -p matterweave-explorer --all-targets --offline -- -D
  warnings`: exit 0. The only warning emitted is a pre-existing rustc lint in vendored `winit`
  (`function_casts_as_integer`), which belongs to neither selected package.
- `python3 tools/check_docs.py`: exit 0 (`PASS: 156 Markdown files, 350 local links, 16 ADRs and
  20 requirements`).
- **Mutation checks.** Each new or changed assertion was shown to fail by breaking the behaviour it
  names, then restored exactly (`git diff --stat -- crates/matterweave-pacing/src/lib.rs` empty)
  with the suites re-running green:
  - **Settle window.** Replacing `if self.stable >= settle_frames.max(1)` with
    `if self.stable >= 1` made the recovery test fail with step-downs `113` and `113` for the
    10- and 40-frame settle windows instead of a difference of 30. That the step-down was already
    `113` with no settle window is also why the removed `step_down >= settle_frames` bound was
    vacuous.
  - **Jitter.** Stubbing the deviation sum to zero (`.map(|sample| ...)` ->
    `.map(|_| 0_u128)`) failed the new test with `left: 0, right: 633333`.
  - **Robust cost.** Replacing the robust cost with the latest frame (`self.work_p95_ns()` ->
    `self.latest_work_ns`) made the alternating-workload test fail: the observed divisor series
    stepped to 2 on every boundary-crossing frame and back to 1 after the settle window.
  - **Cost/interval split** (re-run of the earlier check). Reconnecting the adaptive policy to the
    presentation interval (`self.p95_ns()` in place of `self.work_p95_ns()`) failed all five
    closed-loop tests; the ratchet guard observed `[1, 2, 3]` for a 5 ms workload in the first
    three frames, and the series ratcheted to `[1, 2, 3, 4, 4, ...]`.

## Device gate — OnePlus 13, executed

Lead-owned and actually run on 2026-09-10. Device: OnePlus 13 (`CPH2653`), Android 16,
Adreno 830, Vulkan 1.3.284, driver 2150760522, presenting FIFO. APK
`f7523ed8a9e2e9791ba63c226a3c1e6445e665d28c5908f8315aeb72a501d19d`, source `c37f2e3`, 16 KiB
ELF alignment and v2 signature verified before installation. The display reported a 16.667 ms
period (the panel also advertises 90 and 120 Hz modes). Full report:
[raw device report](../evidence/2026-09-10-pacing-oneplus13.txt).

| Phase | work p95 | raw mean | blocked mean | blocked share | interval p50 | jitter | missed | cadence | changes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| baseline | 4.108 ms | 4.321 ms | 0.098 ms | 2.3% | 16.688 ms | 0.004 ms | 0/120 | 1x | 0 |
| loaded | 24.987 ms | 25.000 ms | 0.023 ms | 0.1% | 33.341 ms | 0.009 ms | 0/120 | 2x | 1 |
| parked | 15.803 ms | 15.834 ms | 0.057 ms | 0.4% | 33.355 ms | 0.004 ms | 0/120 | 2x | 0 |
| recovered | 4.108 ms | 4.168 ms | 0.066 ms | 1.6% | 16.688 ms | 0.002 ms | 0/120 | 1x | 1 |

All four phases PASS, each judged against the cost the device measured. The `parked` phase
records **zero cadence changes across its 240 frames**, so the hysteresis claim rests on the
whole phase rather than its last frame: a cost of 15.803 ms, inside the 2.083 ms guard band
below the 16.667 ms boundary, held 2x throughout. `loaded` and `recovered` each record exactly
one change, the single step up and the single step down.

**How much the FIFO blocking mattered, measured.** The blocked share of measured frame cost —
upload and render fence waits plus `vkAcquireNextImageKHR` — was 2.3% at worst and 0.1% under
load, with no frame left unmeasured. The concern that vsync blocking would contaminate the load
signal is therefore real in principle and small on this device under this pacing; the gate
subtracts it regardless, and the raw and corrected costs are both reported so the difference
stays visible rather than assumed.

An earlier run of the same gate, before these fixes, was interrupted by a real device suspend:
it truncated one phase to 72 of 120 samples and would have judged a later phase against cadence
the suspend had destroyed. That is why the gate now ends INCONCLUSIVE on an interruption. This
run was executed with the display held awake and completed all four phases uninterrupted.

**What this does not establish.** It is a scheduling-correctness result on one device, one
scene and a synthetic frame cost. No throughput, power, thermal or battery claim follows from
it, no comparison against the previous fixed-cadence loop is made, and the 90/120 Hz panel
modes and the re-arm path for a mid-run refresh change were not exercised.

The [performance campaign README](README.md) and [development guide](../DEVELOPMENT.md) hold
the lead-owned execution record and build/test instructions.
