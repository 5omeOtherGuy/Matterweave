//! Host-level proof for the engine frame pacer. All time enters as explicit nanosecond
//! timestamps; no clock, sleep, thread or I/O is involved.

use matterweave_pacing::{Config, Pacer};

const HZ_120_PERIOD: u64 = 8_333_333;
const HZ_60_PERIOD: u64 = 16_666_667;
const CAP_30HZ: u64 = 33_333_333;

/// Drive a pacer with `intervals` starting at `start`, returning every decision.
fn drive(pacer: &mut Pacer, start: u64, intervals: &[u64]) -> Vec<matterweave_pacing::Decision> {
    let mut now = start;
    pacer.observe(now);
    intervals
        .iter()
        .map(|interval| {
            now += interval;
            pacer.observe(now)
        })
        .collect()
}

#[test]
fn steady_pacing_high_refresh() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD));
    let decisions = drive(&mut pacer, 1_000_000_000, &[HZ_120_PERIOD; 150]);
    let last = decisions.last().copied().expect("decisions recorded");
    assert_eq!(
        last.divisor, 1,
        "120 Hz workload holds every-refresh cadence"
    );
    assert_eq!(last.interval_ns, HZ_120_PERIOD);
    assert!(last.throttled);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.sample_count, 120);
    assert_eq!(snapshot.median_ns, HZ_120_PERIOD);
    assert_eq!(snapshot.p95_ns, HZ_120_PERIOD);
    assert_eq!(snapshot.jitter_ns, 0);
    assert_eq!(snapshot.missed_deadlines, 0);
    assert_eq!(snapshot.anomaly_count, 0);
}

#[test]
fn steady_pacing_low_cadence() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_60_PERIOD));
    let decisions = drive(&mut pacer, 1_000_000_000, &[CAP_30HZ; 150]);
    let last = decisions.last().copied().expect("decisions recorded");
    assert_eq!(
        last.divisor, 2,
        "30 ms workload on a 60 Hz display settles on every-second-refresh cadence"
    );
    assert_eq!(last.interval_ns, 2 * HZ_60_PERIOD);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.median_ns, CAP_30HZ);
    assert_eq!(snapshot.p95_ns, CAP_30HZ);
    assert_eq!(snapshot.jitter_ns, 0);
}

#[test]
fn fixed_cap_recommends_configured_interval() {
    let mut pacer = Pacer::new(Config::fixed(HZ_120_PERIOD, CAP_30HZ));
    let decisions = drive(&mut pacer, 1_000_000_000, &[HZ_120_PERIOD; 40]);
    for decision in &decisions {
        assert_eq!(decision.interval_ns, CAP_30HZ);
        assert_eq!(decision.divisor, 0);
        assert!(decision.throttled);
    }
    let uncapped = Pacer::new(Config::uncapped(HZ_120_PERIOD)).decision();
    assert_eq!(uncapped.interval_ns, 0);
    assert!(!uncapped.throttled);
}

#[test]
fn boundary_workload_does_not_oscillate() {
    let margin = 1_000_000;
    let mut pacer = Pacer::new(Config::adaptive(HZ_120_PERIOD, 4, 10, margin));
    // Phase A: comfortably inside every-refresh cadence.
    for decision in drive(&mut pacer, 1_000_000_000, &[8_000_000; 60]) {
        assert_eq!(decision.divisor, 1);
    }
    // Phase B: sustained cost above the 1x boundary forces a step up.
    let decisions = drive(&mut pacer, 2_000_000_000, &[8_600_000; 60]);
    assert_eq!(
        decisions.last().expect("decisions recorded").divisor,
        2,
        "sustained over-boundary cost must step up"
    );
    // Phase C: park just below the boundary but inside the margin. The pacer must hold
    // cadence 2x for the whole phase. A controller without hysteresis would drop to 1x
    // here because the parked cost fits the 1x interval.
    let parked = drive(&mut pacer, 3_000_000_000, &[8_250_000; 200]);
    assert!(
        parked.iter().all(|decision| decision.divisor == 2),
        "parked near-boundary workload must not oscillate"
    );
    // Phase D: a workload clearly below the boundary (outside the margin) must
    // eventually step back down, proving hysteresis is not a stuck controller.
    let recovery = drive(&mut pacer, 9_000_000_000, &[6_000_000; 200]);
    assert_eq!(
        recovery.last().expect("decisions recorded").divisor,
        1,
        "clearly cheaper workload must step back down"
    );
}

#[test]
fn isolated_spike_does_not_swing_decision() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD));
    let mut now = 1_000_000_000;
    pacer.observe(now);
    let mut post_warmup = Vec::new();
    for i in 0..170 {
        let interval = if i == 130 { 60_000_000 } else { 8_000_000 };
        now += interval;
        let decision = pacer.observe(now);
        if i >= 119 {
            post_warmup.push(decision);
        }
    }
    assert!(
        post_warmup.iter().all(|decision| decision.divisor == 1),
        "a single outlier must not swing the cadence"
    );
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.median_ns, 8_000_000);
    assert_eq!(snapshot.missed_deadlines, 1);
}

#[test]
fn startup_with_partial_history() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_60_PERIOD));
    assert!(pacer.is_empty());
    assert_eq!(pacer.sample_count(), 0);
    let decisions = drive(&mut pacer, 500_000_000, &[HZ_60_PERIOD; 5]);
    assert_eq!(pacer.sample_count(), 5);
    assert!(!pacer.is_empty());
    assert_eq!(pacer.median_ns(), HZ_60_PERIOD);
    assert_eq!(pacer.p95_ns(), HZ_60_PERIOD);
    assert_eq!(pacer.jitter_ns(), 0);
    assert_eq!(pacer.missed_deadlines(), 0);
    let last = decisions.last().copied().expect("decisions recorded");
    assert_eq!(last.divisor, 1);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.sample_count, 5);
    assert_eq!(snapshot.min_ns, HZ_60_PERIOD);
    assert_eq!(snapshot.max_ns, HZ_60_PERIOD);
    assert_eq!(snapshot.latest_ns, HZ_60_PERIOD);
    assert_eq!(snapshot.target_interval_ns, HZ_60_PERIOD);
    pacer.reset();
    assert!(pacer.is_empty());
    assert_eq!(pacer.snapshot().sample_count, 0);
    assert_eq!(pacer.decision().divisor, 1);
}

#[test]
fn repeated_timestamp_is_ignored() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD));
    drive(&mut pacer, 1_000_000_000, &[8_000_000; 10]);
    let before = pacer.snapshot();
    let decision_before = pacer.decision();
    let last_now = 1_000_000_000 + 10 * 8_000_000;
    for _ in 0..3 {
        let decision = pacer.observe(last_now);
        assert_eq!(decision, decision_before);
    }
    let after = pacer.snapshot();
    assert_eq!(after.anomaly_count, before.anomaly_count + 3);
    assert_eq!(after.sample_count, before.sample_count);
    assert_eq!(after.median_ns, before.median_ns);
    assert_eq!(after.p95_ns, before.p95_ns);
}

#[test]
fn backwards_timestamp_is_ignored() {
    let mut pacer = Pacer::new(Config::adaptive_default(HZ_120_PERIOD));
    drive(&mut pacer, 1_000_000_000, &[8_000_000; 10]);
    let before = pacer.snapshot();
    let decision_before = pacer.decision();
    let decision = pacer.observe(1_000_000_000 + 5_000_000);
    assert_eq!(decision, decision_before);
    assert_eq!(pacer.anomaly_count(), 1);
    assert_eq!(pacer.sample_count(), before.sample_count);
    // The reference is unchanged, so the clock owner can keep going from the last good
    // timestamp and samples accumulate again.
    let resumed = pacer.observe(1_000_000_000 + 10 * 8_000_000 + 8_000_000);
    assert_eq!(resumed, decision_before);
    assert_eq!(pacer.sample_count(), before.sample_count + 1);
    assert_eq!(pacer.anomaly_count(), 1);
}
