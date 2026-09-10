//! Closed-loop proof for the engine frame pacer.
//!
//! `tests/pacing.rs` drives the pacer with hand-written sample sequences. This suite closes
//! the loop instead. A simulated engine produces a frame costing `work_ns`, waits out the
//! pacer's *own* most recent recommendation plus a realistic overshoot, presents, and feeds
//! the resulting [`FrameSample`] back in. The simulation is clock-free: it uses no
//! `std::time`, sleeps, threads or I/O, so it is deterministic and reproducible on any host.
//!
//! The closed loop is the load-bearing part of the proof. In a paced loop the presentation
//! interval is `max(work_ns, recommendation + overshoot)`, so it contains the pacer's own
//! output. A pacer that derived load from that interval would pace itself: every overshoot
//! would read as extra cost, the recommendation would grow, and the longer interval would
//! overshoot again. `cheap_workload_does_not_ratchet_with_overshoot` is the explicit guard
//! against that; it fails if the adaptive policy reads the interval series as load.

use matterweave_pacing::{Config, FrameSample, Pacer, HISTORY_CAPACITY, MAX_DIVISOR};

/// 120 Hz display refresh period in nanoseconds (about 8.333 ms).
const HZ_120_PERIOD: u64 = 8_333_333;
/// 60 Hz display refresh period in nanoseconds (about 16.667 ms).
const HZ_60_PERIOD: u64 = 16_666_667;

/// Caller wait overshoot used by the lead's simulated ratchet measurement, in nanoseconds.
const OVERSHOOT_NS: u64 = 200_000;

/// Arbitrary monotonic start for the simulated clock.
const START_NS: u64 = 1_000_000_000;

/// Frames a paced loop may take before it must have selected the expected divisor.
/// Stepping to a slower cadence is immediate (one observation), so this bound is tiny on
/// purpose: a pacer that needs longer is not converging.
const CONVERGENCE_FRAMES: usize = 3;

/// Result of one simulated paced loop run.
struct LoopRun {
    /// Divisor returned by every `observe`, one entry per presented frame. Entry `0` is the
    /// arming observation.
    divisors: Vec<u32>,
    /// Presentation interval that preceded each frame, `0` for the first (arming) frame.
    intervals: Vec<u64>,
    /// The pacer as left by the run.
    pacer: Pacer,
}

/// Smallest vsync multiple whose interval covers `work_ns`, capped at `max_divisor`.
///
/// Derived directly from the definition — the least `d` with `d * period >= cost` — by
/// ceiling division, so it is independent of the implementation's search loop.
fn expected_divisor(period_ns: u64, work_ns: u64, max_divisor: u32) -> u32 {
    let covering = work_ns.div_ceil(period_ns).max(1);
    covering.min(u64::from(max_divisor)) as u32
}

/// Simulate a paced frame loop that feeds the pacer's own recommendation back in.
///
/// For each frame the simulated engine produces work costing `work_for_frame(i)`, waits out
/// the previous recommendation plus `overshoot_ns`, and presents. The presentation interval
/// is therefore `max(work_ns, recommendation + overshoot_ns)` — the recommendation is part
/// of the number the pacer observes, which is exactly the loop the cost/interval split must
/// survive.
fn run_paced_loop(
    config: Config,
    frames: usize,
    overshoot_ns: u64,
    start_ns: u64,
    work_for_frame: impl Fn(usize) -> u64,
) -> LoopRun {
    assert!(frames > 0, "a loop must present at least one frame");
    let mut pacer = Pacer::new(config);
    let mut now_ns = start_ns;
    let mut divisors = Vec::with_capacity(frames);
    let mut intervals = Vec::with_capacity(frames);

    // The first observation only arms the pacer's clock reference; the decision it returns
    // is the initial recommendation used for the first presented interval.
    let mut decision = pacer.observe(FrameSample {
        present_ns: now_ns,
        work_ns: work_for_frame(0),
    });
    divisors.push(decision.divisor);
    intervals.push(0);

    for frame in 1..frames {
        let work_ns = work_for_frame(frame);
        let interval_ns = work_ns.max(decision.interval_ns.saturating_add(overshoot_ns));
        now_ns = now_ns.saturating_add(interval_ns);
        decision = pacer.observe(FrameSample {
            present_ns: now_ns,
            work_ns,
        });
        divisors.push(decision.divisor);
        intervals.push(interval_ns);
    }

    LoopRun {
        divisors,
        intervals,
        pacer,
    }
}

/// 120 Hz adaptive pacer with default tuning.
fn adaptive_hz120() -> Config {
    Config::adaptive_default(HZ_120_PERIOD).expect("120 Hz is a valid refresh period")
}

/// Largest divisor the p95 cost series can contain, following the repository's nearest-rank
/// rule. The frames of a new cost phase that must accumulate before p95 can reflect it is
/// this rank, because p95 sits below the `capacity - rank` most expensive samples.
fn p95_rank(capacity: usize) -> usize {
    (95 * capacity).div_ceil(100)
}

#[test]
fn paced_loop_converges_to_the_expected_divisor() {
    // A representative matrix: cheap, intermediate and maximum-cadence workloads on a fast
    // and a slower display. The expected divisor comes from the definition, not from an
    // observed run.
    let cases = [
        ("cheap 4 ms on 120 Hz", HZ_120_PERIOD, 4_000_000_u64),
        (
            "intermediate 12 ms on 120 Hz",
            HZ_120_PERIOD,
            12_000_000_u64,
        ),
        ("maximum 40 ms on 120 Hz", HZ_120_PERIOD, 40_000_000_u64),
        ("cheap 8 ms on 60 Hz", HZ_60_PERIOD, 8_000_000_u64),
        ("intermediate 25 ms on 60 Hz", HZ_60_PERIOD, 25_000_000_u64),
        ("maximum 80 ms on 60 Hz", HZ_60_PERIOD, 80_000_000_u64),
    ];

    for (label, period_ns, work_ns) in cases {
        let expected = expected_divisor(period_ns, work_ns, MAX_DIVISOR);
        let run = run_paced_loop(
            Config::adaptive_default(period_ns).expect("valid refresh period"),
            400,
            OVERSHOOT_NS,
            START_NS,
            |_| work_ns,
        );

        let converged = run.divisors.iter().position(|d| *d == expected);
        let converged = match converged {
            Some(frame) => frame,
            None => panic!(
                "{label}: paced loop never selected the expected divisor {expected}x; \
                 observed divisors {:?}",
                run.divisors
            ),
        };
        assert!(
            converged <= CONVERGENCE_FRAMES,
            "{label}: paced loop took {converged} frames to select {expected}x, \
             expected within {CONVERGENCE_FRAMES}"
        );
        assert!(
            run.divisors[converged..].iter().all(|d| *d == expected),
            "{label}: cadence must hold {expected}x after converging; observed {:?}",
            run.divisors
        );
        assert_eq!(
            run.pacer.decision().divisor,
            expected,
            "{label}: final recommendation must be the converged cadence"
        );
    }
}

#[test]
fn cheap_workload_does_not_ratchet_with_overshoot() {
    // The historical failure: a paced loop with overshoot ratcheted a cheap workload to the
    // slowest cadence within three frames and stayed there. Run the same cheap workload twice
    // — once with a realistic overshoot and once with none — and require the closed loop to
    // reach the same cadence. If the adaptive policy ever reads the presentation interval as
    // load, the overshooting run ratchets and this fails.
    let work_ns = 5_000_000_u64;
    let busy = run_paced_loop(adaptive_hz120(), 600, OVERSHOOT_NS, START_NS, |_| work_ns);
    let reference = run_paced_loop(adaptive_hz120(), 600, 0, START_NS, |_| work_ns);

    assert!(
        busy.divisors.iter().take(3).all(|d| *d == 1),
        "a 5 ms workload must not slow down in the first frames; observed {:?}",
        &busy.divisors[..3]
    );
    assert!(
        busy.divisors.iter().all(|d| *d < MAX_DIVISOR),
        "wait overshoot on a cheap workload must never reach the {MAX_DIVISOR}x maximum; \
         observed divisors {:?}",
        busy.divisors
    );
    assert!(
        busy.divisors.iter().all(|d| *d == 1),
        "a 5 ms workload fits every 120 Hz refresh, so the paced loop must hold 1x; \
         observed divisors {:?}",
        busy.divisors
    );
    assert_eq!(
        busy.pacer.decision().divisor,
        reference.pacer.decision().divisor,
        "the overshooting loop must select the same cadence as the zero-overshoot loop: \
         the cost series, not the paced interval, decides"
    );
    assert_eq!(
        reference.pacer.decision().divisor,
        1,
        "the zero-overshoot reference for a cheap workload is every-refresh"
    );

    // Confirm the loop actually exercised the defect: the overshooting run presented on
    // intervals strictly longer than the recommendation would have been without it.
    assert!(
        busy.intervals.iter().skip(1).all(|i| *i > HZ_120_PERIOD),
        "the overshooting run must actually present later than the refresh period"
    );
    assert!(
        reference
            .intervals
            .iter()
            .skip(1)
            .all(|i| *i == HZ_120_PERIOD),
        "the zero-overshoot reference tracks the recommendation exactly"
    );
}

#[test]
fn selected_cadence_is_insensitive_to_overshoot_but_tracks_real_cost() {
    let overshoots = [0_u64, 100_000, 1_000_000];
    for overshoot_ns in overshoots {
        // A cheap workload fits every 120 Hz refresh no matter how long the caller's wait
        // overshoots.
        let cheap = run_paced_loop(adaptive_hz120(), 300, overshoot_ns, START_NS, |_| 4_000_000);
        assert!(
            cheap.divisors.iter().all(|d| *d == 1),
            "4 ms must hold 1x at {overshoot_ns} ns overshoot; observed {:?}",
            cheap.divisors
        );

        // A genuine cost increase is a different signal and must move the cadence.
        let heavy = run_paced_loop(adaptive_hz120(), 300, overshoot_ns, START_NS, |_| {
            12_000_000
        });
        assert_eq!(
            heavy.pacer.decision().divisor,
            2,
            "12 ms needs every second 120 Hz refresh at {overshoot_ns} ns overshoot"
        );
    }
}

#[test]
fn cadence_recovers_after_sustained_load_clears() {
    let settle_frames: u32 = 10;
    let config =
        Config::adaptive(HZ_120_PERIOD, MAX_DIVISOR, settle_frames, 0).expect("valid config");
    let heavy_frames: usize = 200;
    let cheap_frames: usize = 200;
    let run = run_paced_loop(
        config,
        heavy_frames + cheap_frames,
        OVERSHOOT_NS,
        START_NS,
        |frame| {
            if frame < heavy_frames {
                10_000_000
            } else {
                3_000_000
            }
        },
    );

    assert_eq!(
        run.divisors[heavy_frames - 1],
        2,
        "sustained 10 ms load on a 120 Hz display must step up to every second refresh"
    );

    let step_down = run.divisors[heavy_frames..]
        .iter()
        .position(|d| *d == 1)
        .expect("the cadence must recover once the load clears");
    // The bounded budget: p95 can only fall below the boundary after `rank` cheap samples
    // have displaced the expensive tail of the ring, then `settle_frames` agreeing
    // observations must accumulate. The `+ 4` is slack for the off-by-one position.
    let bound = p95_rank(HISTORY_CAPACITY) + settle_frames as usize + 4;
    assert!(
        step_down <= bound,
        "recovery took {step_down} cheap frames, exceeding the asserted bound {bound}"
    );
    assert!(
        run.divisors[heavy_frames + step_down..]
            .iter()
            .all(|d| *d == 1),
        "the cadence must stay recovered; observed {:?}",
        &run.divisors[heavy_frames + step_down..]
    );
    assert_eq!(run.pacer.decision().divisor, 1);

    // Make the settle window observable. The old `step_down >= settle_frames` assertion
    // was vacuous: nearest-rank p95 over the 120-sample ring cannot fall to the cheap
    // value until 114 cheap samples have displaced the expensive tail, so `step_down` was
    // always at least 114, and `114 >= 10` held even with the settle window deleted.
    // Everything before the settle wait is identical in both runs, so two configurations
    // differing only in `settle_frames` must differ in the step-down frame by exactly the
    // settle difference.
    let longer = recovery_step_down_frames(settle_frames + 30);
    assert_eq!(
        longer - step_down,
        30,
        "a {}-frame settle window must delay step-down by exactly 30 frames over the \
         {settle_frames}-frame window; observed step-downs {step_down} and {longer}",
        settle_frames + 30,
    );
}

/// Frames of the cheap phase that elapse before the cadence reaches 1x on the same
/// 10 ms -> 3 ms recovery as [`cadence_recovers_after_sustained_load_clears`], with the
/// given settle window. Only the settle window differs between calls.
fn recovery_step_down_frames(settle_frames: u32) -> usize {
    let config =
        Config::adaptive(HZ_120_PERIOD, MAX_DIVISOR, settle_frames, 0).expect("valid config");
    let heavy_frames: usize = 200;
    let cheap_frames: usize = 200;
    let run = run_paced_loop(
        config,
        heavy_frames + cheap_frames,
        OVERSHOOT_NS,
        START_NS,
        |frame| {
            if frame < heavy_frames {
                10_000_000
            } else {
                3_000_000
            }
        },
    );
    assert_eq!(run.divisors[heavy_frames - 1], 2);
    run.divisors[heavy_frames..]
        .iter()
        .position(|d| *d == 1)
        .expect("the cadence must recover once the load clears")
}

#[test]
fn workload_alternating_within_one_cadence_never_changes_cadence() {
    // One frame in 39 costs 9.5 ms, which does not fit a single 120 Hz refresh
    // (8.333 ms), while the frames between it cost 3 ms. Four boundary-crossing frames
    // can appear in a 120-sample window, fewer than the six largest samples nearest-rank
    // p95 excludes, so the robust cost stays at 3 ms on the cheap side of the boundary.
    // A controller that reacts to the latest frame's cost — or to any other single-sample
    // statistic — steps to 2x on every 9.5 ms spike; the real p95-based pacer must not.
    let work_for_frame = |frame: usize| {
        if frame > 0 && frame.is_multiple_of(39) {
            9_500_000
        } else {
            3_000_000
        }
    };
    let crossings = (1..400)
        .filter(|frame| work_for_frame(*frame) > HZ_120_PERIOD)
        .count();
    assert!(
        crossings > 0,
        "the workload must contain frames that cross the cadence boundary"
    );
    let run = run_paced_loop(
        adaptive_hz120(),
        400,
        OVERSHOOT_NS,
        START_NS,
        work_for_frame,
    );
    assert_eq!(
        run.pacer.work_p95_ns(),
        3_000_000,
        "the robust cost must stay on the cheap side of the boundary while individual \
         frames cross it"
    );
    assert!(
        run.divisors.iter().all(|d| *d == 1),
        "boundary-crossing frames inside one p95 window must never change the cadence; \
         observed divisors {:?}",
        run.divisors
    );
    assert_eq!(run.pacer.decision().divisor, 1);
}
