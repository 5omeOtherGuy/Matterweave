//! Behaviour tests for the clock-free pacing core. All timestamps are
//! synthetic; no clock is read anywhere.

use matterweave_pacing::{
    ConfigError, Pacer, PacingConfig, PacingDecision, PacingMode, PacingPolicy, MAX_CADENCE,
};

const REFRESH_60HZ_EQUIVALENT_NS: u64 = 16_000_000;
const TARGET_NS: u64 = REFRESH_60HZ_EQUIVALENT_NS;

fn adaptive_pacer() -> Pacer {
    Pacer::new(PacingConfig::new(TARGET_NS, PacingPolicy::Adaptive).unwrap()).unwrap()
}

fn uncapped_pacer() -> Pacer {
    Pacer::new(PacingConfig::new(TARGET_NS, PacingPolicy::Uncapped).unwrap()).unwrap()
}

/// Synthetic frame sequence: `n` frames with a fixed cost, timestamps spaced
/// `step_ns` apart starting at `step_ns` so the first frame establishes the
/// baseline and every later frame yields an interval.
fn run_steady(pacer: &mut Pacer, n: u32, step_ns: u64, cost_ns: u64) -> Vec<PacingDecision> {
    let mut decisions = Vec::with_capacity(n as usize);
    for frame in 1..=n {
        let timestamp = u64::from(frame) * step_ns;
        decisions.push(pacer.on_frame(timestamp, cost_ns));
    }
    decisions
}

#[test]
fn steady_pacing_at_high_refresh_cadence() {
    let mut pacer = adaptive_pacer();
    let decisions = run_steady(&mut pacer, 40, TARGET_NS, 10_000_000);
    for decision in &decisions {
        assert_eq!(
            decision.mode,
            PacingMode::Adaptive {
                frames_per_present: 1
            }
        );
        assert_eq!(decision.target_interval_ns, TARGET_NS);
    }
    let snapshot = pacer.snapshot();
    assert_eq!(
        snapshot.mode,
        PacingMode::Adaptive {
            frames_per_present: 1
        }
    );
    assert_eq!(snapshot.target_interval_ns, TARGET_NS);
    assert_eq!(snapshot.frames_observed, 40);
    assert_eq!(snapshot.intervals_in_history, 39);
    assert_eq!(snapshot.median_frame_ns, TARGET_NS);
    assert_eq!(snapshot.p95_frame_ns, TARGET_NS);
    assert_eq!(snapshot.jitter_ns, 0);
    assert_eq!(snapshot.median_frame_cost_ns, 10_000_000);
    assert_eq!(snapshot.missed_deadlines, 0);
    assert_eq!(snapshot.clock_anomalies, 0);
}

#[test]
fn steady_pacing_at_lower_cadence() {
    let mut pacer = adaptive_pacer();
    // 25 ms frames cannot fit the 16 ms refresh: the policy must settle at
    // cadence 2 (32 ms) and stop counting missed deadlines once paced there.
    let decisions = run_steady(&mut pacer, 40, 25_000_000, 25_000_000);
    assert!(
        decisions.iter().any(|d| d.target_interval_ns == 32_000_000),
        "adaptive policy never reached the lower cadence"
    );
    for decision in decisions
        .iter()
        .skip_while(|d| d.target_interval_ns != 32_000_000)
    {
        assert_eq!(
            decision.mode,
            PacingMode::Adaptive {
                frames_per_present: 2
            }
        );
        assert_eq!(decision.target_interval_ns, 32_000_000);
    }
    let after_adaptation = pacer.snapshot();
    assert_eq!(
        after_adaptation.mode,
        PacingMode::Adaptive {
            frames_per_present: 2
        }
    );
    assert_eq!(after_adaptation.target_interval_ns, 32_000_000);
    assert!(
        after_adaptation.missed_deadlines > 0,
        "missed deadlines while paced at the too-fast cadence were not counted"
    );
    let missed_when_adapted = after_adaptation.missed_deadlines;
    // Once paced at cadence 2, 25 ms intervals meet the 32 ms target within
    // tolerance: no further deadline is missed.
    run_steady(&mut pacer, 10, 25_000_000, 25_000_000);
    let later = pacer.snapshot();
    assert_eq!(later.missed_deadlines, missed_when_adapted);
    assert_eq!(later.median_frame_ns, 25_000_000);
}

#[test]
fn workload_parked_near_cadence_boundary_does_not_oscillate() {
    let mut pacer = adaptive_pacer();
    let mut timestamp = 0u64;
    let mut report = |pacer: &mut Pacer, cost_ns: u64| -> PacingDecision {
        timestamp += TARGET_NS;
        pacer.on_frame(timestamp, cost_ns)
    };

    // Phase A: sustained cost just above the cadence-1 saturation boundary
    // (95% of 16 ms = 15.2 ms). After warmup and the sustained-trigger
    // window the policy must commit to cadence 2.
    let mut reached_cadence_two = false;
    for _ in 0..20 {
        let decision = report(&mut pacer, 15_500_000);
        if decision.target_interval_ns == 32_000_000 {
            reached_cadence_two = true;
            break;
        }
    }
    assert!(
        reached_cadence_two,
        "hysteresis test is vacuous: cadence 2 was never reached"
    );

    // Phase B: the workload is parked at the boundary, alternating just
    // below (15.0 ms) and just above (15.5 ms) the cadence-1 threshold. A
    // hysteresis-free policy (single threshold, or dead-band removal) would
    // fall back to cadence 1 here and then flip again; the decision must
    // hold at cadence 2 for every frame.
    for frame in 0..40 {
        let cost = if frame % 2 == 0 {
            15_000_000
        } else {
            15_500_000
        };
        let decision = report(&mut pacer, cost);
        assert_eq!(
            decision,
            PacingDecision {
                mode: PacingMode::Adaptive {
                    frames_per_present: 2
                },
                target_interval_ns: 32_000_000,
            },
            "decision oscillated at frame {frame} of the boundary phase"
        );
    }
}

#[test]
fn isolated_spike_does_not_swing_decision() {
    let mut pacer = adaptive_pacer();
    // Steady 10 ms frames; frame 30 costs 200 ms. A mean- or max-based
    // decision would exceed the 15.2 ms boundary and step down; the
    // median-based decision must not move.
    let mut decisions = Vec::new();
    for frame in 1u32..=60 {
        let cost = if frame == 30 { 200_000_000 } else { 10_000_000 };
        decisions.push(pacer.on_frame(u64::from(frame) * TARGET_NS, cost));
    }
    for (index, decision) in decisions.iter().enumerate() {
        assert_eq!(
            *decision,
            PacingDecision {
                mode: PacingMode::Adaptive {
                    frames_per_present: 1
                },
                target_interval_ns: TARGET_NS,
            },
            "decision moved at frame index {index} after an isolated spike"
        );
    }
    // The spike is honestly recorded in cost statistics but the median the
    // policy decides on is unaffected.
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.median_frame_cost_ns, 10_000_000);
    assert_eq!(snapshot.missed_deadlines, 0);
}

#[test]
fn startup_with_fewer_samples_than_history_capacity() {
    let mut pacer = adaptive_pacer();
    // Five heavy frames: below WARMUP_FRAMES and below HISTORY_CAPACITY.
    // Stats must be valid, and the adaptive policy must hold its initial
    // cadence until warmup completes.
    let decisions = run_steady(&mut pacer, 5, TARGET_NS, 25_000_000);
    for decision in &decisions {
        assert_eq!(
            *decision,
            PacingDecision {
                mode: PacingMode::Adaptive {
                    frames_per_present: 1
                },
                target_interval_ns: TARGET_NS,
            }
        );
    }
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.frames_observed, 5);
    assert_eq!(snapshot.intervals_in_history, 4);
    assert_eq!(snapshot.median_frame_ns, TARGET_NS);
    assert_eq!(snapshot.p95_frame_ns, TARGET_NS);
    assert_eq!(snapshot.jitter_ns, 0);
    assert_eq!(snapshot.median_frame_cost_ns, 25_000_000);
    // Warmup completes and the sustained heavy cost is then adopted normally.
    let mut timestamp = 5 * TARGET_NS;
    let mut reached = false;
    for _ in 0..12 {
        timestamp += TARGET_NS;
        if pacer.on_frame(timestamp, 25_000_000).target_interval_ns == 32_000_000 {
            reached = true;
            break;
        }
    }
    assert!(reached, "policy never adapted after warmup");
}

#[test]
fn repeated_identical_timestamps_are_counted_as_anomalies() {
    let mut pacer = adaptive_pacer();
    let constant_ts = 1_000_000_000;
    for _ in 0..5 {
        let _ = pacer.on_frame(constant_ts, 10_000_000);
    }
    let snapshot = pacer.snapshot();
    // The first frame establishes the baseline; the four repeats are anomalies.
    assert_eq!(snapshot.clock_anomalies, 4);
    assert_eq!(snapshot.intervals_in_history, 0);
    assert_eq!(snapshot.median_frame_ns, 0);
    assert_eq!(snapshot.p95_frame_ns, 0);
    assert_eq!(snapshot.jitter_ns, 0);
    // Cost statistics and the decision continue to work.
    assert_eq!(snapshot.median_frame_cost_ns, 10_000_000);
    assert_eq!(
        snapshot.mode,
        PacingMode::Adaptive {
            frames_per_present: 1
        }
    );
    // The next valid timestamp measures from the retained good baseline.
    let _ = pacer.on_frame(constant_ts + TARGET_NS, 10_000_000);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.clock_anomalies, 4);
    assert_eq!(snapshot.intervals_in_history, 1);
    assert_eq!(snapshot.median_frame_ns, TARGET_NS);
}

#[test]
fn backwards_timestamp_is_rejected_and_counted() {
    let mut pacer = adaptive_pacer();
    assert_eq!(
        pacer.on_frame(1_000, 10_000_000).mode,
        PacingMode::Adaptive {
            frames_per_present: 1
        }
    );
    assert_eq!(
        pacer.on_frame(900, 10_000_000).mode,
        PacingMode::Adaptive {
            frames_per_present: 1
        }
    );
    assert_eq!(
        pacer.on_frame(800, 10_000_000).mode,
        PacingMode::Adaptive {
            frames_per_present: 1
        }
    );
    // The interval after the anomalies is measured from the last good
    // timestamp (1_000), not from the rejected one.
    let _ = pacer.on_frame(17_000, 10_000_000);
    let snapshot = pacer.snapshot();
    assert_eq!(snapshot.clock_anomalies, 2);
    assert_eq!(snapshot.intervals_in_history, 1);
    assert_eq!(snapshot.median_frame_ns, 16_000);
    assert_eq!(snapshot.missed_deadlines, 0);
}

#[test]
fn history_window_is_bounded_under_long_runs() {
    let mut pacer = adaptive_pacer();
    let decisions = run_steady(&mut pacer, 200, TARGET_NS, 10_000_000);
    assert_eq!(decisions.len(), 200);
    let snapshot = pacer.snapshot();
    assert_eq!(
        snapshot.intervals_in_history,
        matterweave_pacing::HISTORY_CAPACITY
    );
    assert_eq!(snapshot.frames_observed, 200);
    assert_eq!(snapshot.median_frame_ns, TARGET_NS);
    assert_eq!(snapshot.p95_frame_ns, TARGET_NS);
    assert_eq!(
        snapshot.mode,
        PacingMode::Adaptive {
            frames_per_present: 1
        }
    );
}

#[test]
fn uncapped_policy_targets_display_refresh() {
    let mut pacer = uncapped_pacer();
    let decisions = run_steady(&mut pacer, 10, 25_000_000, 25_000_000);
    for decision in &decisions {
        assert_eq!(decision.mode, PacingMode::Uncapped);
        assert_eq!(decision.target_interval_ns, TARGET_NS);
    }
    let snapshot = pacer.snapshot();
    // Every observed interval (25 ms) missed the 16 ms deadline beyond the
    // 2 ms tolerance.
    assert_eq!(snapshot.missed_deadlines, 9);
    assert_eq!(snapshot.median_frame_ns, 25_000_000);
}

#[test]
fn fixed_cap_policy_targets_configured_interval_bounded_by_refresh() {
    let slower_than_refresh = Pacer::new(
        PacingConfig::new(
            TARGET_NS,
            PacingPolicy::FixedCap {
                present_interval_ns: 33_333_334,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let decision = slower_than_refresh.decision();
    assert_eq!(decision.mode, PacingMode::FixedCap);
    assert_eq!(decision.target_interval_ns, 33_333_334);

    // A cap faster than the display is still bounded by the display.
    let faster_than_refresh = Pacer::new(
        PacingConfig::new(
            TARGET_NS,
            PacingPolicy::FixedCap {
                present_interval_ns: 8_000_000,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let decision = faster_than_refresh.decision();
    assert_eq!(decision.mode, PacingMode::FixedCap);
    assert_eq!(decision.target_interval_ns, TARGET_NS);
}

#[test]
fn invalid_configuration_is_rejected() {
    assert_eq!(
        PacingConfig::new(0, PacingPolicy::Uncapped),
        Err(ConfigError::InvalidRefreshPeriod)
    );
    assert_eq!(
        PacingConfig::new(
            TARGET_NS,
            PacingPolicy::FixedCap {
                present_interval_ns: 0
            }
        ),
        Err(ConfigError::InvalidCapInterval)
    );
}

#[test]
fn adaptive_cadence_is_capped_at_max_cadence() {
    let mut pacer = adaptive_pacer();
    // Costs far above every cadence capacity: the policy must saturate at
    // MAX_CADENCE, not grow without bound.
    let mut timestamp = 0u64;
    let mut last = PacingMode::Adaptive {
        frames_per_present: 1,
    };
    for _ in 0..200 {
        timestamp += 100_000_000;
        last = pacer.on_frame(timestamp, 100_000_000).mode;
    }
    assert_eq!(
        last,
        PacingMode::Adaptive {
            frames_per_present: MAX_CADENCE
        }
    );
    assert_eq!(pacer.decision().target_interval_ns, 4 * TARGET_NS);
}
