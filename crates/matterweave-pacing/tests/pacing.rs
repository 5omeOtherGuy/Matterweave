//! Host-level proof for the engine frame pacer. All time enters as explicit nanosecond
//! values; no clock, sleep, thread or I/O is involved.

use matterweave_pacing::{
    Config, ConfigError, Decision, FrameSample, Pacer, Policy, HISTORY_CAPACITY, MAX_DIVISOR,
};

const HZ_120_PERIOD: u64 = 8_333_333;
const HZ_60_PERIOD: u64 = 16_666_667;
const CAP_30HZ: u64 = 33_333_333;

/// Drive a pacer with explicit `(presentation interval, frame work cost)` pairs starting at
/// `start`, returning every decision.
fn drive(pacer: &mut Pacer, start: u64, samples: &[(u64, u64)]) -> Vec<Decision> {
    let mut now = start;
    pacer.observe(FrameSample {
        present_ns: now,
        work_ns: samples.first().map_or(0, |(_, work)| *work),
    });
    samples
        .iter()
        .map(|(interval, work)| {
            now += interval;
            pacer.observe(FrameSample {
                present_ns: now,
                work_ns: *work,
            })
        })
        .collect()
}

/// Drive `count` identical frames.
fn drive_steady(
    pacer: &mut Pacer,
    start: u64,
    count: usize,
    interval_ns: u64,
    work_ns: u64,
) -> Vec<Decision> {
    drive(pacer, start, &vec![(interval_ns, work_ns); count])
}

#[test]
fn steady_pacing_high_refresh() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    let decisions = drive_steady(&mut pacer, 1_000_000_000, 150, HZ_120_PERIOD, 4_000_000);
    let last = decisions.last().copied().expect("decisions recorded");
    assert_eq!(
        last.divisor, 1,
        "a 4 ms workload holds every-refresh cadence on a 120 Hz display"
    );
    assert_eq!(last.interval_ns, HZ_120_PERIOD);
    assert!(last.throttled);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.sample_count, HISTORY_CAPACITY);
    assert_eq!(snapshot.median_ns, HZ_120_PERIOD);
    assert_eq!(snapshot.p95_ns, HZ_120_PERIOD);
    assert_eq!(snapshot.jitter_ns, 0);
    assert_eq!(snapshot.work_median_ns, 4_000_000);
    assert_eq!(snapshot.work_p95_ns, 4_000_000);
    assert_eq!(snapshot.missed_deadlines, 0);
    assert_eq!(snapshot.anomaly_count, 0);
}

#[test]
fn steady_pacing_low_cadence() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_60_PERIOD).expect("valid config"));
    // A 30 ms frame cost cannot fit one 60 Hz refresh, so the pacer settles on every second.
    let decisions = drive_steady(&mut pacer, 1_000_000_000, 150, CAP_30HZ, 30_000_000);
    let last = decisions.last().copied().expect("decisions recorded");
    assert_eq!(last.divisor, 2);
    assert_eq!(last.interval_ns, 2 * HZ_60_PERIOD);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.median_ns, CAP_30HZ);
    assert_eq!(snapshot.p95_ns, CAP_30HZ);
    assert_eq!(snapshot.jitter_ns, 0);
    assert_eq!(snapshot.work_p95_ns, 30_000_000);
}

#[test]
fn jitter_is_mean_absolute_deviation_from_the_lower_median() {
    // Every other test uses a uniform interval sequence, where `jitter_ns == 0` is true
    // for any pacer, including a stub that always reports zero. This drives deliberately
    // non-uniform intervals and pins the documented definition instead.
    //
    // Recorded intervals, in order: 8_900_000, 10_000_000, 8_000_000, 9_100_000,
    // 9_900_000, 8_300_000 ns. Sorted: 8_000_000, 8_300_000, 8_900_000, 9_100_000,
    // 9_900_000, 10_000_000. n = 6, so the lower median is index (6 - 1) / 2 = 2 ->
    // 8_900_000 ns. Absolute deviations from it: 900_000, 600_000, 0, 200_000, 1_000_000,
    // 1_100_000. Sum = 3_800_000; mean = 3_800_000 / 6 = 633_333.33..., rounded down
    // (integer division) -> 633_333 ns.
    let intervals = [
        8_900_000_u64,
        10_000_000,
        8_000_000,
        9_100_000,
        9_900_000,
        8_300_000,
    ];
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    let samples: Vec<(u64, u64)> = intervals.iter().map(|i| (*i, 4_000_000)).collect();
    drive(&mut pacer, 1_000_000_000, &samples);
    assert_eq!(pacer.sample_count(), intervals.len());
    assert_eq!(
        pacer.median_ns(),
        8_900_000,
        "the lower median is index (n - 1) / 2 of the sorted intervals"
    );
    assert_eq!(
        pacer.jitter_ns(),
        633_333,
        "hand-computed mean absolute deviation from the median, rounded down"
    );
    assert_eq!(pacer.snapshot().jitter_ns, 633_333);
}

#[test]
fn fixed_cap_recommends_configured_interval() {
    let mut pacer = Pacer::new(Config::fixed(HZ_120_PERIOD, CAP_30HZ).expect("valid config"));
    let decisions = drive_steady(&mut pacer, 1_000_000_000, 40, CAP_30HZ, 4_000_000);
    for decision in &decisions {
        assert_eq!(decision.interval_ns, CAP_30HZ);
        assert_eq!(decision.divisor, 0);
        assert!(decision.throttled);
    }
    assert_eq!(pacer.snapshot().target_interval_ns, CAP_30HZ);
}

#[test]
fn uncapped_policy_presents_immediately_but_still_reports_refresh() {
    let mut pacer = Pacer::new(Config::uncapped(HZ_120_PERIOD).expect("valid config"));
    let decisions = drive_steady(&mut pacer, 1_000_000_000, 40, 9_000_000, 9_000_000);
    for decision in &decisions {
        assert_eq!(decision.interval_ns, 0);
        assert_eq!(decision.divisor, 0);
        assert!(
            !decision.throttled,
            "uncapped never asks the caller to wait"
        );
    }
    let snapshot = pacer.snapshot();
    assert_eq!(
        snapshot.target_interval_ns, HZ_120_PERIOD,
        "uncapped statistics still measure against the display"
    );
    assert_eq!(
        snapshot.missed_deadlines, 0,
        "9 ms is within the 8.333 ms refresh plus its default tolerance"
    );
}

#[test]
fn boundary_workload_does_not_oscillate() {
    let margin = 1_000_000;
    let mut pacer = Pacer::new(Config::adaptive(HZ_120_PERIOD, 4, 10, margin).expect("valid"));
    // Phase A: comfortably inside every-refresh cadence.
    for decision in drive_steady(&mut pacer, 1_000_000_000, 60, HZ_120_PERIOD, 8_000_000) {
        assert_eq!(decision.divisor, 1);
    }
    // Phase B: sustained cost above the 1x boundary forces an immediate step up.
    let decisions = drive_steady(&mut pacer, 2_000_000_000, 60, 2 * HZ_120_PERIOD, 8_600_000);
    assert_eq!(
        decisions.last().expect("decisions recorded").divisor,
        2,
        "sustained over-boundary cost must step up"
    );
    // Phase C: park just below the boundary but inside the margin. The pacer must hold
    // cadence 2x for the whole phase. A controller without hysteresis would drop to 1x
    // here because the parked cost fits the 1x interval.
    let parked = drive_steady(&mut pacer, 3_000_000_000, 200, 2 * HZ_120_PERIOD, 8_250_000);
    assert!(
        parked.iter().all(|decision| decision.divisor == 2),
        "parked near-boundary workload must not oscillate"
    );
    // Phase D: a workload clearly below the boundary (outside the margin) must eventually
    // step back down, proving hysteresis is not a stuck controller.
    let recovery = drive_steady(&mut pacer, 9_000_000_000, 200, HZ_120_PERIOD, 6_000_000);
    assert_eq!(
        recovery.last().expect("decisions recorded").divisor,
        1,
        "clearly cheaper workload must step back down"
    );
}

#[test]
fn isolated_spike_does_not_swing_decision() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    let mut now = 1_000_000_000;
    pacer.observe(FrameSample {
        present_ns: now,
        work_ns: 4_000_000,
    });
    let mut post_warmup = Vec::new();
    for i in 0..170 {
        let (interval, work) = if i == 130 {
            (60_000_000, 55_000_000)
        } else {
            (HZ_120_PERIOD, 4_000_000)
        };
        now += interval;
        let decision = pacer.observe(FrameSample {
            present_ns: now,
            work_ns: work,
        });
        if i >= HISTORY_CAPACITY - 1 {
            post_warmup.push(decision);
        }
    }
    assert!(
        post_warmup.iter().all(|decision| decision.divisor == 1),
        "a single outlier must not swing the cadence"
    );
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.median_ns, HZ_120_PERIOD);
    assert_eq!(snapshot.work_median_ns, 4_000_000);
    assert_eq!(
        snapshot.missed_deadlines, 1,
        "the spike itself is still reported honestly"
    );
}

#[test]
fn startup_with_partial_history() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_60_PERIOD).expect("valid config"));
    assert!(pacer.is_empty());
    assert_eq!(pacer.sample_count(), 0);
    let decisions = drive_steady(&mut pacer, 500_000_000, 5, HZ_60_PERIOD, 5_000_000);
    assert_eq!(pacer.sample_count(), 5);
    assert!(!pacer.is_empty());
    assert_eq!(pacer.median_ns(), HZ_60_PERIOD);
    assert_eq!(pacer.p95_ns(), HZ_60_PERIOD);
    assert_eq!(pacer.jitter_ns(), 0);
    assert_eq!(pacer.work_median_ns(), 5_000_000);
    assert_eq!(pacer.work_p95_ns(), 5_000_000);
    assert_eq!(pacer.missed_deadlines(), 0);
    assert_eq!(
        decisions
            .last()
            .copied()
            .expect("decisions recorded")
            .divisor,
        1
    );
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.sample_count, 5);
    assert_eq!(snapshot.min_ns, HZ_60_PERIOD);
    assert_eq!(snapshot.max_ns, HZ_60_PERIOD);
    assert_eq!(snapshot.latest_ns, HZ_60_PERIOD);
    assert_eq!(snapshot.work_latest_ns, 5_000_000);
    assert_eq!(snapshot.target_interval_ns, HZ_60_PERIOD);
    pacer.reset();
    assert!(pacer.is_empty());
    assert_eq!(pacer.snapshot().sample_count, 0);
    assert_eq!(pacer.snapshot().work_p95_ns, 0);
    assert_eq!(pacer.decision().divisor, 1);
}

#[test]
fn repeated_timestamp_is_ignored() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    drive_steady(&mut pacer, 1_000_000_000, 10, 8_000_000, 4_000_000);
    let before = pacer.snapshot();
    let decision_before = pacer.decision();
    let last_now = 1_000_000_000 + 10 * 8_000_000;
    for _ in 0..3 {
        let decision = pacer.observe(FrameSample {
            present_ns: last_now,
            work_ns: 99_000_000,
        });
        assert_eq!(decision, decision_before);
    }
    let after = pacer.snapshot();
    assert_eq!(after.anomaly_count, before.anomaly_count + 3);
    assert_eq!(after.sample_count, before.sample_count);
    assert_eq!(after.median_ns, before.median_ns);
    assert_eq!(
        after.work_p95_ns, before.work_p95_ns,
        "a discarded sample contributes no work cost either"
    );
}

#[test]
fn backwards_timestamp_is_ignored() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    drive_steady(&mut pacer, 1_000_000_000, 10, 8_000_000, 4_000_000);
    let before = pacer.snapshot();
    let decision_before = pacer.decision();
    let decision = pacer.observe(FrameSample {
        present_ns: 1_000_000_000 + 5_000_000,
        work_ns: 4_000_000,
    });
    assert_eq!(decision, decision_before);
    assert_eq!(pacer.anomaly_count(), 1);
    assert_eq!(pacer.sample_count(), before.sample_count);
    // The reference is unchanged, so the clock owner can keep going from the last good
    // timestamp and samples accumulate again.
    let resumed = pacer.observe(FrameSample {
        present_ns: 1_000_000_000 + 11 * 8_000_000,
        work_ns: 4_000_000,
    });
    assert_eq!(resumed, decision_before);
    assert_eq!(pacer.sample_count(), before.sample_count + 1);
    assert_eq!(pacer.anomaly_count(), 1);
}

#[test]
fn history_window_is_bounded_under_long_runs() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    drive_steady(&mut pacer, 1_000_000_000, 10_000, HZ_120_PERIOD, 4_000_000);
    assert_eq!(
        pacer.sample_count(),
        HISTORY_CAPACITY,
        "state stays bounded no matter how long the run is"
    );
    assert_eq!(pacer.snapshot().sample_count, HISTORY_CAPACITY);
}

#[test]
fn only_the_newest_samples_are_retained() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    // Fill the ring with an expensive workload, then completely replace it with a cheap one.
    drive_steady(
        &mut pacer,
        1_000_000_000,
        HISTORY_CAPACITY,
        30_000_000,
        28_000_000,
    );
    assert_eq!(pacer.work_median_ns(), 28_000_000);
    drive_steady(
        &mut pacer,
        9_000_000_000,
        HISTORY_CAPACITY,
        HZ_120_PERIOD,
        3_000_000,
    );
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.sample_count, HISTORY_CAPACITY);
    assert_eq!(
        snapshot.work_median_ns, 3_000_000,
        "the expensive samples must have been overwritten in ring order"
    );
    assert_eq!(snapshot.median_ns, HZ_120_PERIOD);
    assert_eq!(snapshot.max_ns, HZ_120_PERIOD);
}

#[test]
fn adaptive_cadence_is_capped_at_max_divisor() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD).expect("valid config"));
    // A frame cost far beyond any allowed cadence must saturate, not run away.
    let decisions = drive_steady(&mut pacer, 1_000_000_000, 200, 500_000_000, 480_000_000);
    let last = decisions.last().copied().expect("decisions recorded");
    assert_eq!(last.divisor, MAX_DIVISOR);
    assert_eq!(
        last.interval_ns,
        u64::from(MAX_DIVISOR) * HZ_120_PERIOD,
        "the recommendation saturates at the configured maximum"
    );
    let mut narrow = Pacer::new(Config::adaptive(HZ_120_PERIOD, 2, 5, 0).expect("valid config"));
    let narrowed = drive_steady(&mut narrow, 1_000_000_000, 200, 500_000_000, 480_000_000);
    assert_eq!(
        narrowed.last().expect("decisions recorded").divisor,
        2,
        "an explicit lower ceiling is honoured"
    );
}

#[test]
fn invalid_configuration_is_rejected() {
    assert_eq!(
        Config::adaptive_default(0).unwrap_err(),
        ConfigError::InvalidRefreshPeriod
    );
    assert_eq!(
        Config::uncapped(0).unwrap_err(),
        ConfigError::InvalidRefreshPeriod
    );
    assert_eq!(
        Config::fixed(HZ_60_PERIOD, 0).unwrap_err(),
        ConfigError::InvalidFixedInterval
    );
    assert_eq!(
        Config::adaptive(HZ_60_PERIOD, 0, 5, 0).unwrap_err(),
        ConfigError::InvalidMaxDivisor
    );
    assert_eq!(
        Config::adaptive(HZ_60_PERIOD, MAX_DIVISOR + 1, 5, 0).unwrap_err(),
        ConfigError::InvalidMaxDivisor
    );
    assert!(Config::adaptive(HZ_60_PERIOD, MAX_DIVISOR, 0, 0).is_ok());
    // A zero margin is a sentinel resolved once, at construction.
    let config = Config::adaptive(HZ_60_PERIOD, 2, 5, 0).expect("valid config");
    assert_eq!(
        config.policy(),
        Policy::Adaptive {
            max_divisor: 2,
            settle_frames: 5,
            margin_ns: HZ_60_PERIOD / 8,
        }
    );
}

#[test]
fn deadline_tolerance_separates_overshoot_from_a_real_miss() {
    let period = HZ_60_PERIOD;
    let config = Config::fixed(period, period).expect("valid config");
    assert_eq!(config.deadline_tolerance_ns(), period / 8);
    let mut pacer = Pacer::new(config);
    // A wait that overshoots its target by 200 us is not a dropped frame.
    drive_steady(&mut pacer, 1_000_000_000, 60, period + 200_000, 5_000_000);
    assert_eq!(pacer.missed_deadlines(), 0);
    // With the grace band removed, the same intervals are all recorded as misses.
    let strict = Config::fixed(period, period)
        .expect("valid config")
        .with_deadline_tolerance_ns(0);
    let mut strict_pacer = Pacer::new(strict);
    drive_steady(
        &mut strict_pacer,
        1_000_000_000,
        60,
        period + 200_000,
        5_000_000,
    );
    assert_eq!(strict_pacer.missed_deadlines(), 60);
    // A genuinely late frame is counted either way.
    let mut pacer = Pacer::new(Config::fixed(period, period).expect("valid config"));
    drive(
        &mut pacer,
        1_000_000_000,
        &[(period + 200_000, 5_000_000); 10],
    );
    let before = pacer.missed_deadlines();
    pacer.observe(FrameSample {
        present_ns: 1_000_000_000 + 11 * (period + 200_000) + 50_000_000,
        work_ns: 50_000_000,
    });
    assert_eq!(pacer.missed_deadlines(), before + 1);
}

#[test]
fn changing_policy_clears_stale_history() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_60_PERIOD).expect("valid config"));
    // An idle cap produces long intervals that say nothing about interactive workloads.
    pacer
        .set_policy(Policy::Fixed {
            interval_ns: 66_666_667,
        })
        .expect("valid policy");
    drive_steady(&mut pacer, 1_000_000_000, 60, 66_666_667, 2_000_000);
    assert_eq!(pacer.sample_count(), 60);
    pacer
        .set_policy(Policy::Adaptive {
            max_divisor: MAX_DIVISOR,
            settle_frames: 5,
            margin_ns: 0,
        })
        .expect("valid policy");
    assert!(
        pacer.is_empty(),
        "idle-cap samples must not be mistaken for interactive frame cost"
    );
    assert_eq!(pacer.decision().divisor, 1);
    // Re-applying the policy already in force is a no-op, not a reset.
    drive_steady(&mut pacer, 9_000_000_000, 10, HZ_60_PERIOD, 4_000_000);
    let count = pacer.sample_count();
    pacer
        .set_policy(Policy::Adaptive {
            max_divisor: MAX_DIVISOR,
            settle_frames: 5,
            margin_ns: 0,
        })
        .expect("valid policy");
    assert_eq!(pacer.sample_count(), count);
    // An invalid policy is rejected and changes nothing.
    assert_eq!(
        pacer
            .set_policy(Policy::Fixed { interval_ns: 0 })
            .unwrap_err(),
        ConfigError::InvalidFixedInterval
    );
    assert_eq!(pacer.sample_count(), count);
}
