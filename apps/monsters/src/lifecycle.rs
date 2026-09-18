//! Platform lifecycle state for the Mossbound host, restating the explorer's
//! shared pattern without depending on the explorer crate. Focus, window and
//! activity state are tracked as typed transitions so the Android callback
//! order cannot change whether a frame or a step may run.

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
