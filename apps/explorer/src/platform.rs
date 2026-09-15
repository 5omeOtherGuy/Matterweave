//! Platform lifecycle state shared by the Android entry and every sample.
//!
//! The transition set is deliberately free of winit and Vulkan types: the
//! platform glue translates the callbacks it receives into [`PlatformEvent`]s,
//! and [`PlatformLifecycle`] is the one place that decides whether a frame or a
//! background unit of work may run. A sample consults it instead of tracking
//! focus, window and activity state itself.
//!
//! Android pauses an activity without delivering a focus event, and the window
//! survives until `TERM_WINDOW`. Both are modelled separately so a paused
//! activity blocks the loop and parks its workers even while the window is
//! still alive, and a transient focus loss behaves the same way for frames
//! without tearing down the cached CPU work.

use std::time::Instant;

/// One platform lifecycle transition. The names map to the Android callbacks:
/// `WindowCreated`/`WindowDestroyed` are `INIT_WINDOW`/`TERM_WINDOW`,
/// `GainedFocus`/`LostFocus` are `APP_CMD_GAINED_FOCUS`/`APP_CMD_LOST_FOCUS`,
/// and `Paused`/`Resumed` are `APP_CMD_PAUSE`/`APP_CMD_RESUME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformEvent {
    WindowCreated,
    WindowDestroyed,
    GainedFocus,
    LostFocus,
    Paused,
    Resumed,
}

/// Whether the app owns a window, has focus, and is in a running activity.
///
/// Initial state: no window, not focused, activity running. The first
/// `WindowCreated` is treated as focused, matching every sample's long-standing
/// behavior that the initial resume is focused until a real focus-loss event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformLifecycle {
    window: bool,
    focused: bool,
    running: bool,
}

impl Default for PlatformLifecycle {
    fn default() -> Self {
        Self {
            window: false,
            focused: false,
            running: true,
        }
    }
}

impl PlatformLifecycle {
    /// Apply one transition. Idempotent and order-tolerant: a repeated paused
    /// or lost-focus event cannot change the decisions, so the Android callback
    /// order cannot matter.
    pub fn apply(&mut self, event: PlatformEvent) {
        match event {
            PlatformEvent::WindowCreated => {
                self.window = true;
                self.focused = true;
            }
            PlatformEvent::WindowDestroyed => {
                self.window = false;
                self.focused = false;
            }
            PlatformEvent::GainedFocus => self.focused = true,
            PlatformEvent::LostFocus => self.focused = false,
            PlatformEvent::Paused => self.running = false,
            PlatformEvent::Resumed => self.running = true,
        }
    }

    /// Whether an interactive frame may present and simulate.
    pub fn work_allowed(&self) -> bool {
        self.window && self.focused && self.running
    }

    /// Whether a bounded host smoke run may present without focus. The activity
    /// must still be running with a window: an explicit frame budget never
    /// overrides a real pause or a destroyed surface.
    pub fn present_allowed(&self) -> bool {
        self.window && self.running
    }

    /// Whether frames, simulation ticks and worker jobs must be stopped. A
    /// destroyed window is as final as a paused activity for this decision.
    pub fn paused(&self) -> bool {
        !self.running || !self.window
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The next-frame deadline the event loop may arm, or `None` to block on the
    /// event queue indefinitely.
    ///
    /// `frames_allowed` is the caller's complete frame policy (focus, or a
    /// bounded smoke run that presents without focus); this method adds the
    /// platform veto. A paused activity or a destroyed window always blocks, so
    /// a pacing deadline that was reached before a pause cannot become a
    /// zero-length timeout that the loop polls without ever presenting.
    pub fn frame_timeout(&self, frames_allowed: bool, next_frame: Instant) -> Option<Instant> {
        (frames_allowed && !self.paused()).then_some(next_frame)
    }
}

/// Longest wall-clock gap one frame may integrate. A pause (or a stalled frame)
/// must not advance wind, cloud drift, water animation or physics by the whole
/// gap, which would lurch the world on resume.
pub const MAX_FRAME_DELTA_S: f32 = 0.1;

/// Seconds since `previous`, clamped to [`MAX_FRAME_DELTA_S`]. Saturating, so a
/// clock that steps backwards yields zero instead of a negative step.
pub fn clamped_frame_delta(previous: Instant, now: Instant) -> f32 {
    now.saturating_duration_since(previous)
        .as_secs_f32()
        .min(MAX_FRAME_DELTA_S)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn running(on_screen: bool) -> PlatformLifecycle {
        let mut lifecycle = PlatformLifecycle::default();
        lifecycle.apply(PlatformEvent::WindowCreated);
        if !on_screen {
            lifecycle.apply(PlatformEvent::Paused);
        }
        lifecycle
    }

    #[test]
    fn default_state_has_no_window_and_does_no_work() {
        let lifecycle = PlatformLifecycle::default();
        assert!(!lifecycle.work_allowed());
        assert!(lifecycle.paused(), "no window means no work and no frames");
        assert!(!lifecycle.present_allowed());
    }

    #[test]
    fn created_window_is_focused_and_resumed() {
        let mut lifecycle = PlatformLifecycle::default();
        lifecycle.apply(PlatformEvent::WindowCreated);
        assert!(lifecycle.work_allowed());
        assert!(lifecycle.focused());
        assert!(!lifecycle.paused());
    }

    #[test]
    fn pause_blocks_frames_and_arming_a_timer_until_resume() {
        let mut lifecycle = running(true);
        let next = Instant::now() + Duration::from_millis(8);
        assert_eq!(lifecycle.frame_timeout(true, next), Some(next));

        lifecycle.apply(PlatformEvent::Paused);
        assert!(!lifecycle.work_allowed());
        assert!(lifecycle.paused());
        assert_eq!(
            lifecycle.frame_timeout(true, next),
            None,
            "a paused activity must block the event queue, not poll a deadline"
        );
        // The frame budget never overrides a pause.
        assert!(!lifecycle.present_allowed());

        lifecycle.apply(PlatformEvent::Resumed);
        assert!(lifecycle.work_allowed());
        assert_eq!(lifecycle.frame_timeout(true, next), Some(next));
    }

    #[test]
    fn focus_loss_stops_frames_but_a_smoke_budget_may_still_present() {
        let mut lifecycle = running(true);
        lifecycle.apply(PlatformEvent::LostFocus);
        assert!(!lifecycle.work_allowed());
        assert!(
            !lifecycle.paused(),
            "a focus loss is not an activity pause: workers keep their state"
        );
        assert!(lifecycle.present_allowed());
        lifecycle.apply(PlatformEvent::GainedFocus);
        assert!(lifecycle.work_allowed());
    }

    #[test]
    fn destroyed_window_stops_work_until_it_is_created_again() {
        let mut lifecycle = running(true);
        lifecycle.apply(PlatformEvent::WindowDestroyed);
        assert!(!lifecycle.work_allowed());
        assert!(lifecycle.paused());
        assert_eq!(lifecycle.frame_timeout(true, Instant::now()), None);

        // INIT_WINDOW again after TERM_WINDOW: focus is re-established with the
        // new surface and work restarts.
        lifecycle.apply(PlatformEvent::WindowCreated);
        assert!(lifecycle.work_allowed());
    }

    #[test]
    fn full_android_transition_sequence_keeps_the_pause_window_closed() {
        // init window, gained focus, lost focus, paused, resumed, term window,
        // init window again - the exact sequence the headless test drives.
        let mut lifecycle = PlatformLifecycle::default();
        lifecycle.apply(PlatformEvent::WindowCreated);
        assert!(lifecycle.work_allowed());

        lifecycle.apply(PlatformEvent::GainedFocus);
        lifecycle.apply(PlatformEvent::LostFocus);
        assert!(!lifecycle.work_allowed(), "lost focus stops frames");

        lifecycle.apply(PlatformEvent::Paused);
        assert!(lifecycle.paused());
        assert_eq!(lifecycle.frame_timeout(true, Instant::now()), None);

        lifecycle.apply(PlatformEvent::Resumed);
        assert!(
            !lifecycle.work_allowed(),
            "resume alone does not re-focus the window"
        );
        lifecycle.apply(PlatformEvent::GainedFocus);
        assert!(lifecycle.work_allowed());

        lifecycle.apply(PlatformEvent::WindowDestroyed);
        assert!(!lifecycle.work_allowed());
        lifecycle.apply(PlatformEvent::WindowCreated);
        assert!(lifecycle.work_allowed());
    }

    #[test]
    fn frame_delta_is_clamped_after_a_long_gap() {
        let start = Instant::now();
        let now = start + Duration::from_secs(600);
        assert_eq!(clamped_frame_delta(start, now), MAX_FRAME_DELTA_S);
        assert!(
            clamped_frame_delta(start, start + Duration::from_millis(16)) < 0.02,
            "a normal frame interval passes through unclamped"
        );
    }
}
