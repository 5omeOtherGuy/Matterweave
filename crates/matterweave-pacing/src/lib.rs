//! Reusable engine-level frame pacing: cadence decisions and honest frame-timing statistics.
//!
//! The pacer observes frame-boundary timestamps supplied by the caller and recommends how
//! the engine should pace presentation (a target present interval / vsync cadence). It also
//! reports interval statistics suitable for a HUD or performance log.
//!
//! # Hard contract
//!
//! The core is deterministic and clock-free. Time enters only as explicit nanosecond
//! timestamps passed to [`Pacer::observe`]. The core never reads a clock, never sleeps,
//! never allocates after construction, never spawns threads, and performs no logging or I/O.
//! All state lives in fixed-capacity inline arrays; every method works on the caller's stack
//! plus those arrays. There are no external dependencies (std only) and no `unsafe` code.
//! This contract is what makes the component deterministically host-testable.
//!
//! # Model
//!
//! The caller supplies one timestamp per presented frame. Consecutive timestamps form frame
//! intervals stored in a fixed ring of [`HISTORY_CAPACITY`] samples. Statistics (median,
//! high percentile, jitter, missed deadlines) are computed over the stored samples. The
//! adaptive policy picks the smallest vsync multiple of the display refresh period that
//! covers the robust frame cost (the high-percentile interval), with hysteresis so a
//! workload parked near a cadence boundary does not oscillate frame to frame.
//!
//! # Intended use
//!
//! ```
//! use matterweave_pacing::{Config, Pacer};
//! let mut pacer = Pacer::new(Config::adaptive_default(8_333_333));
//! let mut now_ns: u64 = 0;
//! for _ in 0..10 {
//!     now_ns += 8_333_333; // caller-owned clock
//!     let decision = pacer.observe(now_ns);
//!     assert!(decision.throttled);
//!     // caller presents, then sleeps `decision.interval_ns` itself
//! }
//! let hud = pacer.snapshot();
//! assert_eq!(hud.sample_count, 9);
//! ```
//!
//! The caller owns the clock and any sleeping. A `0` decision interval (uncapped policy)
//! means "present immediately".

#![forbid(unsafe_code)]

/// Number of frame intervals retained. Fixed at compile time so state is bounded under
/// any input sequence; old samples are overwritten in ring order.
pub const HISTORY_CAPACITY: usize = 120;

/// Largest vsync multiple the adaptive policy may recommend (1 = every refresh).
pub const MAX_DIVISOR: u32 = 4;

/// Default number of consecutive agreeing observations before the adaptive policy steps
/// down to a faster cadence. See [`Pacer`] for the hysteresis mechanism.
pub const DEFAULT_SETTLE_FRAMES: u32 = 30;

/// Pacing policy selected at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Present immediately; the decision interval is always `0`. Statistics still use the
    /// display refresh period as their missed-deadline reference.
    Uncapped,
    /// Always recommend the configured interval, regardless of observed cost.
    Fixed {
        /// Target present interval in nanoseconds. Must be non-zero.
        interval_ns: u64,
    },
    /// Recommend the smallest vsync multiple of the refresh period covering the robust
    /// frame cost, with hysteresis on step-down (see [`Pacer`]).
    Adaptive {
        /// Largest vsync multiple allowed. Clamped into `1..=MAX_DIVISOR`.
        max_divisor: u32,
        /// Consecutive agreeing observations required before stepping to a faster
        /// cadence. `0` disables the waiting period (the margin still applies).
        settle_frames: u32,
        /// Downgrade guard band in nanoseconds. Stepping to a faster cadence requires the
        /// robust cost plus this margin to fit the faster cadence. `0` selects the default
        /// of one eighth of the refresh period.
        margin_ns: u64,
    },
}

/// Pacer configuration. Everything the core needs is fixed here at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Display refresh period in nanoseconds (e.g. `8_333_333` for 120 Hz). Must be
    /// non-zero. Used as the vsync unit and as the missed-deadline reference for the
    /// uncapped policy.
    pub refresh_period_ns: u64,
    /// Pacing policy.
    pub policy: Policy,
}

impl Config {
    /// Uncapped pacing for the given display refresh period.
    #[must_use]
    pub fn uncapped(refresh_period_ns: u64) -> Self {
        Self {
            refresh_period_ns,
            policy: Policy::Uncapped,
        }
    }

    /// Fixed-cap pacing with the given target present interval.
    #[must_use]
    pub fn fixed(refresh_period_ns: u64, interval_ns: u64) -> Self {
        Self {
            refresh_period_ns,
            policy: Policy::Fixed { interval_ns },
        }
    }

    /// Adaptive pacing with default tuning ([`MAX_DIVISOR`], [`DEFAULT_SETTLE_FRAMES`],
    /// and a margin of one eighth of the refresh period).
    #[must_use]
    pub fn adaptive_default(refresh_period_ns: u64) -> Self {
        Self {
            refresh_period_ns,
            policy: Policy::Adaptive {
                max_divisor: MAX_DIVISOR,
                settle_frames: DEFAULT_SETTLE_FRAMES,
                margin_ns: 0,
            },
        }
    }

    /// Adaptive pacing with explicit tuning.
    #[must_use]
    pub fn adaptive(
        refresh_period_ns: u64,
        max_divisor: u32,
        settle_frames: u32,
        margin_ns: u64,
    ) -> Self {
        Self {
            refresh_period_ns,
            policy: Policy::Adaptive {
                max_divisor,
                settle_frames,
                margin_ns,
            },
        }
    }
}

/// Pacing recommendation produced by [`Pacer::observe`] / [`Pacer::decision`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    /// Recommended minimum interval between presents, in nanoseconds. `0` means present
    /// immediately (uncapped policy); otherwise the caller should wait until at least this
    /// much time has passed since the last present.
    pub interval_ns: u64,
    /// Vsync multiple behind the recommendation (`1` = every refresh). `0` when the
    /// recommendation is not a vsync multiple (uncapped and fixed policies).
    pub divisor: u32,
    /// Whether the engine should delay presentation to honour the recommendation.
    /// Always `interval_ns > 0`.
    pub throttled: bool,
}

/// Telemetry snapshot for a HUD or performance log. Plain data only: it exposes the
/// numbers without leaking internal representation, and is `Copy` so reading it never
/// disturbs the pacer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Number of stored intervals (`0..=HISTORY_CAPACITY`).
    pub sample_count: usize,
    /// Median frame interval: the lower middle of the sorted samples
    /// (index `(n-1)/2`). `0` when empty.
    pub median_ns: u64,
    /// High-percentile frame interval (nearest-rank 95th percentile: `ceil(0.95*n)`-th of
    /// the sorted samples). `0` when empty.
    pub p95_ns: u64,
    /// Jitter: mean absolute deviation of the stored intervals from the median, with
    /// integer division rounding down. `0` when empty.
    pub jitter_ns: u64,
    /// Smallest stored interval. `0` when empty.
    pub min_ns: u64,
    /// Largest stored interval. `0` when empty.
    pub max_ns: u64,
    /// Most recently recorded interval. `0` when empty.
    pub latest_ns: u64,
    /// Count of stored intervals strictly greater than the current target reference.
    /// The reference is the current decision interval, except for the uncapped policy
    /// where it is the display refresh period.
    pub missed_deadlines: usize,
    /// Timestamps discarded for clock anomalies (identical or backwards timestamps).
    pub anomaly_count: u64,
    /// Current target present interval (the decision interval, except uncapped reports the
    /// refresh period as its reference).
    pub target_interval_ns: u64,
    /// Current vsync multiple (`0` for uncapped/fixed policies; see [`Decision`]).
    pub divisor: u32,
    /// Whether presentation should currently be delayed (mirrors [`Decision::throttled`]).
    pub throttled: bool,
}

/// Deterministic, clock-free frame pacer. See the [crate-level documentation](crate) for
/// the hard contract.
///
/// # Adaptive hysteresis
///
/// The robust cost is the high-percentile (p95) interval. Stepping to a *slower* cadence
/// (larger divisor) is immediate: as soon as p95 exceeds the current cadence interval, the
/// pacer jumps directly to the smallest divisor whose interval covers p95. Stepping to a
/// *faster* cadence is guarded twice: p95 plus the margin must fit the next-faster cadence
/// interval, and that condition must hold for `settle_frames` consecutive observations;
/// the pacer then steps down exactly one divisor and re-confirms before stepping further.
/// Either guard failing resets the confirmation counter. A workload parked within the
/// margin of a cadence boundary therefore holds its cadence instead of oscillating.
pub struct Pacer {
    config: Config,
    last_ns: Option<u64>,
    samples: [u64; HISTORY_CAPACITY],
    len: usize,
    head: usize,
    latest_ns: u64,
    divisor: u32,
    stable: u32,
    anomalies: u64,
}

impl Pacer {
    /// Create a pacer. Panics if `refresh_period_ns` is zero, or if a fixed policy
    /// interval is zero. Construction performs the only allocation-like setup (inline
    /// arrays); no allocation happens afterwards.
    #[must_use]
    pub fn new(config: Config) -> Self {
        assert!(
            config.refresh_period_ns > 0,
            "refresh period must be non-zero"
        );
        if let Policy::Fixed { interval_ns } = config.policy {
            assert!(interval_ns > 0, "fixed interval must be non-zero");
        }
        Self {
            config,
            last_ns: None,
            samples: [0; HISTORY_CAPACITY],
            len: 0,
            head: 0,
            latest_ns: 0,
            divisor: 1,
            stable: 0,
            anomalies: 0,
        }
    }

    /// Observe a frame-boundary timestamp in nanoseconds and return the pacing decision.
    ///
    /// The first call arms the clock reference and records no interval. A timestamp that
    /// is identical to or older than the previous one is a clock anomaly: it is discarded
    /// (the reference is left unchanged), [`Snapshot::anomaly_count`] is incremented
    /// saturatingly, and the current decision is returned unchanged.
    pub fn observe(&mut self, now_ns: u64) -> Decision {
        match self.last_ns {
            None => {
                self.last_ns = Some(now_ns);
            }
            Some(last) => {
                if now_ns <= last {
                    self.anomalies = self.anomalies.saturating_add(1);
                    return self.decision();
                }
                self.push(now_ns - last);
                self.last_ns = Some(now_ns);
                self.update_adaptive();
            }
        }
        self.decision()
    }

    /// Current pacing decision without recording anything.
    #[must_use]
    pub fn decision(&self) -> Decision {
        match self.config.policy {
            Policy::Uncapped => Decision {
                interval_ns: 0,
                divisor: 0,
                throttled: false,
            },
            Policy::Fixed { interval_ns } => Decision {
                interval_ns,
                divisor: 0,
                throttled: true,
            },
            Policy::Adaptive { .. } => {
                let interval =
                    u64::from(self.divisor).saturating_mul(self.config.refresh_period_ns);
                Decision {
                    interval_ns: interval,
                    divisor: self.divisor,
                    throttled: true,
                }
            }
        }
    }

    /// Number of stored intervals (`0..=HISTORY_CAPACITY`).
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.len.min(HISTORY_CAPACITY)
    }

    /// Whether any interval has been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sample_count() == 0
    }

    /// Discarded non-monotonic timestamps observed so far.
    #[must_use]
    pub fn anomaly_count(&self) -> u64 {
        self.anomalies
    }

    /// Copy of the stored intervals, sorted ascending. Returns the sample count; only
    /// `out[..count]` is valid. Uses caller-provided stack storage: no allocation.
    fn ordered_samples(&self, out: &mut [u64; HISTORY_CAPACITY]) -> usize {
        let count = self.sample_count();
        if count == 0 {
            return 0;
        }
        let start = (self.head + HISTORY_CAPACITY - count) % HISTORY_CAPACITY;
        for (slot, i) in out.iter_mut().zip(0..count) {
            *slot = self.samples[(start + i) % HISTORY_CAPACITY];
        }
        out[..count].sort_unstable();
        count
    }

    /// Median frame interval (lower median). `0` when empty.
    #[must_use]
    pub fn median_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered_samples(&mut sorted);
        if count == 0 {
            return 0;
        }
        sorted[(count - 1) / 2]
    }

    /// High-percentile frame interval (nearest-rank p95). `0` when empty.
    #[must_use]
    pub fn p95_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered_samples(&mut sorted);
        if count == 0 {
            return 0;
        }
        let rank = (95 * count).div_ceil(100);
        sorted[rank - 1]
    }

    /// Jitter: mean absolute deviation from the median (rounded down). `0` when empty.
    #[must_use]
    pub fn jitter_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered_samples(&mut sorted);
        if count == 0 {
            return 0;
        }
        let median = sorted[(count - 1) / 2];
        let mut total: u128 = 0;
        for sample in sorted[..count].iter() {
            total += (*sample).abs_diff(median) as u128;
        }
        (total / count as u128) as u64
    }

    /// Current target reference for missed-deadline counting and telemetry: the decision
    /// interval, except the uncapped policy reports the refresh period.
    #[must_use]
    pub fn target_interval_ns(&self) -> u64 {
        match self.config.policy {
            Policy::Uncapped => self.config.refresh_period_ns,
            Policy::Fixed { interval_ns } => interval_ns,
            Policy::Adaptive { .. } => {
                u64::from(self.divisor).saturating_mul(self.config.refresh_period_ns)
            }
        }
    }

    /// Count of stored intervals strictly greater than [`Self::target_interval_ns`].
    #[must_use]
    pub fn missed_deadlines(&self) -> usize {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered_samples(&mut sorted);
        let target = self.target_interval_ns();
        sorted[..count]
            .iter()
            .filter(|sample| **sample > target)
            .count()
    }

    /// Telemetry snapshot. Reading it never disturbs the pacer.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered_samples(&mut sorted);
        let (min_ns, max_ns, median_ns, p95_ns, jitter_ns) = if count == 0 {
            (0, 0, 0, 0, 0)
        } else {
            let median = sorted[(count - 1) / 2];
            let rank = (95 * count).div_ceil(100);
            let mut total: u128 = 0;
            for sample in sorted[..count].iter() {
                total += (*sample).abs_diff(median) as u128;
            }
            (
                sorted[0],
                sorted[count - 1],
                median,
                sorted[rank - 1],
                (total / count as u128) as u64,
            )
        };
        let target = self.target_interval_ns();
        let decision = self.decision();
        Snapshot {
            sample_count: count,
            median_ns,
            p95_ns,
            jitter_ns,
            min_ns,
            max_ns,
            latest_ns: if count == 0 { 0 } else { self.latest_ns },
            missed_deadlines: sorted[..count]
                .iter()
                .filter(|sample| **sample > target)
                .count(),
            anomaly_count: self.anomalies,
            target_interval_ns: target,
            divisor: decision.divisor,
            throttled: decision.throttled,
        }
    }

    /// Clear all samples, anomalies and adaptive state. The next [`Self::observe`] call
    /// re-arms the clock reference. Intended for lifecycle recovery (e.g. returning from
    /// background) where pre-existing timestamps are stale.
    pub fn reset(&mut self) {
        self.last_ns = None;
        self.samples = [0; HISTORY_CAPACITY];
        self.len = 0;
        self.head = 0;
        self.latest_ns = 0;
        self.divisor = 1;
        self.stable = 0;
        self.anomalies = 0;
    }

    fn push(&mut self, interval_ns: u64) {
        self.samples[self.head] = interval_ns;
        self.head = (self.head + 1) % HISTORY_CAPACITY;
        self.len = self.len.saturating_add(1).min(HISTORY_CAPACITY);
        self.latest_ns = interval_ns;
    }

    fn adaptive_params(&self) -> Option<(u32, u32, u64)> {
        if let Policy::Adaptive {
            max_divisor,
            settle_frames,
            margin_ns,
        } = self.config.policy
        {
            let max = max_divisor.clamp(1, MAX_DIVISOR);
            let margin = if margin_ns == 0 {
                self.config.refresh_period_ns / 8
            } else {
                margin_ns
            };
            Some((max, settle_frames, margin))
        } else {
            None
        }
    }

    fn update_adaptive(&mut self) {
        let Some((max_divisor, settle_frames, margin_ns)) = self.adaptive_params() else {
            return;
        };
        let period = self.config.refresh_period_ns;
        let cost = self.p95_ns();
        let mut desired: u32 = 1;
        while desired < max_divisor && u64::from(desired).saturating_mul(period) < cost {
            desired += 1;
        }
        if desired > self.divisor {
            self.divisor = desired;
            self.stable = 0;
        } else if desired < self.divisor {
            let lower = u64::from(self.divisor - 1).saturating_mul(period);
            if cost.saturating_add(margin_ns) <= lower {
                self.stable = self.stable.saturating_add(1);
                if self.stable >= settle_frames.max(1) {
                    self.divisor -= 1;
                    self.stable = 0;
                }
            } else {
                self.stable = 0;
            }
        } else {
            self.stable = 0;
        }
    }
}
