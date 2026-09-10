//! Reusable engine-level frame pacing.
//!
//! `matterweave-pacing` decides how the engine should pace presentation and
//! reports honest frame-timing statistics. It is a pure scheduling component:
//! it owns no rendering, physics, windowing, Vulkan, Android or game types and
//! has no external dependencies.
//!
//! # Deterministic, clock-free core contract
//!
//! Time enters the decision logic only as explicit caller-supplied parameters
//! (nanosecond timestamps and measured durations, as `u64`). The core
//! **never** reads a clock, sleeps, allocates after construction, spawns
//! threads, logs, or performs I/O. All state is fixed-capacity. This is what
//! makes every behaviour testable with synthetic timestamps; it is a hard
//! contract, not an implementation detail. Callers (the future engine frame
//! loop) supply monotonic timestamps from their own clock.
//!
//! # Model
//!
//! The caller reports each completed frame with
//! [`Pacer::on_frame(timestamp_ns, frame_cost_ns)`]:
//!
//! - `timestamp_ns` is the caller's timestamp for the completed frame
//!   (typically present submission). Consecutive differences form the
//!   observed *frame interval*.
//! - `frame_cost_ns` is the measured cost of the frame's work (for example
//!   CPU submit plus an estimate of GPU completion). The adaptive policy uses
//!   the recent median cost, which is robust to isolated spikes.
//!
//! The return value is the [`PacingDecision`] to use for subsequent frames:
//! a [`PacingMode`] and an effective target present interval. Policies:
//!
//! - [`PacingPolicy::Uncapped`]: target the display refresh period.
//! - [`PacingPolicy::FixedCap`]: target `max(configured cap, refresh)`.
//! - [`PacingPolicy::Adaptive`]: choose the smallest cadence
//!   `k ∈ 1..=MAX_CADENCE` (present every `k` refresh periods) whose recent
//!   median frame cost fits with headroom, with hysteresis (see below).
//!
//! # Hysteresis (adaptive policy)
//!
//! Let `r` be the display refresh period and `C` the median of the last
//! [`DECISION_WINDOW`] frame costs. Boundaries are asymmetric:
//!
//! - Step down to cadence `k+1` only when `C > 95% · k·r` for
//!   [`HYSTERESIS_FRAMES`] consecutive decision windows.
//! - Step back up to cadence `k−1` only when `C < 75% · (k−1)·r` for
//!   [`HYSTERESIS_FRAMES`] consecutive decision windows.
//!
//! The 20-percentage-point dead band between the two triggers, combined with
//! the sustained-consecutive requirement, means a workload parked near a
//! cadence boundary cannot oscillate cadence frame to frame. A single
//! decision is also gated behind [`WARMUP_FRAMES`] observed frames so
//! startup samples cannot commit a premature cadence.
//!
//! # Statistics
//!
//! The last [`HISTORY_CAPACITY`] valid frame intervals are kept in a
//! fixed-capacity ring. [`Pacer::snapshot`] reports, over that window:
//!
//! - **Median frame time**: the lower median (`sorted[(n−1)/2]`) of valid
//!   present-to-present intervals.
//! - **High-percentile frame time**: the 95th percentile, defined as
//!   `sorted[ceil(0.95·n) − 1]`.
//! - **Jitter**: the mean absolute deviation of valid intervals from their
//!   median, rounded to the nearest nanosecond (`(Σ|Δ| + n/2) / n`).
//! - **Missed display deadlines**: a count of completed frames whose observed
//!   interval exceeded the *effective target interval then in force* by more
//!   than the configured [`PacingConfig::deadline_tolerance_ns`]. The target
//!   in force is the decision computed from frames before this one, so a
//!   frame is judged by the pacing it was actually given.
//! - **Clock anomalies**: frames whose timestamp repeated or went backwards
//!   relative to the previous frame. Such frames contribute their cost (if
//!   any) to cost statistics but no interval, never a division by zero, and
//!   never an interval measured across the anomaly; the previous good
//!   timestamp is retained so the next valid interval is measured from it.
//!
//! # Bounded state
//!
//! Two rings of [`HISTORY_CAPACITY`] `u64` values plus counters. No input
//! sequence can grow memory: `on_frame` allocates nothing after
//! construction. A fresh [`Pacer`] behaves as cadence 1 (display refresh)
//! until [`WARMUP_FRAMES`] valid frames are observed.
//!
//! See `docs/performance/pacing-component.md` for the design record,
//! limitations, and the explicit statement that no device measurement was
//! performed in producing this crate.

use crate::history::Ring;

/// Fixed capacity of the interval and cost history rings.
pub const HISTORY_CAPACITY: usize = 64;
/// Recent frame costs (median) that drive the adaptive decision.
pub const DECISION_WINDOW: usize = 16;
/// Valid frames that must be observed before the adaptive policy may leave
/// its initial cadence.
pub const WARMUP_FRAMES: u32 = 8;
/// Consecutive decision windows that must satisfy a trigger before the
/// adaptive cadence changes.
pub const HYSTERESIS_FRAMES: u32 = 4;
/// Largest supported adaptive cadence (present every `k` refresh periods).
pub const MAX_CADENCE: u32 = 4;

mod history;

/// Frame-cost thresholds for the adaptive policy, expressed as per-mille of a
/// cadence capacity so all arithmetic stays in integers and stays
/// deterministic.
const SATURATED_PERMILLE: u64 = 950;
const RECOVERED_PERMILLE: u64 = 750;

/// Pacing policy configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacingPolicy {
    /// Target the display refresh period; never slow below it.
    Uncapped,
    /// Cap presentation at a fixed interval. The effective target is
    /// `max(present_interval_ns, refresh period)`: a cap faster than the
    /// display is still bounded by the display.
    FixedCap {
        /// Minimum present interval in nanoseconds (e.g. 33_333_334 for 30 Hz).
        present_interval_ns: u64,
    },
    /// Choose the smallest cadence (present every `k` refresh periods,
    /// `k ∈ 1..=[MAX_CADENCE]`) whose recent median frame cost fits with
    /// headroom, with the hysteresis described in the crate docs.
    Adaptive,
}

/// Pacing configuration validated at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacingConfig {
    /// Nominal display refresh period in nanoseconds (e.g. 16_666_667 for 60 Hz).
    pub refresh_period_ns: u64,
    /// Chosen policy.
    pub policy: PacingPolicy,
    /// Grace added to the effective target before an interval counts as a
    /// missed display deadline. A non-zero default is set by
    /// [`PacingConfig::new`] (`refresh/8`, at least 1 ns).
    pub deadline_tolerance_ns: u64,
}

/// Error returned by [`Pacer::new`] for an invalid [`PacingConfig`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// `refresh_period_ns` must be non-zero.
    InvalidRefreshPeriod,
    /// A `FixedCap` policy requires a non-zero `present_interval_ns`.
    InvalidCapInterval,
}

impl PacingConfig {
    /// Builds a configuration with a default deadline tolerance of
    /// `refresh_period_ns / 8` (at least 1 ns).
    pub fn new(refresh_period_ns: u64, policy: PacingPolicy) -> Result<Self, ConfigError> {
        let mut config = Self {
            refresh_period_ns,
            policy,
            deadline_tolerance_ns: 0,
        };
        config.validate()?;
        config.deadline_tolerance_ns = (refresh_period_ns / 8).max(1);
        Ok(config)
    }

    /// Validates the refresh period and any fixed-cap interval. Deadline
    /// tolerance may be zero (exact deadlines).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.refresh_period_ns == 0 {
            return Err(ConfigError::InvalidRefreshPeriod);
        }
        if let PacingPolicy::FixedCap {
            present_interval_ns: 0,
        } = self.policy
        {
            return Err(ConfigError::InvalidCapInterval);
        }
        Ok(())
    }
}

/// Current pacing recommendation: the mode in force and the effective target
/// present interval for subsequent frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacingDecision {
    /// Policy mode currently in force. For [`PacingMode::Adaptive`] this
    /// carries the cadence actually selected.
    pub mode: PacingMode,
    /// Effective target present interval in nanoseconds.
    pub target_interval_ns: u64,
}

/// The pacing mode in force. Reported without leaking internal state such as
/// ring indices or trigger counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacingMode {
    /// Presentation paced only by the display refresh period.
    Uncapped,
    /// Presentation capped at a caller-configured interval.
    FixedCap,
    /// Adaptive cadence: present every `frames_per_present` refresh periods.
    Adaptive {
        /// Selected cadence `k`, in `1..=[MAX_CADENCE]`.
        frames_per_present: u32,
    },
}

/// Telemetry snapshot for a HUD or performance log. Plain data; carries no
/// reference into the pacer's internal representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacingSnapshot {
    /// Policy mode in force.
    pub mode: PacingMode,
    /// Effective target present interval in nanoseconds.
    pub target_interval_ns: u64,
    /// Valid frames observed since construction (saturating).
    pub frames_observed: u64,
    /// Valid intervals currently held in history (0..=[HISTORY_CAPACITY]).
    pub intervals_in_history: usize,
    /// Median present-to-present interval over the history window (0 if none).
    pub median_frame_ns: u64,
    /// 95th-percentile present-to-present interval over the history window
    /// (0 if none).
    pub p95_frame_ns: u64,
    /// Mean absolute deviation of valid intervals from their median, rounded
    /// to the nearest nanosecond (0 if fewer than two intervals).
    pub jitter_ns: u64,
    /// Median of recent frame costs, the statistic the adaptive policy
    /// decides on (0 if none).
    pub median_frame_cost_ns: u64,
    /// Completed frames whose interval exceeded the then-effective target by
    /// more than the deadline tolerance.
    pub missed_deadlines: u64,
    /// Frames whose timestamp repeated or went backwards.
    pub clock_anomalies: u64,
}

/// Engine-level frame pacer. See the crate docs for the full contract.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    config: PacingConfig,
    intervals: Ring,
    costs: Ring,
    prev_timestamp_ns: Option<u64>,
    valid_frames: u64,
    missed_deadlines: u64,
    clock_anomalies: u64,
    cadence: u32,
    down_streak: u32,
    up_streak: u32,
}

impl Pacer {
    /// Validates `config` and constructs a pacer. Allocation happens only
    /// here (none is needed; state is inline) and never again.
    pub fn new(config: PacingConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            intervals: Ring::new(),
            costs: Ring::new(),
            prev_timestamp_ns: None,
            valid_frames: 0,
            missed_deadlines: 0,
            clock_anomalies: 0,
            cadence: 1,
            down_streak: 0,
            up_streak: 0,
        })
    }

    /// The configuration in force.
    pub fn config(&self) -> &PacingConfig {
        &self.config
    }

    /// Reports a completed frame and returns the pacing decision to use for
    /// subsequent frames. `timestamp_ns` is the caller's timestamp for the
    /// completed frame; `frame_cost_ns` is the measured cost of the frame's
    /// work. Both are caller-supplied; nothing is measured here.
    ///
    /// A repeated or backwards timestamp is recorded as a clock anomaly: the
    /// frame's cost still counts, no interval is recorded, and the previous
    /// good timestamp is retained so the next valid interval is measured
    /// from it.
    pub fn on_frame(&mut self, timestamp_ns: u64, frame_cost_ns: u64) -> PacingDecision {
        match self.prev_timestamp_ns {
            None => self.prev_timestamp_ns = Some(timestamp_ns),
            Some(prev) if timestamp_ns > prev => {
                let interval = timestamp_ns - prev;
                let target = self.target_interval_ns();
                if interval > target.saturating_add(self.config.deadline_tolerance_ns) {
                    self.missed_deadlines += 1;
                }
                self.intervals.push(interval);
                self.prev_timestamp_ns = Some(timestamp_ns);
            }
            // Repeated or backwards timestamp: the frame's cost still counts,
            // no interval is recorded, and the previous good timestamp is
            // retained so the next valid interval is measured from it.
            Some(_) => self.clock_anomalies += 1,
        }
        self.costs.push(frame_cost_ns);
        self.valid_frames = self.valid_frames.saturating_add(1);
        self.update_adaptive_decision();
        self.decision()
    }

    /// The current decision without reporting a frame.
    pub fn decision(&self) -> PacingDecision {
        PacingDecision {
            mode: self.mode(),
            target_interval_ns: self.target_interval_ns(),
        }
    }

    /// Telemetry snapshot for a HUD or performance log.
    pub fn snapshot(&self) -> PacingSnapshot {
        let mut buf = [0u64; HISTORY_CAPACITY];
        let n = self.intervals.copy_ordered(&mut buf);
        let (median_frame_ns, p95_frame_ns) = if n == 0 {
            (0, 0)
        } else {
            sort_prefix(&mut buf[..n]);
            (buf[median_index(n)], buf[percentile_95_index(n)])
        };
        let jitter_ns = if n < 2 {
            0
        } else {
            jitter_rounded(&buf[..n], median_frame_ns)
        };
        let mut cost_buf = [0u64; HISTORY_CAPACITY];
        let cost_n = self.costs.copy_ordered(&mut cost_buf);
        let median_frame_cost_ns = if cost_n == 0 {
            0
        } else {
            sort_prefix(&mut cost_buf[..cost_n]);
            cost_buf[median_index(cost_n)]
        };
        PacingSnapshot {
            mode: self.mode(),
            target_interval_ns: self.target_interval_ns(),
            frames_observed: self.valid_frames,
            intervals_in_history: n,
            median_frame_ns,
            p95_frame_ns,
            jitter_ns,
            median_frame_cost_ns,
            missed_deadlines: self.missed_deadlines,
            clock_anomalies: self.clock_anomalies,
        }
    }

    fn mode(&self) -> PacingMode {
        match self.config.policy {
            PacingPolicy::Uncapped => PacingMode::Uncapped,
            PacingPolicy::FixedCap { .. } => PacingMode::FixedCap,
            PacingPolicy::Adaptive => PacingMode::Adaptive {
                frames_per_present: self.cadence,
            },
        }
    }

    /// Effective target present interval for the policy currently in force.
    fn target_interval_ns(&self) -> u64 {
        match self.config.policy {
            PacingPolicy::Uncapped => self.config.refresh_period_ns,
            PacingPolicy::FixedCap {
                present_interval_ns,
            } => present_interval_ns.max(self.config.refresh_period_ns),
            PacingPolicy::Adaptive => self
                .config
                .refresh_period_ns
                .saturating_mul(u64::from(self.cadence)),
        }
    }

    /// Re-evaluates the adaptive cadence from the recent median frame cost.
    /// Non-adaptive policies are unaffected. Until [`WARMUP_FRAMES`] valid
    /// frames exist the cadence stays at its initial value of 1.
    fn update_adaptive_decision(&mut self) {
        if !matches!(self.config.policy, PacingPolicy::Adaptive) {
            return;
        }
        if self.valid_frames < u64::from(WARMUP_FRAMES) {
            return;
        }
        let mut buf = [0u64; HISTORY_CAPACITY];
        let n = self.costs.last_n(DECISION_WINDOW, &mut buf);
        if n == 0 {
            return;
        }
        sort_prefix(&mut buf[..n]);
        let cost = buf[median_index(n)];
        let refresh = self.config.refresh_period_ns;
        let saturated_ns = scale_permille(
            refresh.saturating_mul(u64::from(self.cadence)),
            SATURATED_PERMILLE,
        );
        let recovered_ns = if self.cadence > 1 {
            scale_permille(
                refresh.saturating_mul(u64::from(self.cadence - 1)),
                RECOVERED_PERMILLE,
            )
        } else {
            0
        };
        if cost > saturated_ns {
            self.down_streak = self.down_streak.saturating_add(1);
            self.up_streak = 0;
            if self.down_streak >= HYSTERESIS_FRAMES && self.cadence < MAX_CADENCE {
                self.cadence += 1;
                self.down_streak = 0;
                self.up_streak = 0;
            }
        } else if self.cadence > 1 && cost < recovered_ns {
            self.up_streak = self.up_streak.saturating_add(1);
            self.down_streak = 0;
            if self.up_streak >= HYSTERESIS_FRAMES {
                self.cadence -= 1;
                self.down_streak = 0;
                self.up_streak = 0;
            }
        } else {
            self.down_streak = 0;
            self.up_streak = 0;
        }
    }
}

/// `value * permille / 1000`, saturating.
fn scale_permille(value: u64, permille: u64) -> u64 {
    value.saturating_mul(permille) / 1000
}

/// Lower-median index for `n` sorted values.
fn median_index(n: usize) -> usize {
    (n - 1) / 2
}

/// 95th-percentile index for `n` sorted values: `ceil(0.95·n) − 1`.
fn percentile_95_index(n: usize) -> usize {
    (n * 95).div_ceil(100) - 1
}

/// In-place insertion sort of a short prefix; allocation-free by design.
fn sort_prefix(values: &mut [u64]) {
    for i in 1..values.len() {
        let mut j = i;
        while j > 0 && values[j - 1] > values[j] {
            values.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Mean absolute deviation from `median`, rounded to nearest.
fn jitter_rounded(sorted: &[u64], median: u64) -> u64 {
    let sum: u128 = sorted
        .iter()
        .map(|value| u128::from(value.abs_diff(median)))
        .sum();
    let n = u128::from(u64::try_from(sorted.len()).unwrap_or(u64::MAX));
    ((sum + n / 2) / n).min(u64::MAX as u128) as u64
}
