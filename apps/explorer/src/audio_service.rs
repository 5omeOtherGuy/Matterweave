//! Shared application audio adapter used by both samples.
//!
//! One [`AudioAdapter`] belongs to the application (`Experience` owns it) and is lent
//! to whichever sample is active. The wetland and Voxel Relay turn gameplay facts
//! into sound through the same [`GameplayEvent`] vocabulary and the same verified
//! `matterweave-audio` service: nothing here is sample-specific and no sample gets
//! its own audio path.
//!
//! # What the adapter owns
//!
//! * **Clip registration.** Each event has one deterministic synthetic clip (48 kHz
//!   mono f32; no asset files, no runtime synthesis per trigger). Every clip is
//!   registered exactly once per opened device and then reused through its
//!   [`ClipHandle`]. The table is small next to the service limits: six clips of 32
//!   and 175 KiB of PCM of the 4 MiB budget.
//! * **The gameplay vocabulary.** [`GameplayEvent`] is the whole vocabulary. It is
//!   plain Rust data: no `ndk`, JNI, `android_activity` or backend type appears in it
//!   or in any adapter method signature. Two places carry the service's own
//!   plain-data error enum: [`AdapterStatus::last_error`] and
//!   [`DropReason::Service`]. Neither carries a platform or backend type, so a
//!   gameplay caller still never names one.
//! * **Backpressure.** The service refuses a play when all eight voice slots are busy
//!   ([`AudioServiceError::VoiceLimit`]) or when its bounded command queue is full.
//!   Both are counted and reported through [`TriggerOutcome::Dropped`]; nothing is
//!   unwrapped, nothing panics and the caller is never blocked.
//! * **Mute, volume, suspend/resume and sample switching.** See below.
//!
//! # Backend selection is compile time, not a runtime guess
//!
//! This module contains no `cfg(target_os)` and no backend type; it calls
//! [`AudioService`], and `apps/explorer/Cargo.toml` selects what that links:
//!
//! * `backend-mock` on every non-Android target (host tests, host runs): a
//!   deterministic backend driven synchronously by the caller. It proves nothing
//!   about a real device.
//! * `backend-android` on `target_os = "android"`: real AAudio output.
//!
//! There is no runtime probe or fallback chain: a host build never links the Android
//! backend, and an Android build never links the mock.
//!
//! # Silence, degradation and observability
//!
//! The output device is opened by the first [`AudioAdapter::resume`] (the app's
//! resumed lifecycle event), never by construction, and never while suspended. A
//! device that cannot be opened, or that dies later, degrades the adapter to
//! silence: [`AudioAdapter::trigger`] then reports [`DropReason::NoDevice`], the
//! failure stays visible in [`AdapterStatus::last_error`], and the app keeps running.
//! [`AudioAdapter::poll_device`] recreates a lost stream;
//! [`AudioAdapter::resume`] retries an open that failed.
//!
//! A resume the service cannot complete (for example a stream stolen by another
//! app) stays a foreground intent instead of a silent dead end: the adapter
//! remembers that the app wants output, drops events while output is not running
//! rather than queueing them for late playback, and [`AudioAdapter::poll_device`]
//! retries the resume on a bounded cooldown until the service reports running
//! again. No second lifecycle resume is needed. Intentional suspension cancels that
//! intent, so a late poll can never restart background audio.
//!
//! Counters are not audibility. Nothing here claims that a phone produced sound:
//! host tests render the mock mixer into a buffer, which shows mixer output, not
//! device output. Nothing drives the mock mixer on a host run either, so a host run is
//! silent and its eight voice slots fill after the first eight events until something
//! renders the mock. That is the test instrument, not the adapter.
//!
//! # Mute and volume
//!
//! While muted, `trigger` starts no voice at all, and voices that are already
//! sounding are pushed to gain 0, so mute takes effect without tearing down the
//! stream or the service. `set_volume` scales sounding and subsequent voices the
//! same way. Neither ever stops or restarts the device.
//!
//! # Suspend, resume and sample switching
//!
//! `suspend` freezes the service (sounding voices continue where they stopped),
//! makes the adapter drop new events instead of queueing them, and cancels a
//! pending resume retry, so a backgrounded sample cannot leak commands or voices
//! and a late poll cannot restart output the app asked to pause. `switch_to`
//! retires every voice the leaving scope started: their stops are queued before
//! the arriving sample can trigger, and a stop the service cannot accept yet is
//! retried by the next maintenance pass instead of being forgotten.
//!
//! # Integration
//!
//! [`Experience`](crate::experience::Experience) owns exactly one adapter and pumps
//! it once per app frame: it polls the device, drains the active sample's bounded
//! [`EventQueue`] and triggers each event. Samples queue plain events; they never
//! open a device, own a service or keep a second audio path.
//!
//! A sample launched directly (`--voxel-relay`, or a test that drives a sample app
//! itself) has no audio owner. Its [`EventQueue`] is bounded and simply never
//! drained, so that path is explicitly silent instead of opening its own device.
//!
//! Mute and volume are implemented here ([`AudioAdapter::set_muted`],
//! [`AudioAdapter::set_volume`]), but no sample UI exposes them yet: the app has no
//! settings seam for them today, so nothing drives these methods in production.

use matterweave_audio::{
    AudioService, AudioServiceError, ClipHandle, ClipSpec, PlayOptions, VoiceHandle, MAX_VOICES,
};
use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

/// A gameplay fact that should be audible.
///
/// This is the entire vocabulary; both samples emit these and nothing else. The type
/// is plain data with no platform, backend or service type in it, and variants are
/// exhaustive on purpose: adding a sound means adding a variant, which the compiler
/// then requires the clip table and the intensity table to handle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GameplayEvent {
    /// A step on solid ground.
    Footstep,
    /// The player leaves the ground.
    Jump,
    /// A body lands after a fall.
    Land {
        /// Impact strength. Finite values are clamped to `0.0..=1.0`; non-finite
        /// values are treated as `1.0`.
        strength: f32,
    },
    /// A voxel was placed or removed.
    BlockEdit,
    /// A progress objective completed (pressure plate, door, exit, reward).
    Objective,
    /// A menu or UI action was confirmed.
    UiConfirm,
}

impl GameplayEvent {
    /// Every variant, for iteration in tests and diagnostics.
    ///
    /// [`GameplayEvent::Land`] appears with full strength; the payload is intensity
    /// only and never changes which clip is used. Keep this list complete when a
    /// variant is added: the coverage test checks it against the vocabulary.
    pub const ALL: [Self; 6] = [
        Self::Footstep,
        Self::Jump,
        Self::Land { strength: 1.0 },
        Self::BlockEdit,
        Self::Objective,
        Self::UiConfirm,
    ];

    /// Stable name for logs, diagnostics and test messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Footstep => "footstep",
            Self::Jump => "jump",
            Self::Land { .. } => "land",
            Self::BlockEdit => "block-edit",
            Self::Objective => "objective",
            Self::UiConfirm => "ui-confirm",
        }
    }

    /// Relative level of this event, before the master volume.
    fn gain(self) -> f32 {
        match self {
            Self::Footstep => 0.35,
            Self::Jump => 0.5,
            Self::Land { strength } => 0.7 * clamp_strength(strength),
            Self::BlockEdit => 0.5,
            Self::Objective => 0.6,
            Self::UiConfirm => 0.45,
        }
    }
}

/// `strength` clamped to `0.0..=1.0`; a non-finite strength becomes `1.0`.
fn clamp_strength(strength: f32) -> f32 {
    if strength.is_finite() {
        strength.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Capacity of one sample's gameplay-event queue.
///
/// Gameplay queues events between the input/frame path that produces them and the
/// app's once-per-frame adapter drain, so a handful of slots covers normal play;
/// the cap exists so a sample cannot grow an unbounded backlog if the app stops
/// draining (standalone launch, suspension, a stalled frame).
pub const EVENT_QUEUE_CAPACITY: usize = 16;

/// Bounded FIFO of plain gameplay events held by one sample.
///
/// A push into a full queue drops the *oldest* event and keeps the newest: the
/// most recent actions are the ones worth hearing, and the queue stays bounded
/// without ever blocking the caller. Nothing here is a service or a device; a
/// sample can queue events with no audio owner (standalone launch) and the queue
/// simply stays silent.
#[derive(Debug, Default)]
pub struct EventQueue {
    events: VecDeque<GameplayEvent>,
    dropped: u64,
}

impl EventQueue {
    /// Maximum number of queued events.
    pub const CAPACITY: usize = EVENT_QUEUE_CAPACITY;

    /// Queue `event`, dropping the oldest queued event when the queue is full.
    pub fn push(&mut self, event: GameplayEvent) {
        if self.events.len() >= Self::CAPACITY {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(event);
    }

    /// Remove and return the oldest queued event, if any.
    pub fn pop(&mut self) -> Option<GameplayEvent> {
        self.events.pop_front()
    }

    /// Drop every queued event without playing it.
    ///
    /// The app calls this when the sample loses focus, is suspended or is replaced:
    /// feedback from before a pause or a scope switch must not sound afterwards.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Number of queued events (tests and diagnostics).
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.events.len()
    }

    /// Events dropped because the queue was full (tests and diagnostics).
    #[cfg(test)]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// The application (sample) that currently owns the adapter.
///
/// The adapter serves one scope at a time. Voices are tagged with the scope that
/// started them, so [`AudioAdapter::switch_to`] can retire exactly the voices of the
/// scope being left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioScope {
    /// The wetland sample.
    Wetland,
    /// The legacy sandbox menu (`Explorer`). It triggers no events today; the variant
    /// exists so that leaving a sample for the sandbox still retires its voices.
    Sandbox,
    /// The Voxel Relay sample.
    VoxelRelay,
}

/// Why an event produced no sound.
///
/// Every reason is ordinary control flow: the caller may keep running and ignore it,
/// log it or show it. Nothing here is fatal and nothing needs to be retried by
/// gameplay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropReason {
    /// No running output: the device was never opened or its open failed, the
    /// stream is lost until [`AudioAdapter::poll_device`] recreates it, or the
    /// service still holds suspension while a refused [`AudioAdapter::resume`] is
    /// retried. Nothing is queued for later.
    NoDevice,
    /// The adapter is suspended; the event is not queued for later.
    Suspended,
    /// The adapter is muted.
    Muted,
    /// No voice slot was free: the adapter tracks at most `MAX_VOICES` voices and
    /// never steals a sounding one.
    VoiceLimit,
    /// The service's bounded command queue was full. Nothing was enqueued.
    CommandQueueFull,
    /// The service refused for another reason. Observable, never fatal.
    Service(AudioServiceError),
}

/// Result of one [`AudioAdapter::trigger`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOutcome {
    /// A voice was started for the event.
    Started,
    /// No sound was started.
    Dropped(DropReason),
}

impl TriggerOutcome {
    /// Whether a voice was started.
    #[allow(dead_code)] // exercised by the adapter tests
    pub fn started(&self) -> bool {
        matches!(self, Self::Started)
    }

    /// The drop reason, if the event produced no sound.
    pub fn dropped(&self) -> Option<&DropReason> {
        match self {
            Self::Started => None,
            Self::Dropped(reason) => Some(reason),
        }
    }
}

/// Adapter counters, monotonic for the adapter's lifetime.
///
/// Invariant: `started + dropped == triggered`, and `dropped` equals the sum of the
/// `dropped_*` counters. These count events, not audibility.
#[allow(dead_code)] // full diagnostic surface; the app reads only the failure counts
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdapterCounters {
    /// Events passed to [`AudioAdapter::trigger`].
    pub triggered: u64,
    /// Events that started a voice.
    pub started: u64,
    /// Events that produced no sound.
    pub dropped: u64,
    /// Dropped: no output device.
    pub dropped_no_device: u64,
    /// Dropped: adapter suspended.
    pub dropped_suspended: u64,
    /// Dropped: adapter muted.
    pub dropped_muted: u64,
    /// Dropped: no free voice slot.
    pub dropped_voice_limit: u64,
    /// Dropped: service command queue full.
    pub dropped_command_queue: u64,
    /// Dropped: any other service refusal.
    pub dropped_service_error: u64,
    /// Open, start, suspend or recovery failures this adapter observed.
    pub device_failures: u64,
    /// Clip registrations the service rejected (zero in a healthy adapter).
    pub registration_failures: u64,
    /// Voice stops or gain changes deferred because the command queue was full.
    /// Deferred work is retried by the next maintenance pass.
    pub cleanup_backpressure: u64,
}

/// Diagnostic snapshot of the adapter, for the HUD, logs and tests.
#[allow(dead_code)] // full diagnostic surface; the app reads only part of it today
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterStatus {
    /// True while output is open and running; false before the first resume, after
    /// a failed open, while suspended, and while a lost stream or a refused resume
    /// waits for recovery.
    pub available: bool,
    /// True while muted.
    pub muted: bool,
    /// True while suspended.
    pub suspended: bool,
    /// Master volume applied on top of each event's own level (`0.0..=1.0`).
    pub volume: f32,
    /// Scope that currently owns the adapter.
    pub scope: AudioScope,
    /// Clips registered on the open device.
    pub registered_clips: usize,
    /// PCM bytes those clips retain.
    pub registered_pcm_bytes: usize,
    /// Voices the adapter started and has not yet stopped or reclaimed. Bookkeeping
    /// only: it is not a claim that anything is audible.
    pub tracked_voices: usize,
    /// Device errors the service has observed on this adapter's output (mirrored from
    /// its health; zero without a device). A recovered error stays visible here.
    pub device_errors: u64,
    /// Completed mixer callbacks on the current device; not proof of audibility.
    pub callback_count: u64,
    /// Frames rendered on the current device; resets when that device is replaced.
    pub frames_rendered: u64,
    /// The last failure the adapter observed, or `None` if it never observed one. A
    /// record of the most recent failure, not a live error state: a later success
    /// does not clear it.
    pub last_error: Option<AudioServiceError>,
    /// Event and failure counters.
    pub counters: AdapterCounters,
}

/// One voice the adapter started that it has not yet stopped or reclaimed.
#[derive(Debug, Clone, Copy)]
struct VoiceRecord {
    handle: VoiceHandle,
    /// Scope that started this voice; `None` once a stop was requested, which marks
    /// the record for the stop retry pass.
    scope: Option<AudioScope>,
    /// The event's own gain, before the master volume. Also what unmute restores.
    base_gain: f32,
    /// Gain last pushed to the service, or `None` when a push is due (new voice, or
    /// mute/volume change).
    applied_gain: Option<f32>,
    /// Completed render invocations when the play command was queued (read after the
    /// play). A record may only be reclaimed two invocations later, which is what
    /// separates "the render thread has not seen it yet" from "it is finished".
    callback_baseline: u64,
}

/// The clips registered for one open device, one per event.
///
/// Named fields instead of an indexed table: the compiler then requires every new
/// variant to be registered explicitly, and there is no index to get out of step
/// with the vocabulary.
#[derive(Debug, Clone, Copy)]
struct Clips {
    footstep: ClipHandle,
    jump: ClipHandle,
    land: ClipHandle,
    block_edit: ClipHandle,
    objective: ClipHandle,
    ui_confirm: ClipHandle,
}

impl Clips {
    /// Number of clips in the table. Pinned against [`GameplayEvent::ALL`] by the
    /// vocabulary test: every event must be registered, and
    /// [`AdapterStatus::registered_clips`] reports exactly this when a device is open.
    const COUNT: usize = GameplayEvent::ALL.len();

    /// Register one synthesized clip per event. The caller keeps the device only if
    /// every clip registered.
    fn register(service: &mut AudioService) -> Result<Self, AudioServiceError> {
        Ok(Self {
            footstep: register_clip(service, GameplayEvent::Footstep)?,
            jump: register_clip(service, GameplayEvent::Jump)?,
            land: register_clip(service, GameplayEvent::Land { strength: 1.0 })?,
            block_edit: register_clip(service, GameplayEvent::BlockEdit)?,
            objective: register_clip(service, GameplayEvent::Objective)?,
            ui_confirm: register_clip(service, GameplayEvent::UiConfirm)?,
        })
    }

    /// The clip for `event`.
    fn for_event(&self, event: GameplayEvent) -> ClipHandle {
        match event {
            GameplayEvent::Footstep => self.footstep,
            GameplayEvent::Jump => self.jump,
            GameplayEvent::Land { .. } => self.land,
            GameplayEvent::BlockEdit => self.block_edit,
            GameplayEvent::Objective => self.objective,
            GameplayEvent::UiConfirm => self.ui_confirm,
        }
    }
}

/// An open output device with the adapter's clips already registered.
///
/// A value of this type only exists after a successful open and a complete clip
/// registration, so "has a device" and "every clip is registered" are one fact.
struct Device {
    service: AudioService,
    clips: Clips,
}

/// True while the device is open *and* the service reports output running.
///
/// Open is not running: a service that refused a resume keeps suspension even
/// after its start failed, and a poll may open a replacement that stays paused.
/// "Available" and "may start a voice" both need the service's own suspension
/// truth, not just negotiated properties.
fn device_running(device: &Device) -> bool {
    device.service.stream_properties().is_some() && !device.service.health().suspended
}

/// Wall-clock cooldown between retries of a foreground resume the service refused.
///
/// It caps a permanently failing device at two reopen/start attempts per second
/// instead of one per frame, while events keep being dropped deliberately for the
/// duration of the cooldown. The adapter reads `Instant::now()`; tests inject a clock
/// through [`AudioAdapter::now`] so the cooldown is exercised without sleeping.
const RESUME_RETRY_COOLDOWN: Duration = Duration::from_millis(500);

/// Turn gameplay events into sound through the shared audio service.
///
/// See the module documentation for the contract. Every method is control-thread
/// only, never panics, and never blocks.
pub struct AudioAdapter {
    device: Option<Device>,
    voices: [Option<VoiceRecord>; MAX_VOICES],
    scope: AudioScope,
    muted: bool,
    suspended: bool,
    /// When a refused foreground resume may be retried, or `None` when no retry is
    /// pending.
    ///
    /// Set when a `resume` (or its retry) leaves the service suspended; cleared by
    /// a confirmed resume, by intentional `suspend`, and by a failed initial open
    /// (which keeps the next-lifecycle-resume policy). Separate from `suspended`:
    /// the app is in front, but output is not running yet.
    resume_retry_at: Option<Instant>,
    volume: f32,
    last_error: Option<AudioServiceError>,
    counters: AdapterCounters,
    /// Test-only injection: the next device open fails with this error instead of
    /// opening anything. The host backend always opens successfully and the Android
    /// path is not exercised by host tests, so this is the only way to drive the
    /// failed-open branch; see `failed_open_degrades_to_silence_and_resume_retries`.
    #[cfg(test)]
    open_failure: Option<AudioServiceError>,
    /// Test-only clock override; production reads `Instant::now()` through
    /// [`AudioAdapter::now`]. Tests advance it so the resume cooldown runs without
    /// sleeping.
    #[cfg(test)]
    test_now: Option<Instant>,
}

impl AudioAdapter {
    /// Create a silent adapter for `scope`.
    ///
    /// No device is opened here: the first [`AudioAdapter::resume`] opens it, so a
    /// process that is never resumed never touches the audio device. Construction
    /// cannot fail and cannot panic.
    pub fn new(scope: AudioScope) -> Self {
        Self {
            device: None,
            voices: [None; MAX_VOICES],
            scope,
            muted: false,
            suspended: false,
            resume_retry_at: None,
            volume: 1.0,
            last_error: None,
            counters: AdapterCounters::default(),
            #[cfg(test)]
            open_failure: None,
            #[cfg(test)]
            test_now: None,
        }
    }

    /// Turn one gameplay fact into sound.
    ///
    /// Never panics, never blocks and never fails the caller: a sound that cannot be
    /// started comes back as [`TriggerOutcome::Dropped`] and is counted in
    /// [`AudioAdapter::status`]. A dropped sound effect under voice pressure is
    /// correct behaviour, not an error to propagate into gameplay.
    pub fn trigger(&mut self, event: GameplayEvent) -> TriggerOutcome {
        self.counters.triggered += 1;
        match self.start_event(event) {
            Ok(()) => {
                self.counters.started += 1;
                TriggerOutcome::Started
            }
            Err(reason) => self.drop_event(reason),
        }
    }

    /// Stop every voice the adapter is tracking, immediately and legitimately: sound
    /// effects are short, and a stop that the service cannot accept yet is retried by
    /// the next maintenance pass rather than forgotten.
    pub fn stop_all(&mut self) {
        for record in self.voices.iter_mut().flatten() {
            record.scope = None;
        }
        self.push_pending_stops();
    }

    /// Enter `scope` and retire the voices of every other scope.
    ///
    /// This is the sample-switch cleanup: the leaving sample's voices get a stop
    /// queued before the arriving sample can trigger anything, and no record or stale
    /// handle from the leaving scope survives. Calling it with the current scope
    /// changes nothing.
    pub fn switch_to(&mut self, scope: AudioScope) {
        self.scope = scope;
        for record in self.voices.iter_mut().flatten() {
            if record.scope != Some(scope) {
                record.scope = None;
            }
        }
        self.push_pending_stops();
    }

    /// Mute or unmute without touching the device.
    ///
    /// While muted no new voice is started, and sounding voices are pushed to gain 0.
    /// Unmuting restores each sounding voice's own level.
    ///
    /// No sample UI exposes mute yet, so the app does not drive this; adding the
    /// settings seam is an explicit follow-up, not an integration claim.
    #[allow(dead_code)]
    pub fn set_muted(&mut self, muted: bool) {
        if self.muted == muted {
            return;
        }
        self.muted = muted;
        self.push_pending_gains();
    }

    /// Set the master volume for sounding and subsequent voices.
    ///
    /// The value is clamped into `0.0..=1.0`. Non-finite input is ignored and the
    /// previous volume stays in effect. Volume is not mute: unlike mute it does not
    /// suppress voice starts.
    ///
    /// No sample UI exposes volume yet; see [`AudioAdapter::set_muted`].
    #[allow(dead_code)]
    pub fn set_volume(&mut self, volume: f32) {
        if !volume.is_finite() {
            return;
        }
        let volume = volume.clamp(0.0, 1.0);
        if self.volume == volume {
            return;
        }
        self.volume = volume;
        self.push_pending_gains();
    }

    /// Suspend output: the app is going to the background.
    ///
    /// The service freezes its mixer clock (sounding voices continue where they
    /// stopped on resume) and the adapter stops accepting events, so nothing is
    /// queued behind the pause. Idempotent. A suspend the device refuses is recorded
    /// and the adapter stays suspended by intent: no new events and no device opens
    /// while the app is not in front. A pending resume retry is cancelled, so a
    /// late poll can never restart output the app asked to pause.
    pub fn suspend(&mut self) {
        self.suspended = true;
        self.resume_retry_at = None;
        let Some(device) = self.device.as_mut() else {
            return;
        };
        match device.service.suspend() {
            Ok(()) => {}
            Err(AudioServiceError::CommandQueueFull) => {
                self.counters.cleanup_backpressure += 1;
            }
            Err(error) => {
                self.counters.device_failures += 1;
                self.last_error = Some(error);
            }
        }
    }

    /// Resume after [`AudioAdapter::suspend`], or after a resume that could not open
    /// the device: this is the one place that opens a device, so a failed open is
    /// retried on the next app resume and never once per frame. Idempotent while
    /// running.
    ///
    /// A resume the service refuses (open, start or command backpressure) leaves a
    /// retryable foreground intent that [`AudioAdapter::poll_device`] finishes on a
    /// bounded cooldown, so one lifecycle resume is enough. Events are dropped while
    /// the service still reports suspension.
    pub fn resume(&mut self) {
        self.suspended = false;
        self.resume_retry_at = None;
        if self.device.is_none() {
            self.open_device();
        }
        if self.device.is_some() {
            self.attempt_resume();
        }
        self.maintain();
    }

    /// Per-frame upkeep: let the service recreate a lost stream, finish a resume the
    /// service refused, then reclaim finished voices, retry deferred stops and push
    /// deferred gain changes.
    ///
    /// This never opens a device that was never opened (that is
    /// [`AudioAdapter::resume`]'s job) and does nothing while suspended: a
    /// backgrounded app must not poke the device every frame, and a late poll must
    /// not undo an intentional suspension. A pending resume retry runs at most once
    /// per `RESUME_RETRY_COOLDOWN` of wall-clock time, so a refused resume cannot
    /// close, reopen and start the device on every frame.
    pub fn poll_device(&mut self) {
        if self.suspended {
            return;
        }
        if let Some(due) = self.resume_retry_at {
            if self.now() >= due {
                self.attempt_resume();
            }
        } else if let Some(device) = self.device.as_mut() {
            match device.service.poll_device() {
                Ok(()) => {}
                Err(AudioServiceError::CommandQueueFull) => {
                    self.counters.cleanup_backpressure += 1;
                }
                Err(error) => {
                    self.counters.device_failures += 1;
                    self.last_error = Some(error);
                }
            }
        }
        self.maintain();
    }

    /// True while the adapter is suspended and refuses new events.
    ///
    /// A narrow accessor so the per-frame pump can skip the device and the drain
    /// without building a full diagnostic snapshot.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// True while output is open and running (not suspended).
    ///
    /// An open but paused stream is not usable output: this stays false after a
    /// refused resume until the bounded retry observes the service running again.
    pub fn is_available(&self) -> bool {
        self.device.as_ref().is_some_and(device_running)
    }

    /// Diagnostic snapshot (see [`AdapterStatus`]).
    pub fn status(&self) -> AdapterStatus {
        let (registered_clips, registered_pcm_bytes, device_errors) = match &self.device {
            Some(device) => (
                Clips::COUNT,
                device.service.retained_pcm_bytes(),
                device.service.health().device_errors,
            ),
            None => (0, 0, 0),
        };
        let health = self.device.as_ref().map(|device| device.service.health());
        AdapterStatus {
            callback_count: health.as_ref().map_or(0, |value| value.callback_count),
            frames_rendered: health.as_ref().map_or(0, |value| value.frames_rendered),
            available: self.is_available(),
            muted: self.muted,
            suspended: self.suspended,
            volume: self.volume,
            scope: self.scope,
            registered_clips,
            registered_pcm_bytes,
            tracked_voices: self.voices.iter().filter(|voice| voice.is_some()).count(),
            device_errors,
            last_error: self.last_error.clone(),
            counters: self.counters,
        }
    }

    // ---- device ------------------------------------------------------------

    /// Open the output device and register the clips exactly once for it.
    ///
    /// Called from [`AudioAdapter::resume`] only. Any failure leaves the adapter
    /// silent with the reason recorded; the app is never failed by audio.
    fn open_device(&mut self) {
        if self.device.is_some() {
            return;
        }
        // No device means no voice can be live and no clip handle can be current:
        // anything an earlier (dropped) service left behind is stale, not state.
        self.voices = [None; MAX_VOICES];
        #[cfg(test)]
        if let Some(error) = self.open_failure.take() {
            self.counters.device_failures += 1;
            self.last_error = Some(error);
            return;
        }
        let mut service = match AudioService::new() {
            Ok(service) => service,
            Err(error) => {
                self.counters.device_failures += 1;
                self.last_error = Some(error);
                return;
            }
        };
        match Clips::register(&mut service) {
            Ok(clips) => self.device = Some(Device { service, clips }),
            Err(error) => {
                // A half-registered device is not a usable state. Drop the service
                // (which closes the stream) and stay silent with the reason visible.
                self.counters.registration_failures += 1;
                self.counters.device_failures += 1;
                self.last_error = Some(error);
            }
        }
    }

    /// Ask the service to leave suspension and record whether output is running.
    ///
    /// The service keeps `health().suspended` true when a resume cannot open,
    /// start or enqueue its Resume command, even though the adapter already left
    /// intentional suspension. Treating that as success is the integration defect
    /// this method prevents: the adapter instead records a retryable foreground
    /// intent that [`AudioAdapter::poll_device`] finishes later.
    fn attempt_resume(&mut self) {
        let Some(device) = self.device.as_mut() else {
            self.resume_retry_at = None;
            return;
        };
        match device.service.resume() {
            Ok(()) => {}
            Err(AudioServiceError::CommandQueueFull) => {
                self.counters.cleanup_backpressure += 1;
            }
            Err(error) => {
                self.counters.device_failures += 1;
                self.last_error = Some(error);
            }
        }
        // The service's own state decides, not the result: `CommandQueueFull`
        // leaves suspension in force just as a failed start does, and only the
        // service can say output is running again.
        let still_suspended = device.service.health().suspended;
        self.resume_retry_at = if still_suspended {
            Some(self.now() + RESUME_RETRY_COOLDOWN)
        } else {
            None
        };
    }

    /// The adapter's clock, read by the resume-retry cooldown.
    ///
    /// Production reads `Instant::now()`. Tests replace it through the test-only
    /// `test_now` field so the cooldown is exercised deterministically without
    /// sleeping.
    fn now(&self) -> Instant {
        #[cfg(test)]
        {
            if let Some(now) = self.test_now {
                return now;
            }
        }
        Instant::now()
    }

    // ---- voices ------------------------------------------------------------

    /// Start a voice for `event`, or say why not.
    fn start_event(&mut self, event: GameplayEvent) -> Result<(), DropReason> {
        if self.suspended {
            return Err(DropReason::Suspended);
        }
        if self.muted {
            return Err(DropReason::Muted);
        }
        match self.device.as_ref() {
            Some(device) if device_running(device) => {}
            // No device, a lost stream that `poll_device` has to recreate, or a
            // service that still holds suspension after a refused resume: nothing
            // may be queued that cannot start now.
            _ => return Err(DropReason::NoDevice),
        }
        self.sweep_finished();
        // A full table means no service voice slot is free either: every tracked voice
        // is either sounding or too new to be confirmed finished, and the adapter
        // never steals a sounding voice to start another one.
        let Some(slot) = self.voices.iter().position(Option::is_none) else {
            return Err(DropReason::VoiceLimit);
        };
        let base_gain = event.gain();
        let gain = self.volume * base_gain;
        let scope = self.scope;
        let Some(device) = self.device.as_mut() else {
            return Err(DropReason::NoDevice);
        };
        let clip = device.clips.for_event(event);
        match device.service.play(clip, PlayOptions::with_gain(gain)) {
            Ok(handle) => {
                // Read the invocation count after the play: reclaiming a voice needs
                // two completed invocations from here on (see `voice_finished`), and a
                // later baseline only makes that test more conservative.
                let callback_baseline = device.service.health().callback_count;
                self.voices[slot] = Some(VoiceRecord {
                    handle,
                    scope: Some(scope),
                    base_gain,
                    applied_gain: Some(gain),
                    callback_baseline,
                });
                Ok(())
            }
            Err(AudioServiceError::VoiceLimit) => Err(DropReason::VoiceLimit),
            Err(AudioServiceError::CommandQueueFull) => Err(DropReason::CommandQueueFull),
            Err(error) => Err(DropReason::Service(error)),
        }
    }

    /// Count a dropped event and build the caller-facing outcome.
    fn drop_event(&mut self, reason: DropReason) -> TriggerOutcome {
        self.counters.dropped += 1;
        match &reason {
            DropReason::NoDevice => self.counters.dropped_no_device += 1,
            DropReason::Suspended => self.counters.dropped_suspended += 1,
            DropReason::Muted => self.counters.dropped_muted += 1,
            DropReason::VoiceLimit => self.counters.dropped_voice_limit += 1,
            DropReason::CommandQueueFull => self.counters.dropped_command_queue += 1,
            DropReason::Service(_) => self.counters.dropped_service_error += 1,
        }
        TriggerOutcome::Dropped(reason)
    }

    /// Whether the service confirms this voice is no longer audible.
    ///
    /// `AudioService::is_voice_active` is also false for a voice whose play command has
    /// not been applied yet, so a record is reclaimed only once at least two render
    /// invocations have completed since its play. Commands are drained at the start of
    /// a render invocation, so the second completed invocation guarantees this voice's
    /// play (or its stop) was applied before the live mask that `is_voice_active`
    /// reads. Without that wait, a burst could forget a voice that starts afterwards
    /// and a sample switch would leave it sounding.
    fn voice_finished(&self, record: &VoiceRecord) -> bool {
        let Some(device) = self.device.as_ref() else {
            return false;
        };
        device
            .service
            .health()
            .callback_count
            .saturating_sub(record.callback_baseline)
            >= 2
            && !device.service.is_voice_active(record.handle)
    }

    /// Forget records whose voices the service confirms finished.
    fn sweep_finished(&mut self) {
        for slot in 0..MAX_VOICES {
            let finished = self.voices[slot]
                .as_ref()
                .is_some_and(|record| self.voice_finished(record));
            if finished {
                self.voices[slot] = None;
            }
        }
    }

    /// Stop every record marked for retirement, keeping those the service cannot accept
    /// yet so a later pass retries them. A record leaves the table only when the
    /// service accepted (or already completed) the stop, so no stale handle is kept.
    fn push_pending_stops(&mut self) {
        let Some(device) = self.device.as_mut() else {
            return;
        };
        for slot in 0..MAX_VOICES {
            let Some(record) = self.voices[slot] else {
                continue;
            };
            if record.scope.is_some() {
                continue;
            }
            match device.service.stop_voice(record.handle) {
                Ok(()) => self.voices[slot] = None,
                // Finished or superseded: the handle no longer names a live voice, so
                // the record is done either way.
                Err(AudioServiceError::VoiceNotActive)
                | Err(AudioServiceError::StaleVoiceHandle) => {
                    self.voices[slot] = None;
                }
                // The queue is full: nothing else can be pushed now, so stop trying
                // and leave the marked records for the next maintenance pass.
                Err(AudioServiceError::CommandQueueFull) => {
                    self.counters.cleanup_backpressure += 1;
                    break;
                }
                Err(error) => {
                    self.counters.cleanup_backpressure += 1;
                    self.last_error = Some(error);
                    break;
                }
            }
        }
    }

    /// Push the gain each sounding voice should have for the current mute and volume
    /// state. Failures are retried by the next pass; a voice the service reports gone
    /// leaves the table, since that answer is authoritative.
    fn push_pending_gains(&mut self) {
        let Some(device) = self.device.as_mut() else {
            return;
        };
        let muted = self.muted;
        let volume = self.volume;
        for slot in 0..MAX_VOICES {
            let Some(record) = self.voices[slot] else {
                continue;
            };
            if record.scope.is_none() {
                continue; // retiring; its gain no longer matters
            }
            let target = if muted {
                0.0
            } else {
                volume * record.base_gain
            };
            if record.applied_gain == Some(target) {
                continue;
            }
            match device.service.set_voice_gain(record.handle, target) {
                Ok(()) => {
                    if let Some(record) = self.voices[slot].as_mut() {
                        record.applied_gain = Some(target);
                    }
                }
                Err(AudioServiceError::VoiceNotActive)
                | Err(AudioServiceError::StaleVoiceHandle) => {
                    self.voices[slot] = None;
                }
                Err(AudioServiceError::CommandQueueFull) => {
                    self.counters.cleanup_backpressure += 1;
                    break;
                }
                Err(error) => {
                    self.counters.cleanup_backpressure += 1;
                    self.last_error = Some(error);
                    break;
                }
            }
        }
    }

    /// Retry everything a previous pass had to defer.
    fn maintain(&mut self) {
        if self.device.is_none() {
            return;
        }
        self.sweep_finished();
        self.push_pending_stops();
        self.push_pending_gains();
    }
}

// ---- clip synthesis --------------------------------------------------------

/// Synthesize and register the single clip for `event`.
fn register_clip(
    service: &mut AudioService,
    event: GameplayEvent,
) -> Result<ClipHandle, AudioServiceError> {
    let samples = synth_clip(event);
    service.register_clip(ClipSpec::mono(&samples))
}

/// The clip for `event`: deterministic, mono, 48 kHz, well under a second, peak
/// amplitude below full scale. Pure function of the event, so every device and every
/// run registers exactly the same PCM.
fn synth_clip(event: GameplayEvent) -> Vec<f32> {
    match event {
        // Soft filtered-noise step.
        GameplayEvent::Footstep => noise_burst(0.055, 0.30, 0.55, 0x51ED_2701),
        // Rising tone: something was launched.
        GameplayEvent::Jump => note(0.14, 260.0, 640.0, 0.5),
        // Low thump with a contact click; strength changes the level, not the clip.
        GameplayEvent::Land { .. } => thump(0.19),
        // Short mid click: a voxel changed.
        GameplayEvent::BlockEdit => note(0.075, 720.0, 600.0, 0.45),
        // Three rising notes: progress acknowledged.
        GameplayEvent::Objective => arpeggio(&[523.25, 659.25, 783.99], 0.13, 0.45),
        // Brighter, shorter confirmation blip.
        GameplayEvent::UiConfirm => note(0.085, 940.0, 820.0, 0.4),
    }
}

/// Sample count for a duration at the service's required sample rate.
fn frame_count(seconds: f32) -> usize {
    (seconds * ClipSpec::SAMPLE_RATE as f32).round() as usize
}

/// Percussive envelope on `0.0..=1.0`: fast attack, decaying tail, exact zero at both
/// ends so concatenated clips do not click.
fn envelope(position: f32) -> f32 {
    let attack = (position / 0.08).min(1.0);
    let tail = (1.0 - position).max(0.0);
    attack * tail * tail
}

/// Sine note that glides from `from_hz` to `to_hz` under a percussive envelope.
fn note(seconds: f32, from_hz: f32, to_hz: f32, gain: f32) -> Vec<f32> {
    let count = frame_count(seconds);
    let mut phase = 0.0f32;
    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        let position = index as f32 / count as f32;
        let hz = from_hz + (to_hz - from_hz) * position;
        phase = (phase + TAU * hz / ClipSpec::SAMPLE_RATE as f32) % TAU;
        samples.push(phase.sin() * envelope(position) * gain);
    }
    samples
}

/// Filtered noise burst: a step or a contact click rather than a tone.
fn noise_burst(seconds: f32, smoothing: f32, gain: f32, seed: u32) -> Vec<f32> {
    let count = frame_count(seconds);
    let mut noise = Noise::new(seed);
    let mut filtered = 0.0f32;
    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        let position = index as f32 / count as f32;
        filtered += smoothing * (noise.next() - filtered);
        samples.push(filtered * envelope(position) * gain);
    }
    samples
}

/// Low body plus a short noise click.
fn thump(seconds: f32) -> Vec<f32> {
    let mut samples = note(seconds, 130.0, 52.0, 0.6);
    let click = noise_burst(0.02, 0.55, 0.35, 0x2F9C_1B47);
    for (sample, click_sample) in samples.iter_mut().zip(click) {
        *sample += click_sample;
    }
    samples
}

/// One note after another, each with its own envelope.
fn arpeggio(notes: &[f32], note_seconds: f32, gain: f32) -> Vec<f32> {
    let mut samples = Vec::with_capacity(frame_count(note_seconds * notes.len() as f32));
    for hz in notes {
        samples.extend(note(note_seconds, *hz, *hz, gain));
    }
    samples
}

/// Deterministic white noise in `-1.0..1.0` (xorshift32), so clips need no randomness
/// dependency and are identical on every run.
struct Noise(u32);

impl Noise {
    fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_audio::{HealthSnapshot, MAX_CLIPS, MAX_COMMANDS, MAX_PCM_BYTES};

    /// An adapter opened the way the app opens it: constructed, then resumed.
    fn adapter(scope: AudioScope) -> AudioAdapter {
        let mut adapter = AudioAdapter::new(scope);
        adapter.resume();
        assert!(adapter.is_available(), "the host mock device must open");
        adapter
    }

    /// Drive one completed mock render invocation of `frames` stereo frames and return
    /// what the mixer produced. Silence is exact zeros.
    fn render(adapter: &mut AudioAdapter, frames: usize) -> Vec<f32> {
        let device = adapter.device.as_mut().expect("device is open");
        let mut out = vec![0.0; frames * 2];
        let rendered = device
            .service
            .mock_render(&mut out)
            .expect("the mock stream is open");
        assert_eq!(rendered, frames);
        out
    }

    /// Peak absolute sample of a render window.
    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    }

    /// Saturate the service's bounded command FIFO through its public API so the
    /// next pushed command is refused with [`AudioServiceError::CommandQueueFull`].
    ///
    /// Repeatedly stopping a voice that started before the pause keeps pushing
    /// commands (a stop is not applied until a render invocation), which fills the
    /// queue deterministically: no render, no clock and no sleep. This is the real
    /// backpressure refusal path of the mock-backed service; the adapter's retry
    /// policy must handle it exactly like a refused stream start.
    fn saturate_command_queue(adapter: &mut AudioAdapter) {
        let handle = adapter
            .voices
            .iter()
            .flatten()
            .next()
            .expect("a voice must be sounding")
            .handle;
        let device = adapter.device.as_mut().expect("device");
        loop {
            match device.service.stop_voice(handle) {
                Ok(()) => {}
                Err(AudioServiceError::CommandQueueFull) => return,
                Err(error) => panic!("unexpected stop failure: {error:?}"),
            }
        }
    }

    fn health(adapter: &AudioAdapter) -> HealthSnapshot {
        adapter.device.as_ref().expect("device").service.health()
    }

    #[test]
    fn status_reports_real_completed_mix_work() {
        let mut adapter = adapter(AudioScope::Wetland);
        assert_eq!(adapter.status().callback_count, 0);
        assert_eq!(adapter.status().frames_rendered, 0);
        assert_eq!(
            adapter.trigger(GameplayEvent::BlockEdit),
            TriggerOutcome::Started
        );
        let samples = render(&mut adapter, 64);
        assert!(peak(&samples) > 0.0);
        assert_eq!(adapter.status().callback_count, 1);
        assert_eq!(adapter.status().frames_rendered, 64);
    }

    #[test]
    fn clips_register_once_and_stay_within_service_limits() {
        let mut adapter = adapter(AudioScope::Wetland);
        let opened = adapter.status();
        assert!(opened.available);
        assert_eq!(opened.registered_clips, Clips::COUNT);
        assert_eq!(opened.registered_clips, GameplayEvent::ALL.len());
        assert!(opened.registered_clips <= MAX_CLIPS);
        assert!(opened.registered_pcm_bytes > 0);
        assert!(opened.registered_pcm_bytes <= MAX_PCM_BYTES);
        assert_eq!(opened.counters.registration_failures, 0);
        assert_eq!(opened.tracked_voices, 0);
        {
            let device = adapter.device.as_ref().expect("device");
            for event in GameplayEvent::ALL {
                let clip = device.clips.for_event(event);
                assert!(
                    device.service.clip_is_registered(clip),
                    "{event:?} has no registered clip"
                );
            }
            // Every event owns a distinct clip, so the vocabulary cannot collapse
            // onto one sound by mistake.
            for (index, event) in GameplayEvent::ALL.iter().enumerate() {
                for other in &GameplayEvent::ALL[index + 1..] {
                    assert_ne!(
                        device.clips.for_event(*event),
                        device.clips.for_event(*other),
                        "{event:?} and {other:?} share a clip"
                    );
                }
            }
        }

        // Reuse: triggering every event starts a voice and registers nothing again.
        for event in GameplayEvent::ALL {
            assert_eq!(adapter.trigger(event), TriggerOutcome::Started, "{event:?}");
        }
        let reused = adapter.status();
        assert_eq!(reused.registered_clips, opened.registered_clips);
        assert_eq!(reused.registered_pcm_bytes, opened.registered_pcm_bytes);
        assert_eq!(reused.counters.started, GameplayEvent::ALL.len() as u64);
        assert_eq!(reused.counters.registration_failures, 0);
        assert_eq!(reused.tracked_voices, GameplayEvent::ALL.len());
    }

    #[test]
    fn vocabulary_is_complete_and_plain_copy_data() {
        // Compile-time: the gameplay surface is plain data, so no platform, handle or
        // backend type can be reachable through it.
        fn assert_plain<T: Copy + Send + Sync + 'static>() {}
        assert_plain::<GameplayEvent>();
        assert_plain::<AudioScope>();

        assert_eq!(GameplayEvent::ALL.len(), Clips::COUNT);
        for (index, event) in GameplayEvent::ALL.iter().enumerate() {
            assert!(!event.name().is_empty());
            for other in &GameplayEvent::ALL[index + 1..] {
                assert_ne!(event.name(), other.name(), "duplicate vocabulary entry");
            }
        }
        // Intensity is clamped into the service's gain range, including bad input.
        for strength in [
            -1.0,
            0.0,
            0.5,
            1.0,
            2.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
        ] {
            let gain = GameplayEvent::Land { strength }.gain();
            assert!(
                gain.is_finite() && (0.0..=0.7).contains(&gain),
                "strength {strength} produced gain {gain}"
            );
        }
    }

    #[test]
    fn voice_pressure_drops_events_without_losing_adapter_state() {
        let mut adapter = adapter(AudioScope::VoxelRelay);
        for _ in 0..MAX_VOICES {
            assert_eq!(
                adapter.trigger(GameplayEvent::Footstep),
                TriggerOutcome::Started
            );
        }
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Dropped(DropReason::VoiceLimit),
            "the ninth voice must be dropped, not stolen or unwrapped"
        );
        let pressured = adapter.status();
        assert_eq!(pressured.counters.started, MAX_VOICES as u64);
        assert_eq!(pressured.counters.dropped_voice_limit, 1);
        assert_eq!(pressured.counters.dropped, 1);
        assert_eq!(pressured.tracked_voices, MAX_VOICES);

        // The voices that did start render, so the drop corrupted nothing.
        assert!(peak(&render(&mut adapter, 64)) > 0.0);
        // Two completed invocations and a finished clip: the table is reclaimed and
        // pressure is transient, not a stuck state.
        render(&mut adapter, ClipSpec::SAMPLE_RATE as usize);
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Started,
            "a finished voice must free its tracking slot"
        );
        assert!(peak(&render(&mut adapter, 64)) > 0.0);
        let recovered = adapter.status();
        assert_eq!(recovered.counters.dropped_voice_limit, 1);
        assert_eq!(recovered.counters.started, MAX_VOICES as u64 + 1);
        assert_eq!(recovered.tracked_voices, 1);
    }

    #[test]
    fn mute_silences_without_tearing_down_the_device() {
        let mut adapter = adapter(AudioScope::Wetland);
        assert_eq!(
            adapter.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        assert!(peak(&render(&mut adapter, 8)) > 0.0);

        adapter.set_muted(true);
        assert!(adapter.status().muted);
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Dropped(DropReason::Muted),
            "a muted adapter must start no voice"
        );
        let muted = adapter.status();
        assert_eq!(muted.counters.dropped_muted, 1);
        assert_eq!(muted.tracked_voices, 1, "a muted event adds no record");
        // The sounding voice is pushed to gain 0 rather than left playing.
        assert_eq!(peak(&render(&mut adapter, 16)), 0.0);

        adapter.set_muted(false);
        assert!(!adapter.status().muted);
        assert!(
            peak(&render(&mut adapter, 16)) > 0.0,
            "unmute must restore the sounding voice"
        );
        assert!(adapter.is_available());
        assert_eq!(
            health(&adapter).stream_recreations,
            0,
            "mute must not close or replace the stream"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
        assert_eq!(adapter.status().counters.dropped, 1);
    }

    #[test]
    fn volume_scales_new_voices() {
        // The mock is deterministic, so the same window of the same clip must scale
        // exactly with the volume.
        let onset_peak = |volume: f32| {
            let mut adapter = adapter(AudioScope::Wetland);
            adapter.set_volume(volume);
            assert_eq!(
                adapter.trigger(GameplayEvent::Objective),
                TriggerOutcome::Started
            );
            peak(&render(&mut adapter, 64))
        };
        let loud = onset_peak(1.0);
        let quiet = onset_peak(0.25);
        assert!(loud > 0.0);
        assert!(
            (quiet - loud * 0.25).abs() < 1e-5,
            "volume must scale a new voice: {loud} -> {quiet}"
        );
    }

    #[test]
    fn volume_updates_a_sounding_voice() {
        // Same sequence twice: one adapter changes volume between windows, the other
        // does not. Comparing the second windows rules out the clip's own envelope as
        // the explanation.
        let mut adjusted = adapter(AudioScope::Wetland);
        let mut reference = adapter(AudioScope::Wetland);
        for candidate in [&mut adjusted, &mut reference] {
            assert_eq!(
                candidate.trigger(GameplayEvent::Objective),
                TriggerOutcome::Started
            );
        }
        let first_adjusted = peak(&render(&mut adjusted, 64));
        let first_reference = peak(&render(&mut reference, 64));
        assert_eq!(first_adjusted, first_reference);

        adjusted.set_volume(0.25);
        let quiet = peak(&render(&mut adjusted, 64));
        let loud = peak(&render(&mut reference, 64));
        assert!(loud > 0.0);
        assert!(
            (quiet - loud * 0.25).abs() < 1e-5,
            "volume must attenuate a sounding voice: {loud} -> {quiet}"
        );
        // Out-of-range volume is clamped; non-finite input is ignored.
        adjusted.set_volume(4.0);
        assert_eq!(adjusted.status().volume, 1.0);
        adjusted.set_volume(f32::NAN);
        assert_eq!(adjusted.status().volume, 1.0);
        // Volume 0 is silence but not mute: events still start voices.
        adjusted.set_volume(0.0);
        assert_eq!(peak(&render(&mut adjusted, 64)), 0.0);
        assert_eq!(
            adjusted.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
    }

    #[test]
    fn suspend_drops_events_and_resume_restores_playback() {
        let mut adapter = adapter(AudioScope::VoxelRelay);
        assert_eq!(
            adapter.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        adapter.suspend();
        assert!(adapter.status().suspended);
        assert!(
            health(&adapter).suspended,
            "the service itself must be paused, not just the adapter flag"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Dropped(DropReason::Suspended)
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Land { strength: 0.5 }),
            TriggerOutcome::Dropped(DropReason::Suspended)
        );
        let suspended = adapter.status();
        assert_eq!(suspended.counters.dropped_suspended, 2);
        assert_eq!(
            suspended.tracked_voices, 1,
            "events during suspension must not leak records or handles"
        );
        assert_eq!(
            peak(&render(&mut adapter, 32)),
            0.0,
            "a suspended mixer renders silence"
        );

        adapter.resume();
        assert!(!adapter.status().suspended);
        assert!(!health(&adapter).suspended);
        assert!(adapter.is_available());
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Started
        );
        assert!(
            peak(&render(&mut adapter, 32)) > 0.0,
            "events after resume must sound again"
        );
        let resumed = adapter.status();
        assert_eq!(resumed.counters.dropped, 2);
        assert_eq!(resumed.counters.started, 2);
        assert_eq!(resumed.tracked_voices, 2);
    }

    #[test]
    fn switch_to_retires_the_leaving_scope_voices() {
        let mut adapter = adapter(AudioScope::Wetland);
        for event in [
            GameplayEvent::Objective,
            GameplayEvent::Footstep,
            GameplayEvent::Jump,
        ] {
            assert_eq!(adapter.trigger(event), TriggerOutcome::Started);
        }
        let leaving: Vec<VoiceHandle> = adapter
            .voices
            .iter()
            .flatten()
            .map(|record| record.handle)
            .collect();
        assert_eq!(leaving.len(), 3);

        // No render happened yet, so the plays are still queued: the switch must
        // retire them before they can be heard.
        adapter.switch_to(AudioScope::VoxelRelay);
        let switched = adapter.status();
        assert_eq!(switched.scope, AudioScope::VoxelRelay);
        assert_eq!(switched.tracked_voices, 0, "no record may survive a switch");
        assert_eq!(
            peak(&render(&mut adapter, 256)),
            0.0,
            "voices of the leaving scope must not sound"
        );
        {
            let service = &adapter.device.as_ref().expect("device").service;
            for handle in &leaving {
                assert!(
                    !service.is_voice_active(*handle),
                    "{handle} is still live after the switch"
                );
            }
        }

        // The arriving scope works normally, and leaving it retires its voices too.
        assert_eq!(
            adapter.trigger(GameplayEvent::BlockEdit),
            TriggerOutcome::Started
        );
        assert!(peak(&render(&mut adapter, 64)) > 0.0);
        assert_eq!(
            adapter.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        adapter.switch_to(AudioScope::Sandbox);
        assert_eq!(adapter.status().tracked_voices, 0);
        assert_eq!(peak(&render(&mut adapter, 256)), 0.0);
        assert_eq!(adapter.status().scope, AudioScope::Sandbox);

        // Switching to the scope that is already current changes nothing.
        let before = adapter.status().counters;
        adapter.switch_to(AudioScope::Sandbox);
        assert_eq!(adapter.status().counters, before);
    }

    #[test]
    fn lost_device_degrades_to_silence_and_poll_recovers() {
        let mut adapter = adapter(AudioScope::VoxelRelay);
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
        // The mock's only injection point: the device error callback a real backend
        // reports on disconnect.
        adapter
            .device
            .as_mut()
            .expect("device")
            .service
            .mock_backend()
            .expect("mock backend")
            .inject_device_error(-2);
        assert!(!adapter.is_available(), "a lost stream must be observable");
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Dropped(DropReason::NoDevice)
        );
        assert_eq!(adapter.status().counters.dropped_no_device, 1);

        adapter.poll_device();
        assert!(adapter.is_available(), "polling must recreate the stream");
        let recovered = adapter.status();
        assert_eq!(
            recovered.device_errors, 1,
            "the lost device must stay visible"
        );
        assert_eq!(
            recovered.counters.device_failures, 0,
            "a recovered loss is a device error, not an adapter failure"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Started
        );
        assert!(
            peak(&render(&mut adapter, 32)) > 0.0,
            "the recreated stream must play"
        );
    }

    #[test]
    fn failed_open_degrades_to_silence_and_the_next_resume_retries() {
        let mut adapter = AudioAdapter::new(AudioScope::Wetland);
        adapter.open_failure = Some(AudioServiceError::StreamOpenFailed { code: -11 });
        adapter.resume();
        assert!(!adapter.is_available());
        let failed = adapter.status();
        assert_eq!(
            failed.last_error,
            Some(AudioServiceError::StreamOpenFailed { code: -11 })
        );
        assert_eq!(failed.counters.device_failures, 1);
        assert_eq!(failed.registered_clips, 0);
        assert_eq!(failed.tracked_voices, 0);
        // Gameplay keeps running: no panic, no failed startup, no stale state.
        assert_eq!(
            adapter.trigger(GameplayEvent::Land { strength: 1.0 }),
            TriggerOutcome::Dropped(DropReason::NoDevice)
        );
        adapter.poll_device();
        assert!(
            !adapter.is_available(),
            "polling must not open a device that was never opened"
        );
        assert_eq!(adapter.status().counters.device_failures, 1);

        // A later app resume retries the open and audio works again.
        adapter.resume();
        assert!(adapter.is_available());
        assert_eq!(adapter.status().registered_clips, GameplayEvent::ALL.len());
        assert_eq!(
            adapter.status().counters.device_failures,
            1,
            "a successful retry adds no failure"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Land { strength: 1.0 }),
            TriggerOutcome::Started
        );
        assert!(peak(&render(&mut adapter, 32)) > 0.0);
    }

    /// Regression: the real stolen-stream resume failure from the device log.
    ///
    /// `android-audio/failed-resume.log` shows this sequence: the app resumes,
    /// `AAudioStream_requestStart(s#1)` fails -899 because the stream was stolen,
    /// the service drops that stream and keeps suspension with `recovery_pending`,
    /// and a later poll opens a paused replacement that never starts. The adapter
    /// used to clear its own `suspended` flag anyway and never retried, so output
    /// stayed silent until the next lifecycle resume; a trigger could even be
    /// accepted onto the paused replacement and queue delayed playback. This drives
    /// the real failure through the mock's one-shot start refusal and checks event
    /// dropping, bounded retry and recovery from `poll_device` alone.
    #[test]
    fn stolen_stream_resume_recovers_after_bounded_retry() {
        let mut adapter = adapter(AudioScope::Wetland);
        assert_eq!(
            adapter.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        adapter.suspend();
        assert!(health(&adapter).suspended);

        // The platform refuses the next start request (AAudio -899, stolen stream).
        adapter
            .device
            .as_mut()
            .expect("device")
            .service
            .mock_backend()
            .expect("mock backend")
            .fail_next_start(-899);

        let base = Instant::now();
        adapter.test_now = Some(base);
        adapter.resume();
        assert!(
            !adapter.is_available(),
            "a failed start must not look like usable output"
        );
        assert!(
            !adapter.status().suspended,
            "the app is foreground by intent"
        );
        assert_eq!(
            adapter.status().last_error,
            Some(AudioServiceError::StreamStartFailed { code: -899 })
        );
        assert_eq!(
            adapter.status().counters.device_failures,
            1,
            "the start failure must be recorded"
        );
        {
            let device = adapter.device.as_ref().expect("device");
            assert!(
                device.service.stream_properties().is_none(),
                "the failed stream must be closed, not left paused"
            );
        }

        // Events must drop while the dead stream is not running; the cooldown poll
        // must not open a replacement early, and nothing may be queued meanwhile.
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Dropped(DropReason::NoDevice)
        );
        adapter.poll_device();
        assert!(
            !adapter.is_available(),
            "the retry must respect its cooldown"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Land { strength: 1.0 }),
            TriggerOutcome::Dropped(DropReason::NoDevice),
            "a dead stream must not accept delayed playback"
        );
        assert_eq!(
            adapter.status().tracked_voices,
            1,
            "dropped events must not leave voice records"
        );

        // The cooldown elapses: one foreground poll finishes the resume, replacing
        // the dead stream and starting it, with no second lifecycle resume.
        adapter.test_now = Some(base + RESUME_RETRY_COOLDOWN);
        adapter.poll_device();
        assert!(
            adapter.is_available(),
            "the bounded retry must recover output"
        );
        assert!(!health(&adapter).suspended);
        assert_eq!(
            health(&adapter).stream_recreations,
            1,
            "the dead stream must be replaced exactly once"
        );
        assert_eq!(
            adapter.status().counters.device_failures,
            1,
            "a successful recovery adds no failure"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
        assert!(
            peak(&render(&mut adapter, 64)) > 0.0,
            "recovered output must play"
        );
    }

    /// Regression: command backpressure during resume takes the same recovery path.
    ///
    /// A full command queue refuses the Resume command without dropping the stream,
    /// so the service stays suspended while its stream is still open. The adapter
    /// must treat that refusal exactly like a failed start: drop events, retry on
    /// the cooldown and recover without a second lifecycle resume. This variant also
    /// exercises the open-but-paused stream directly: nothing may be queued onto it
    /// even after a render frees queue space.
    #[test]
    fn refused_resume_is_retried_on_cooldown_and_drops_events_meanwhile() {
        let mut adapter = adapter(AudioScope::Wetland);
        assert_eq!(
            adapter.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        adapter.suspend();
        saturate_command_queue(&mut adapter);

        // The Resume command is refused; the service keeps its suspension, while
        // the app has already declared itself foreground again.
        let base = Instant::now();
        adapter.test_now = Some(base);
        adapter.resume();
        assert!(
            health(&adapter).suspended,
            "the service still refuses output"
        );
        assert!(
            !adapter.status().suspended,
            "the app is foreground by intent"
        );
        assert!(
            !adapter.is_available(),
            "a refused resume must not look like usable output"
        );
        let refused = adapter.status().counters;
        assert!(
            refused.cleanup_backpressure >= 1,
            "backpressure must be visible"
        );
        assert_eq!(
            refused.device_failures, 0,
            "a full queue is backpressure, not a device failure"
        );

        // Free queue space. A naive adapter would now accept plays onto the
        // still-paused service and queue delayed playback; both must be refused.
        render(&mut adapter, 8);
        let tracked = adapter.status().tracked_voices;
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Dropped(DropReason::NoDevice),
            "events must drop while output is suspended, not queue for later"
        );
        assert_eq!(
            adapter.status().tracked_voices,
            tracked,
            "a dropped event must not leave a voice record"
        );
        assert_eq!(
            peak(&render(&mut adapter, 64)),
            0.0,
            "nothing may sound while the resume is unfinished"
        );

        // Bounded retry: polls inside the cooldown leave the device alone, and the
        // poll once the deadline passes finishes the resume without a second one.
        adapter.poll_device();
        assert!(
            !adapter.is_available(),
            "the retry must respect its cooldown"
        );
        adapter.test_now = Some(base + RESUME_RETRY_COOLDOWN - Duration::from_millis(1));
        adapter.poll_device();
        assert!(
            !adapter.is_available(),
            "a poll before the deadline must not retry"
        );
        adapter.test_now = Some(base + RESUME_RETRY_COOLDOWN);
        adapter.poll_device();
        assert!(
            adapter.is_available(),
            "the poll at the deadline must finish the bounded retry"
        );
        assert!(!health(&adapter).suspended);
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
        assert!(
            peak(&render(&mut adapter, 64)) > 0.0,
            "recovered output must play"
        );
    }

    /// Regression: intentional suspension cancels a pending resume retry.
    #[test]
    fn intentional_suspend_cancels_pending_resume_retry() {
        let mut adapter = adapter(AudioScope::Wetland);
        assert_eq!(
            adapter.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        adapter.suspend();
        saturate_command_queue(&mut adapter);
        let base = Instant::now();
        adapter.test_now = Some(base);
        adapter.resume();
        assert!(health(&adapter).suspended, "resume must have failed");
        assert!(
            adapter.resume_retry_at.is_some(),
            "a refused resume must leave a retry pending"
        );

        // The app backgrounds again; the pending retry is cancelled and the queue
        // is drained, so a retry *could* succeed if one were attempted.
        adapter.suspend();
        assert!(adapter.is_suspended());
        assert!(
            adapter.resume_retry_at.is_none(),
            "intentional suspension must cancel the pending retry"
        );
        render(&mut adapter, 8);
        adapter.test_now = Some(base + RESUME_RETRY_COOLDOWN * 2);
        for _ in 0..8 {
            adapter.poll_device();
        }
        assert!(
            !adapter.is_available() && health(&adapter).suspended,
            "a late poll must never resume intentional suspension"
        );
        assert_eq!(
            adapter.trigger(GameplayEvent::Jump),
            TriggerOutcome::Dropped(DropReason::Suspended)
        );
        assert_eq!(peak(&render(&mut adapter, 64)), 0.0, "no stale playback");

        // An explicit foreground resume still works afterwards, on this adapter.
        adapter.resume();
        assert!(adapter.is_available());
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
        assert!(peak(&render(&mut adapter, 64)) > 0.0);
    }

    #[test]
    fn deferred_cleanup_is_retried_by_the_next_maintenance_pass() {
        let mut adapter = adapter(AudioScope::Wetland);
        for event in [GameplayEvent::Objective, GameplayEvent::Footstep] {
            assert_eq!(adapter.trigger(event), TriggerOutcome::Started);
        }
        // Fill the bounded command queue through the service's own API: stops of a
        // queued live voice are accepted until the FIFO is full.
        let handle = adapter.voices[0].expect("first voice").handle;
        let device = adapter.device.as_mut().expect("device");
        for _ in 0..MAX_COMMANDS {
            let _ = device.service.stop_voice(handle);
        }

        adapter.stop_all();
        let deferred = adapter.status();
        assert!(
            deferred.counters.cleanup_backpressure >= 1,
            "a full queue must defer the stop instead of dropping the record"
        );
        assert_eq!(
            deferred.tracked_voices, 2,
            "deferred records stay tracked for the retry"
        );
        render(&mut adapter, 4);
        assert!(
            adapter.status().tracked_voices > 0,
            "nothing may be retried before a maintenance pass"
        );
        adapter.poll_device();
        assert_eq!(
            adapter.status().tracked_voices,
            0,
            "the retry pass must finish the deferred cleanup"
        );
    }

    #[test]
    fn counters_account_for_every_trigger() {
        let mut adapter = adapter(AudioScope::Wetland);
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep),
            TriggerOutcome::Started
        );
        adapter.set_muted(true);
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep).dropped(),
            Some(&DropReason::Muted)
        );
        adapter.set_muted(false);
        adapter.suspend();
        assert_eq!(
            adapter.trigger(GameplayEvent::Footstep).dropped(),
            Some(&DropReason::Suspended)
        );
        adapter.resume();
        // Three events so far; fill the remaining voice slots, then overfill.
        for _ in 1..MAX_VOICES {
            assert_eq!(
                adapter.trigger(GameplayEvent::UiConfirm),
                TriggerOutcome::Started
            );
        }
        assert_eq!(
            adapter.trigger(GameplayEvent::UiConfirm).dropped(),
            Some(&DropReason::VoiceLimit)
        );

        let counters = adapter.status().counters;
        // 3 drops + MAX_VOICES started.
        assert_eq!(counters.triggered, MAX_VOICES as u64 + 3);
        assert_eq!(counters.started, MAX_VOICES as u64);
        assert_eq!(counters.started + counters.dropped, counters.triggered);
        assert_eq!(
            counters.dropped,
            counters.dropped_no_device
                + counters.dropped_suspended
                + counters.dropped_muted
                + counters.dropped_voice_limit
                + counters.dropped_command_queue
                + counters.dropped_service_error
        );
        assert_eq!(adapter.status().tracked_voices, MAX_VOICES);
    }

    #[test]
    fn event_queue_is_bounded_and_keeps_the_newest_events() {
        let mut queue = EventQueue::default();
        for _ in 0..EventQueue::CAPACITY {
            queue.push(GameplayEvent::Footstep);
        }
        assert_eq!(queue.queued(), EventQueue::CAPACITY);
        assert_eq!(queue.dropped(), 0);

        // One push past capacity drops the oldest event and keeps the newest.
        queue.push(GameplayEvent::Objective);
        assert_eq!(queue.queued(), EventQueue::CAPACITY);
        assert_eq!(queue.dropped(), 1);
        let mut events = Vec::new();
        while let Some(event) = queue.pop() {
            events.push(event);
        }
        assert!(
            events.contains(&GameplayEvent::Objective),
            "the newest event must survive"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == GameplayEvent::Footstep)
                .count(),
            EventQueue::CAPACITY - 1
        );

        // Clearing drops the queue without affecting the drop counter.
        queue.push(GameplayEvent::Jump);
        queue.clear();
        assert_eq!(queue.queued(), 0);
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.dropped(), 1);
    }
}
