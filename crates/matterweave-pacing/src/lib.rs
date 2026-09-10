//! Reusable engine-level frame pacing: cadence decisions and honest frame-timing statistics.
//!
//! The pacer observes frame samples supplied by the caller and recommends how the engine
//! should pace presentation (a target present interval / vsync cadence). It also reports
//! interval statistics suitable for a HUD or performance log.
//!
//! # Hard contract
//!
//! The core is deterministic and clock-free. Time enters only as explicit nanosecond values
//! inside [`FrameSample`]. The core never reads a clock, never sleeps, never allocates after
//! construction, never spawns threads, and performs no logging or I/O. All state lives in
//! fixed-capacity inline arrays; every method works on the caller's stack plus those arrays.
//! There are no external dependencies (std only) and no `unsafe` code. This contract is what
//! makes the component deterministically host-testable.
//!
//! # Model
//!
//! Each presented frame contributes one [`FrameSample`] carrying two independent numbers:
//!
//! - `present_ns`: the frame-boundary timestamp. Consecutive timestamps form *presentation
//!   intervals*, which are what the display actually showed. These drive the statistics.
//! - `work_ns`: the cost of *producing* that frame, excluding any pacing wait the caller
//!   performed. This is the load signal, and it alone drives the adaptive cadence.
//!
//! Keeping these apart is essential, not cosmetic. A paced loop sleeps until its target
//! interval, so the presentation interval of a healthy frame equals the target (plus the
//! caller's wait overshoot) no matter how cheap the frame was. Deriving load from that
//! number closes a positive-feedback loop: every overshoot looks like extra cost, the pacer
//! slows down, the longer interval overshoots again, and the cadence ratchets to its slowest
//! setting and stays there. `work_ns` is not throttled by the pacer's own output, so it
//! cannot drive that loop. See `tests/closed_loop.rs` for the executable proof.
//!
//! Both series live in fixed rings of [`HISTORY_CAPACITY`] samples; old samples are
//! overwritten in ring order.
//!
//! # Intended use
//!
//! ```
//! use matterweave_pacing::{Config, FrameSample, Pacer};
//! let config = Config::adaptive_default(8_333_333).expect("valid refresh period");
//! let mut pacer = Pacer::new(config);
//! let mut now_ns: u64 = 0;
//! for _ in 0..10 {
//!     now_ns += 8_333_333; // caller-owned clock
//!     let decision = pacer.observe(FrameSample {
//!         present_ns: now_ns,
//!         work_ns: 3_000_000, // measured frame production cost
//!     });
//!     assert!(decision.throttled);
//!     // caller presents, then waits out `decision.interval_ns` itself
//! }
//! let hud = pacer.snapshot();
//! assert_eq!(hud.sample_count, 9);
//! ```
//!
//! The caller owns the clock and any waiting. A `0` decision interval (uncapped policy)
//! means "present immediately".

#![forbid(unsafe_code)]

/// Number of frame samples retained per series. Fixed at compile time so state is bounded
/// under any input sequence; old samples are overwritten in ring order.
pub const HISTORY_CAPACITY: usize = 120;

/// Largest vsync multiple the adaptive policy may be configured to use (1 = every refresh).
pub const MAX_DIVISOR: u32 = 4;

/// Default number of consecutive agreeing observations before the adaptive policy steps
/// down to a faster cadence. See [`Pacer`] for the hysteresis mechanism.
pub const DEFAULT_SETTLE_FRAMES: u32 = 30;

/// One presented frame: when it reached the display, and what it cost to produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSample {
    /// Monotonic frame-boundary timestamp in nanoseconds, on the caller's clock.
    /// Consecutive values form the presentation intervals reported by [`Snapshot`].
    pub present_ns: u64,
    /// Cost of producing this frame in nanoseconds: the work the engine actually did,
    /// **excluding** any wait the caller performed to honour a previous decision. This is
    /// the load signal for the adaptive policy; see the [module docs](crate) for why it
    /// must not be the presentation interval.
    pub work_ns: u64,
}

/// Pacing policy.
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
        /// Largest vsync multiple allowed. Must be in `1..=MAX_DIVISOR`.
        max_divisor: u32,
        /// Consecutive agreeing observations required before stepping to a faster
        /// cadence. `0` disables the waiting period (the margin still applies).
        settle_frames: u32,
        /// Downgrade guard band in nanoseconds. Stepping to a faster cadence requires the
        /// robust cost plus this margin to fit the faster cadence. Passing `0` to a
        /// [`Config`] constructor selects the default of one eighth of the refresh period
        /// and stores that resolved value; a stored policy never carries the sentinel.
        margin_ns: u64,
    },
}

/// Why a [`Config`] or [`Policy`] was rejected. Configuration is validated where the
/// invalid value enters, so a constructed [`Config`] is always valid and [`Pacer::new`]
/// cannot fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// `refresh_period_ns` was zero. A display period is required as the vsync unit and as
    /// the missed-deadline reference.
    InvalidRefreshPeriod,
    /// A [`Policy::Fixed`] interval was zero.
    InvalidFixedInterval,
    /// A [`Policy::Adaptive`] `max_divisor` was outside `1..=MAX_DIVISOR`.
    InvalidMaxDivisor,
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::InvalidRefreshPeriod => "display refresh period must be non-zero",
            Self::InvalidFixedInterval => "fixed present interval must be non-zero",
            Self::InvalidMaxDivisor => "adaptive max_divisor must be in 1..=MAX_DIVISOR",
        };
        f.write_str(text)
    }
}

impl std::error::Error for ConfigError {}

/// Validated pacer configuration. Fields are private so an invalid configuration cannot be
/// constructed; every constructor validates and returns [`ConfigError`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    refresh_period_ns: u64,
    policy: Policy,
    deadline_tolerance_ns: u64,
}

/// Validate a policy and resolve any sentinel against the refresh period.
fn resolve_policy(refresh_period_ns: u64, policy: Policy) -> Result<Policy, ConfigError> {
    match policy {
        Policy::Uncapped => Ok(Policy::Uncapped),
        Policy::Fixed { interval_ns } => {
            if interval_ns == 0 {
                Err(ConfigError::InvalidFixedInterval)
            } else {
                Ok(Policy::Fixed { interval_ns })
            }
        }
        Policy::Adaptive {
            max_divisor,
            settle_frames,
            margin_ns,
        } => {
            if !(1..=MAX_DIVISOR).contains(&max_divisor) {
                return Err(ConfigError::InvalidMaxDivisor);
            }
            Ok(Policy::Adaptive {
                max_divisor,
                settle_frames,
                margin_ns: if margin_ns == 0 {
                    default_margin_ns(refresh_period_ns)
                } else {
                    margin_ns
                },
            })
        }
    }
}

/// Default adaptive guard band: one eighth of the refresh period, at least 1 ns.
fn default_margin_ns(refresh_period_ns: u64) -> u64 {
    (refresh_period_ns / 8).max(1)
}

/// Default grace before a presentation interval counts as a missed deadline: one eighth of
/// the refresh period, at least 1 ns. A caller's wait always overshoots its target slightly;
/// without a grace band every paced frame would be recorded as a miss.
fn default_deadline_tolerance_ns(refresh_period_ns: u64) -> u64 {
    (refresh_period_ns / 8).max(1)
}

impl Config {
    /// Build a configuration for an arbitrary policy.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the refresh period is zero or the policy is invalid.
    pub fn new(refresh_period_ns: u64, policy: Policy) -> Result<Self, ConfigError> {
        if refresh_period_ns == 0 {
            return Err(ConfigError::InvalidRefreshPeriod);
        }
        Ok(Self {
            refresh_period_ns,
            policy: resolve_policy(refresh_period_ns, policy)?,
            deadline_tolerance_ns: default_deadline_tolerance_ns(refresh_period_ns),
        })
    }

    /// Uncapped pacing for the given display refresh period.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRefreshPeriod`] when the refresh period is zero.
    pub fn uncapped(refresh_period_ns: u64) -> Result<Self, ConfigError> {
        Self::new(refresh_period_ns, Policy::Uncapped)
    }

    /// Fixed-cap pacing with the given target present interval.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the refresh period or the interval is zero.
    pub fn fixed(refresh_period_ns: u64, interval_ns: u64) -> Result<Self, ConfigError> {
        Self::new(refresh_period_ns, Policy::Fixed { interval_ns })
    }

    /// Adaptive pacing with default tuning ([`MAX_DIVISOR`], [`DEFAULT_SETTLE_FRAMES`],
    /// and a margin of one eighth of the refresh period).
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidRefreshPeriod`] when the refresh period is zero.
    pub fn adaptive_default(refresh_period_ns: u64) -> Result<Self, ConfigError> {
        Self::new(
            refresh_period_ns,
            Policy::Adaptive {
                max_divisor: MAX_DIVISOR,
                settle_frames: DEFAULT_SETTLE_FRAMES,
                margin_ns: 0,
            },
        )
    }

    /// Adaptive pacing with explicit tuning. A `margin_ns` of `0` selects the default.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the refresh period is zero or `max_divisor` is outside
    /// `1..=MAX_DIVISOR`.
    pub fn adaptive(
        refresh_period_ns: u64,
        max_divisor: u32,
        settle_frames: u32,
        margin_ns: u64,
    ) -> Result<Self, ConfigError> {
        Self::new(
            refresh_period_ns,
            Policy::Adaptive {
                max_divisor,
                settle_frames,
                margin_ns,
            },
        )
    }

    /// Replace the missed-deadline grace band. `0` counts every interval past the target as
    /// a miss (exact deadlines).
    #[must_use]
    pub fn with_deadline_tolerance_ns(mut self, deadline_tolerance_ns: u64) -> Self {
        self.deadline_tolerance_ns = deadline_tolerance_ns;
        self
    }

    /// Display refresh period in nanoseconds.
    #[must_use]
    pub fn refresh_period_ns(&self) -> u64 {
        self.refresh_period_ns
    }

    /// The validated policy, with any sentinel already resolved.
    #[must_use]
    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Grace added to the target before an interval counts as a missed deadline.
    #[must_use]
    pub fn deadline_tolerance_ns(&self) -> u64 {
        self.deadline_tolerance_ns
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
    /// Number of stored samples (`0..=HISTORY_CAPACITY`). Both series share this count.
    pub sample_count: usize,
    /// Median presentation interval: the lower middle of the sorted samples
    /// (index `(n-1)/2`). `0` when empty.
    pub median_ns: u64,
    /// High-percentile presentation interval (nearest-rank 95th percentile:
    /// `ceil(0.95*n)`-th of the sorted samples). `0` when empty.
    pub p95_ns: u64,
    /// Jitter: mean absolute deviation of the presentation intervals from their median,
    /// with integer division rounding down. `0` when empty.
    pub jitter_ns: u64,
    /// Smallest stored presentation interval. `0` when empty.
    pub min_ns: u64,
    /// Largest stored presentation interval. `0` when empty.
    pub max_ns: u64,
    /// Most recently recorded presentation interval. `0` when empty.
    pub latest_ns: u64,
    /// Median frame production cost. `0` when empty.
    pub work_median_ns: u64,
    /// High-percentile frame production cost (same nearest-rank rule). This is the robust
    /// cost driving the adaptive cadence. `0` when empty.
    pub work_p95_ns: u64,
    /// Most recently recorded frame production cost. `0` when empty.
    pub work_latest_ns: u64,
    /// Count of stored presentation intervals strictly greater than the current target
    /// reference plus [`Config::deadline_tolerance_ns`]. The reference is the current
    /// decision interval, except for the uncapped policy where it is the display refresh
    /// period.
    pub missed_deadlines: usize,
    /// Samples discarded for clock anomalies (identical or backwards timestamps).
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
/// The robust cost is the high-percentile (p95) *frame production cost*, never the paced
/// presentation interval. Stepping to a *slower* cadence (larger divisor) is immediate: as
/// soon as the robust cost exceeds the current cadence interval, the pacer jumps directly
/// to the smallest divisor whose interval covers it. Stepping to a *faster* cadence is
/// guarded twice: the robust cost plus the margin must fit the next-faster cadence
/// interval, and that condition must hold for `settle_frames` consecutive observations; the
/// pacer then steps down exactly one divisor and re-confirms before stepping further.
/// Either guard failing resets the confirmation counter. A workload parked within the
/// margin of a cadence boundary therefore holds its cadence instead of oscillating.
pub struct Pacer {
    config: Config,
    last_ns: Option<u64>,
    intervals: [u64; HISTORY_CAPACITY],
    work: [u64; HISTORY_CAPACITY],
    len: usize,
    head: usize,
    latest_interval_ns: u64,
    latest_work_ns: u64,
    divisor: u32,
    stable: u32,
    anomalies: u64,
}

impl Pacer {
    /// Create a pacer from a validated [`Config`]. Construction performs the only
    /// allocation-like setup (inline arrays); no allocation happens afterwards.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            last_ns: None,
            intervals: [0; HISTORY_CAPACITY],
            work: [0; HISTORY_CAPACITY],
            len: 0,
            head: 0,
            latest_interval_ns: 0,
            latest_work_ns: 0,
            divisor: 1,
            stable: 0,
            anomalies: 0,
        }
    }

    /// The configuration in force.
    #[must_use]
    pub fn config(&self) -> Config {
        self.config
    }

    /// Replace the policy, keeping the display refresh period and deadline tolerance.
    ///
    /// History is cleared, because a policy change alters what the recorded intervals mean:
    /// samples taken under a 15 Hz idle cap are not evidence about a 120 Hz workload. The
    /// next [`Self::observe`] call re-arms the clock reference.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the policy is invalid; the pacer is left untouched.
    pub fn set_policy(&mut self, policy: Policy) -> Result<(), ConfigError> {
        let resolved = resolve_policy(self.config.refresh_period_ns, policy)?;
        if resolved == self.config.policy {
            return Ok(());
        }
        self.config.policy = resolved;
        self.reset();
        Ok(())
    }

    /// Observe one presented frame and return the pacing decision.
    ///
    /// The first call arms the clock reference and records no interval. A `present_ns` that
    /// is identical to or older than the previous one is a clock anomaly: the whole sample
    /// is discarded (the reference is left unchanged), [`Snapshot::anomaly_count`] is
    /// incremented saturatingly, and the current decision is returned unchanged.
    pub fn observe(&mut self, sample: FrameSample) -> Decision {
        match self.last_ns {
            None => {
                self.last_ns = Some(sample.present_ns);
            }
            Some(last) => {
                if sample.present_ns <= last {
                    self.anomalies = self.anomalies.saturating_add(1);
                    return self.decision();
                }
                self.push(sample.present_ns - last, sample.work_ns);
                self.last_ns = Some(sample.present_ns);
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
            Policy::Adaptive { .. } => Decision {
                interval_ns: u64::from(self.divisor).saturating_mul(self.config.refresh_period_ns),
                divisor: self.divisor,
                throttled: true,
            },
        }
    }

    /// Number of stored samples (`0..=HISTORY_CAPACITY`).
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.len.min(HISTORY_CAPACITY)
    }

    /// Whether any sample has been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sample_count() == 0
    }

    /// Discarded non-monotonic samples observed so far.
    #[must_use]
    pub fn anomaly_count(&self) -> u64 {
        self.anomalies
    }

    /// Copy one series into `out`, sorted ascending. Returns the sample count; only
    /// `out[..count]` is valid. Uses caller-provided stack storage: no allocation.
    fn ordered(
        &self,
        series: &[u64; HISTORY_CAPACITY],
        out: &mut [u64; HISTORY_CAPACITY],
    ) -> usize {
        let count = self.sample_count();
        if count == 0 {
            return 0;
        }
        let start = (self.head + HISTORY_CAPACITY - count) % HISTORY_CAPACITY;
        for (slot, i) in out.iter_mut().zip(0..count) {
            *slot = series[(start + i) % HISTORY_CAPACITY];
        }
        out[..count].sort_unstable();
        count
    }

    /// Lower median of a sorted prefix. `0` when empty.
    fn median_of(sorted: &[u64]) -> u64 {
        if sorted.is_empty() {
            0
        } else {
            sorted[(sorted.len() - 1) / 2]
        }
    }

    /// Nearest-rank 95th percentile of a sorted prefix. `0` when empty.
    fn p95_of(sorted: &[u64]) -> u64 {
        if sorted.is_empty() {
            0
        } else {
            sorted[(95 * sorted.len()).div_ceil(100) - 1]
        }
    }

    /// Mean absolute deviation from the median of a sorted prefix, rounded down.
    fn jitter_of(sorted: &[u64]) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let median = Self::median_of(sorted);
        let total: u128 = sorted
            .iter()
            .map(|sample| u128::from(sample.abs_diff(median)))
            .sum();
        (total / sorted.len() as u128) as u64
    }

    /// Median presentation interval. `0` when empty.
    #[must_use]
    pub fn median_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.intervals, &mut sorted);
        Self::median_of(&sorted[..count])
    }

    /// High-percentile presentation interval (nearest-rank p95). `0` when empty.
    #[must_use]
    pub fn p95_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.intervals, &mut sorted);
        Self::p95_of(&sorted[..count])
    }

    /// Jitter: mean absolute deviation of presentation intervals from their median. `0`
    /// when empty.
    #[must_use]
    pub fn jitter_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.intervals, &mut sorted);
        Self::jitter_of(&sorted[..count])
    }

    /// Median frame production cost. `0` when empty.
    #[must_use]
    pub fn work_median_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.work, &mut sorted);
        Self::median_of(&sorted[..count])
    }

    /// High-percentile frame production cost: the robust cost driving the adaptive
    /// cadence. `0` when empty.
    #[must_use]
    pub fn work_p95_ns(&self) -> u64 {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.work, &mut sorted);
        Self::p95_of(&sorted[..count])
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

    /// Count of stored presentation intervals strictly greater than
    /// [`Self::target_interval_ns`] plus the configured deadline tolerance.
    #[must_use]
    pub fn missed_deadlines(&self) -> usize {
        let mut sorted = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.intervals, &mut sorted);
        let limit = self
            .target_interval_ns()
            .saturating_add(self.config.deadline_tolerance_ns);
        sorted[..count].iter().filter(|s| **s > limit).count()
    }

    /// Telemetry snapshot. Reading it never disturbs the pacer.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        let mut intervals = [0_u64; HISTORY_CAPACITY];
        let count = self.ordered(&self.intervals, &mut intervals);
        let intervals = &intervals[..count];
        let mut work = [0_u64; HISTORY_CAPACITY];
        let work_count = self.ordered(&self.work, &mut work);
        let work = &work[..work_count];
        let limit = self
            .target_interval_ns()
            .saturating_add(self.config.deadline_tolerance_ns);
        let decision = self.decision();
        Snapshot {
            sample_count: count,
            median_ns: Self::median_of(intervals),
            p95_ns: Self::p95_of(intervals),
            jitter_ns: Self::jitter_of(intervals),
            min_ns: intervals.first().copied().unwrap_or(0),
            max_ns: intervals.last().copied().unwrap_or(0),
            latest_ns: if count == 0 {
                0
            } else {
                self.latest_interval_ns
            },
            work_median_ns: Self::median_of(work),
            work_p95_ns: Self::p95_of(work),
            work_latest_ns: if count == 0 { 0 } else { self.latest_work_ns },
            missed_deadlines: intervals.iter().filter(|s| **s > limit).count(),
            anomaly_count: self.anomalies,
            target_interval_ns: self.target_interval_ns(),
            divisor: decision.divisor,
            throttled: decision.throttled,
        }
    }

    /// Clear all samples, anomalies and adaptive state. The next [`Self::observe`] call
    /// re-arms the clock reference. Intended for lifecycle recovery (e.g. returning from
    /// background, or recreating the renderer) where pre-existing timestamps are stale.
    pub fn reset(&mut self) {
        self.last_ns = None;
        self.intervals = [0; HISTORY_CAPACITY];
        self.work = [0; HISTORY_CAPACITY];
        self.len = 0;
        self.head = 0;
        self.latest_interval_ns = 0;
        self.latest_work_ns = 0;
        self.divisor = 1;
        self.stable = 0;
        self.anomalies = 0;
    }

    fn push(&mut self, interval_ns: u64, work_ns: u64) {
        self.intervals[self.head] = interval_ns;
        self.work[self.head] = work_ns;
        self.head = (self.head + 1) % HISTORY_CAPACITY;
        self.len = self.len.saturating_add(1).min(HISTORY_CAPACITY);
        self.latest_interval_ns = interval_ns;
        self.latest_work_ns = work_ns;
    }

    fn update_adaptive(&mut self) {
        let Policy::Adaptive {
            max_divisor,
            settle_frames,
            margin_ns,
        } = self.config.policy
        else {
            return;
        };
        let period = self.config.refresh_period_ns;
        let cost = self.work_p95_ns();
        let mut desired: u32 = 1;
        while desired < max_divisor && u64::from(desired).saturating_mul(period) < cost {
            desired += 1;
        }
        if desired > self.divisor {
            self.divisor = desired;
            self.stable = 0;
        } else if desired < self.divisor {
            let faster = u64::from(self.divisor - 1).saturating_mul(period);
            if cost.saturating_add(margin_ns) <= faster {
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
