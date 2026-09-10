//! Native frame-pacing gate. Drives the production engine pacer
//! (`matterweave_pacing`) against a real window, a real Vulkan present and the
//! real display refresh, with a controlled synthetic frame cost.
//!
//! This is a correctness gate for frame-loop scheduling, not a throughput or
//! efficiency benchmark: it makes no claim about frames per second, power or
//! heat. What it establishes is that on this display, with this loop's real wait
//! overshoot, the pacer selects the cadence its own definition requires, holds a
//! cost parked inside the guard band instead of oscillating, and recovers when
//! the load clears.
//!
//! Every expectation is derived from the frame cost this device actually
//! measured, never from the cost the phase requested. A device that cannot place
//! its frame cost in the window a phase needs reports INCONCLUSIVE; it never
//! reports PASS.
use glam::{Mat4, Vec3};
use matterweave_core::World;
use matterweave_pacing::{
    Config as PacingConfig, FrameSample, Pacer, Policy as PacingPolicy, Snapshot,
    DEFAULT_SETTLE_FRAMES, MAX_DIVISOR,
};
use matterweave_render::{DrawDiagnostics, FrameResult, Hud, LightingSettings, Renderer};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow},
    window::{Window, WindowId},
};

/// Frames presented before a phase's samples are read, so the pacer's 120-sample
/// history holds only this phase's cost. The step-down guard additionally needs
/// its settle window, which is why the recovery phase is the longest.
const RECOVERY_FRAMES: u32 = 360;
const STEADY_FRAMES: u32 = 240;

/// One measured phase: a synthetic frame cost as permille of the display refresh
/// period, and the cadence it must produce.
struct Phase {
    name: &'static str,
    cost_permille: u64,
    frames: u32,
    claim: &'static str,
}

const PHASES: &[Phase] = &[
    Phase {
        name: "baseline",
        cost_permille: 250,
        frames: STEADY_FRAMES,
        claim: "a cheap frame holds every-refresh cadence despite real wait overshoot",
    },
    Phase {
        name: "loaded",
        cost_permille: 1500,
        frames: STEADY_FRAMES,
        claim: "a frame that cannot fit one refresh steps to every second refresh",
    },
    Phase {
        name: "parked",
        cost_permille: 950,
        frames: STEADY_FRAMES,
        claim: "a cost parked inside the guard band below the boundary holds its cadence \
                instead of oscillating",
    },
    Phase {
        name: "recovered",
        cost_permille: 250,
        frames: RECOVERY_FRAMES,
        claim: "a cleared load returns to every-refresh cadence",
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pass,
    Fail,
    Inconclusive,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Inconclusive => "INCONCLUSIVE",
        }
    }
}

/// Cadence observed across the frames of one phase.
///
/// `Snapshot::divisor` is only the value after the last frame, so a cadence that
/// oscillated and happened to end on the expected multiple is indistinguishable
/// from one that held it. Recording every per-frame `decision.divisor` makes the
/// number of changes and the observed range part of the phase's evidence.
#[derive(Clone, Copy, Default)]
struct CadenceStats {
    /// Times the divisor differed from the previous frame's value.
    changes: u32,
    /// Smallest and largest divisor observed, both `0` before the first frame.
    min: u32,
    max: u32,
    last: Option<u32>,
}

impl CadenceStats {
    fn record(&mut self, divisor: u32) {
        match self.last {
            None => {
                self.min = divisor;
                self.max = divisor;
            }
            Some(last) => {
                if last != divisor {
                    self.changes = self.changes.saturating_add(1);
                }
                self.min = self.min.min(divisor);
                self.max = self.max.max(divisor);
            }
        }
        self.last = Some(divisor);
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Display-paced blocking inside one rendered frame, in nanoseconds: the upload
/// and submission fence waits plus the swapchain acquire. `present_ms` is
/// deliberately excluded because it times queueing the present request, not
/// waiting on scanout. `None` when the renderer did not measure it.
fn blocked_ns(diagnostics: Option<DrawDiagnostics>) -> Option<u64> {
    let diagnostics = diagnostics?;
    let ms = diagnostics.upload_fence_wait_ms.unwrap_or(0.)
        + diagnostics.render_fence_wait_ms.unwrap_or(0.)
        + diagnostics.acquire_ms.unwrap_or(0.);
    if !ms.is_finite() || ms < 0. {
        return None;
    }
    Some(u64::try_from(Duration::from_secs_f64(ms / 1000.).as_nanos()).unwrap_or(u64::MAX))
}

/// Check one finished phase against what this device actually measured.
///
/// `margin_ns` is the pacer's own step-down guard band, so the windows here are
/// the windows the policy is defined in terms of.
fn judge(
    phase: &str,
    snapshot: &Snapshot,
    cadence: CadenceStats,
    period_ns: u64,
    margin_ns: u64,
) -> (Outcome, String) {
    let cost = snapshot.work_p95_ns;
    let divisor = snapshot.divisor;
    match phase {
        "baseline" | "recovered" => {
            // Every-refresh cadence is only required of a cost that actually fits a
            // refresh with the guard band to spare.
            if cost + margin_ns > period_ns {
                return (
                    Outcome::Inconclusive,
                    format!(
                        "measured cost p95 {cost} ns does not fit one {period_ns} ns refresh \
                         with the {margin_ns} ns guard band; this device cannot produce a frame \
                         cheap enough for the phase"
                    ),
                );
            }
            if divisor == 1 {
                (Outcome::Pass, format!("held 1x at cost p95 {cost} ns"))
            } else {
                (
                    Outcome::Fail,
                    format!("cost p95 {cost} ns fits one refresh but cadence is {divisor}x"),
                )
            }
        }
        "loaded" => {
            if cost <= period_ns || cost > 2 * period_ns {
                return (
                    Outcome::Inconclusive,
                    format!(
                        "measured cost p95 {cost} ns is outside the one-to-two refresh window \
                         ({period_ns}..={} ns) this phase needs",
                        2 * period_ns
                    ),
                );
            }
            if divisor == 2 {
                (
                    Outcome::Pass,
                    format!("stepped to 2x at cost p95 {cost} ns"),
                )
            } else {
                (
                    Outcome::Fail,
                    format!("cost p95 {cost} ns needs 2x but cadence is {divisor}x"),
                )
            }
        }
        "parked" => {
            // The hysteresis claim only means anything when the cost really is inside
            // the guard band below the one-refresh boundary.
            if cost > period_ns || cost + margin_ns <= period_ns {
                return (
                    Outcome::Inconclusive,
                    format!(
                        "measured cost p95 {cost} ns is not inside the guard band \
                         ({}..={period_ns} ns) this phase needs",
                        period_ns.saturating_sub(margin_ns)
                    ),
                );
            }
            // The final divisor is not evidence of a held cadence. Require that no frame
            // of the phase changed cadence, and claim only what was checked.
            if cadence.changes > 0 {
                return (
                    Outcome::Fail,
                    format!(
                        "parked cost p95 {cost} ns changed cadence {} times (divisor {}..={}) \
                         instead of holding one cadence",
                        cadence.changes, cadence.min, cadence.max
                    ),
                );
            }
            if divisor == 2 {
                (
                    Outcome::Pass,
                    format!(
                        "held cadence 2x with no cadence change at parked cost p95 {cost} ns \
                         (divisor constant at {}x)",
                        cadence.min
                    ),
                )
            } else {
                (
                    Outcome::Fail,
                    format!("parked cost p95 {cost} ns dropped cadence to {divisor}x"),
                )
            }
        }
        other => (
            Outcome::Fail,
            format!("no expectation defined for phase {other}"),
        ),
    }
}

fn scene() -> World {
    let mut world = World::new(9);
    for x in -3i32..=3 {
        for z in -3i32..=3 {
            world.set([x, -1, z], 1);
            for y in 0..3 {
                if x.abs() == 3 || z.abs() == 3 {
                    world.set([x, y, z], 2);
                }
            }
        }
    }
    world
}

pub struct PacingCheck {
    report: PathBuf,
    lines: Vec<String>,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    pacer: Option<Pacer>,
    period_ns: u64,
    margin_ns: u64,
    epoch: Instant,
    next_frame: Instant,
    phase: usize,
    phase_frames: u32,
    /// Frames whose real render already cost more than the phase requested, so no
    /// synthetic cost could be added. Reported rather than hidden.
    over_budget: u32,
    /// Per-phase cadence evidence: every `decision.divisor`, not just the last.
    cadence: CadenceStats,
    /// Sum of the raw measured frame cost over the current phase, before the
    /// display-paced blocked share was subtracted.
    phase_raw_work_ns: u64,
    /// Sum of the blocked share measured over the current phase's frames.
    phase_blocked_ns: u64,
    /// Frames of the current phase whose blocked time the renderer did not
    /// measure. Reported rather than silently read as zero.
    phase_blocked_unavailable: u32,
    outcome: Outcome,
    /// Why the run ended before every phase was judged, if a lifecycle event
    /// destroyed the state the remaining phases would have been judged against.
    interrupted: Option<String>,
    finished: bool,
}

impl PacingCheck {
    pub fn new(report: PathBuf) -> Self {
        Self {
            report,
            lines: Vec::new(),
            renderer: None,
            window: None,
            pacer: None,
            period_ns: 0,
            margin_ns: 0,
            epoch: Instant::now(),
            next_frame: Instant::now(),
            phase: 0,
            phase_frames: 0,
            over_budget: 0,
            cadence: CadenceStats::default(),
            phase_raw_work_ns: 0,
            phase_blocked_ns: 0,
            phase_blocked_unavailable: 0,
            outcome: Outcome::Pass,
            interrupted: None,
            finished: false,
        }
    }

    fn record(&mut self, line: String) {
        log::info!("pacing-check: {line}");
        println!("pacing-check: {line}");
        self.lines.push(line);
        if let Err(e) = fs::write(&self.report, self.lines.join("\n") + "\n") {
            log::error!("pacing-check report not written: {e}");
        }
    }

    /// Finish the current phase: judge it, report it, and arm the next one.
    fn close_phase(&mut self, event_loop: &ActiveEventLoop) {
        let Some(pacer) = &mut self.pacer else {
            return;
        };
        let snapshot = pacer.snapshot();
        let phase = &PHASES[self.phase];
        let cadence = self.cadence;
        let (outcome, detail) = judge(
            phase.name,
            &snapshot,
            cadence,
            self.period_ns,
            self.margin_ns,
        );
        if outcome != Outcome::Pass && self.outcome != Outcome::Fail {
            self.outcome = outcome;
        }
        let ms = |ns: u64| ns as f64 / 1e6;
        let measured_frames = u64::from(self.phase_frames.max(1));
        let raw_mean_ns = self.phase_raw_work_ns / measured_frames;
        let blocked_mean_ns = self.phase_blocked_ns / measured_frames;
        let blocked_share_pct =
            100. * self.phase_blocked_ns as f64 / self.phase_raw_work_ns.max(1) as f64;
        let line = format!(
            "{} phase={} requested_cost_permille={} frames={} work_p50_ms={:.3} \
             work_p95_ms={:.3} work_raw_mean_ms={:.3} blocked_mean_ms={:.3} \
             blocked_share_pct={:.1} blocked_unavailable_frames={} interval_p50_ms={:.3} \
             interval_p95_ms={:.3} jitter_ms={:.3} missed_deadlines={}/{} anomalies={} \
             divisor={} cadence_changes={} divisor_min={} divisor_max={} target_ms={:.3} \
             over_budget_frames={} claim=\"{}\" detail=\"{}\"",
            outcome.label(),
            phase.name,
            phase.cost_permille,
            phase.frames,
            ms(snapshot.work_median_ns),
            ms(snapshot.work_p95_ns),
            ms(raw_mean_ns),
            ms(blocked_mean_ns),
            blocked_share_pct,
            self.phase_blocked_unavailable,
            ms(snapshot.median_ns),
            ms(snapshot.p95_ns),
            ms(snapshot.jitter_ns),
            snapshot.missed_deadlines,
            snapshot.sample_count,
            snapshot.anomaly_count,
            snapshot.divisor,
            cadence.changes,
            cadence.min,
            cadence.max,
            ms(snapshot.target_interval_ns),
            self.over_budget,
            phase.claim,
            detail,
        );
        self.record(line);
        self.phase += 1;
        self.phase_frames = 0;
        self.over_budget = 0;
        self.cadence.reset();
        self.phase_raw_work_ns = 0;
        self.phase_blocked_ns = 0;
        self.phase_blocked_unavailable = 0;
        if self.phase >= PHASES.len() {
            self.finish(event_loop);
        }
    }

    fn finish(&mut self, event_loop: &ActiveEventLoop) {
        if self.finished {
            return;
        }
        self.finished = true;
        let interruption = match &self.interrupted {
            Some(reason) => format!(" Run interrupted before judging every phase: {reason}."),
            None => String::new(),
        };
        let verdict = format!(
            "{} pacing gate: {} of {} phases closed on a {:.3} ms display period; expectations \
             derived from measured frame cost.{interruption} Correctness only: no throughput, \
             power or thermal claim.",
            self.outcome.label(),
            self.phase,
            PHASES.len(),
            self.period_ns as f64 / 1e6,
        );
        self.record(verdict);
        self.renderer = None;
        self.window = None;
        event_loop.exit();
    }
}

impl ApplicationHandler for PacingCheck {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // A run that already finished — including one ended by a suspension — must not
        // be restarted: `Pacer::new` would hand it an empty history and a divisor of 1
        // while `phase`, `phase_frames` and `outcome` kept the state a suspend destroyed.
        if self.finished || self.interrupted.is_some() || self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Matterweave | pacing gate")
                        .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
                )
                .expect("pacing gate window"),
        );
        let mut renderer =
            pollster::block_on(Renderer::new(window.clone())).expect("pacing gate renderer");
        // Measure display-paced blocking on this gate's own renderer. The gate owns the
        // renderer, so nothing else contends for the flag, and `blocked_ns` can then
        // subtract it from the cost fed to the pacer.
        renderer.set_diagnostics_enabled(true);
        renderer.upload(&scene().mesh()).expect("pacing gate scene");
        // The display, not a constant, sets the vsync unit this gate measures against.
        let period_ns = window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .filter(|rate| *rate > 0)
            .map_or(16_666_667, |rate| 1_000_000_000_000 / u64::from(rate));
        let config = PacingConfig::new(
            period_ns,
            PacingPolicy::Adaptive {
                max_divisor: MAX_DIVISOR,
                settle_frames: DEFAULT_SETTLE_FRAMES,
                margin_ns: 0,
            },
        )
        .expect("a non-zero display period and a static policy are always valid");
        let PacingPolicy::Adaptive { margin_ns, .. } = config.policy() else {
            unreachable!("the policy was just constructed as adaptive")
        };
        self.period_ns = period_ns;
        self.margin_ns = margin_ns;
        self.pacer = Some(Pacer::new(config));
        self.epoch = Instant::now();
        self.next_frame = Instant::now();
        self.record(format!(
            "start display_period_ms={:.3} guard_band_ms={:.3} settle_frames={DEFAULT_SETTLE_FRAMES} \
             max_divisor={MAX_DIVISOR} graphics=\"{}\"",
            period_ns as f64 / 1e6,
            margin_ns as f64 / 1e6,
            renderer.capabilities
        ));
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            self.record("FAIL closed before the gate finished".into());
            event_loop.exit();
            return;
        }
        if let WindowEvent::Resized(size) = event {
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(size.width, size.height);
            }
            return;
        }
        if !matches!(event, WindowEvent::RedrawRequested) || self.finished {
            return;
        }
        let (Some(renderer), Some(window)) = (&mut self.renderer, &self.window) else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let frame_start = Instant::now();
        renderer.begin_frame_diagnostics();
        let eye = Vec3::new(1.8, 1.6, 1.8);
        let view = Mat4::perspective_rh(
            70f32.to_radians(),
            size.width as f32 / size.height as f32,
            0.05,
            100.,
        ) * Mat4::look_at_rh(eye, Vec3::new(-2., 0.9, -1.), Vec3::Y);
        let result = renderer.render_with_lighting(
            view.to_cols_array_2d(),
            eye.to_array(),
            &Hud::new(size.width as f32, size.height as f32),
            &LightingSettings::default(),
        );
        let diagnostics = renderer.draw_diagnostics();
        match result {
            FrameResult::Presented => {}
            FrameResult::Retry => return,
            other => {
                self.record(format!("FAIL renderer: {other:?}"));
                self.finished = true;
                event_loop.exit();
                return;
            }
        }
        // Place this frame's production cost where the phase needs it. The real
        // render already happened; the remainder is spent here, on this device, so
        // the cost the pacer sees is measured rather than asserted.
        let target = Duration::from_nanos(
            self.period_ns
                .saturating_mul(PHASES[self.phase].cost_permille)
                / 1000,
        );
        if frame_start.elapsed() >= target {
            self.over_budget += 1;
        }
        while frame_start.elapsed() < target {
            std::hint::spin_loop();
        }
        let finished_at = Instant::now();
        // With FIFO present mode the acquire and the submission fence can block on the
        // display's own cadence. That wait sits inside the wall time of the frame, but it
        // is not engine work: feeding display-paced time to the pacer as load is exactly
        // the feedback the cost/interval split exists to keep out of the load signal. The
        // renderer measured the blocking parts, so subtract them (saturating) and report
        // both the raw measured cost and the blocked share per phase.
        let blocked = blocked_ns(diagnostics);
        let raw_work_ns = u64::try_from(
            finished_at
                .saturating_duration_since(frame_start)
                .as_nanos(),
        )
        .unwrap_or(u64::MAX);
        let sample = FrameSample {
            present_ns: u64::try_from(finished_at.saturating_duration_since(self.epoch).as_nanos())
                .unwrap_or(u64::MAX),
            work_ns: raw_work_ns.saturating_sub(blocked.unwrap_or(0)),
        };
        self.phase_raw_work_ns = self.phase_raw_work_ns.saturating_add(raw_work_ns);
        match blocked {
            Some(ns) => self.phase_blocked_ns = self.phase_blocked_ns.saturating_add(ns),
            None => {
                if self.phase_blocked_unavailable == 0 {
                    self.record(
                        "blocked-time diagnostics unavailable; frame cost is reported \
                         uncorrected for this phase"
                            .into(),
                    );
                }
                self.phase_blocked_unavailable += 1;
            }
        }
        let decision = self
            .pacer
            .as_mut()
            .expect("pacer exists once resumed")
            .observe(sample);
        self.cadence.record(decision.divisor);
        // Honour the recommendation exactly as the production loop does, so the
        // wait overshoot this gate measures is a real one.
        self.next_frame = frame_start + Duration::from_nanos(decision.interval_ns);
        self.phase_frames += 1;
        if self.phase_frames >= PHASES[self.phase].frames {
            self.close_phase(event_loop);
        }
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
        if self.finished {
            return;
        }
        // Do not resume this run. Resuming rebuilds the pacer with `Pacer::new`, whose
        // divisor starts at 1 and whose history is empty, while `phase`, `phase_frames`
        // and `outcome` survive: `parked` inherited its 2x cadence from `loaded` and
        // could never reach it again, so `judge` would return FAIL for a cadence this run
        // never dropped. A gate that reports a failure the code did not commit is as wrong
        // as one that reports a pass it did not earn, so end the run INCONCLUSIVE and name
        // the interruption instead of judging phases against destroyed state.
        self.outcome = Outcome::Inconclusive;
        self.interrupted = Some(
            "the window was suspended mid-run, so a rebuilt pacer cannot be compared \
             against phases measured on the pre-suspend pacer"
                .into(),
        );
        self.record(
            "INCONCLUSIVE run interrupted: window suspended mid-run; remaining phases were \
             not judged"
                .into(),
        );
        self.finish(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.finished {
            return;
        }
        // While suspended there is no window to redraw, so `request_redraw` cannot advance
        // the deadline: `WaitUntil` on an already-passed `next_frame` wakes immediately and
        // forever, burning a core until the platform kills the process.
        let Some(window) = &self.window else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        if Instant::now() >= self.next_frame {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parked-phase evidence must see every frame, not only the last one.
    #[test]
    fn cadence_stats_counts_changes_and_tracks_the_range() {
        let mut stats = CadenceStats::default();
        for divisor in [2, 2, 2] {
            stats.record(divisor);
        }
        assert_eq!(
            (stats.changes, stats.min, stats.max),
            (0, 2, 2),
            "a held cadence has no changes and a single observed value"
        );
        // A cadence that oscillates and happens to end where it started must still report
        // its changes and the full range. `Snapshot::divisor` alone would show only 2x.
        for divisor in [1, 1, 2] {
            stats.record(divisor);
        }
        assert_eq!((stats.changes, stats.min, stats.max), (2, 1, 2));
        stats.reset();
        assert_eq!((stats.changes, stats.min, stats.max), (0, 0, 0));
    }

    /// Only display-paced waits are subtracted, and unmeasured blocking is reported as
    /// unavailable rather than silently read as zero.
    #[test]
    fn blocked_ns_sums_only_display_paced_waits() {
        let diagnostics = DrawDiagnostics {
            upload_fence_wait_ms: Some(100.),
            render_fence_wait_ms: Some(100.),
            acquire_ms: Some(50.),
            // Queueing the present request is not blocking on scanout.
            present_ms: Some(1000.),
            ..Default::default()
        };
        assert_eq!(blocked_ns(Some(diagnostics)), Some(250_000_000));
        assert_eq!(blocked_ns(None), None);
        // A component the renderer did not measure contributes nothing; the sum is over
        // the available waits.
        let partial = DrawDiagnostics {
            render_fence_wait_ms: Some(125.),
            ..Default::default()
        };
        assert_eq!(blocked_ns(Some(partial)), Some(125_000_000));
    }
}
