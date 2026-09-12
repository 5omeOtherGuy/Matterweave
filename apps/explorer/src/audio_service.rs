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
//!   or in any adapter method signature. The only service type anywhere on the
//!   surface is the plain-data error enum in [`AdapterStatus::last_error`].
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
//! `suspend` freezes the service (sounding voices continue where they stopped) and
//! makes the adapter drop new events instead of queueing them, so a backgrounded
//! sample cannot leak commands or voices. `switch_to` retires every voice the
//! leaving scope started: their stops are queued before the arriving sample can
//! trigger, and a stop the service cannot accept yet is retried by the next
//! maintenance pass instead of being forgotten.
//!
//! # Not wired yet
//!
//! This is the D4.1 adapter only. Wiring `experience.rs` and the samples' call sites
//! is the lead's follow-up; the temporary `#![allow(dead_code)]` below keeps strict
//! clippy (`-D warnings`) green until that commit lands and should be removed with it.

#![allow(dead_code)] // D4.1 integration follow-up; see "Not wired yet" above.

use matterweave_audio::{
    AudioService, AudioServiceError, ClipHandle, ClipSpec, PlayOptions, VoiceHandle, MAX_VOICES,
};
use std::f32::consts::TAU;

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
    /// No output device is open: the open failed, or the stream is lost until
    /// [`AudioAdapter::poll_device`] recreates it (or [`AudioAdapter::resume`]
    /// retries the open).
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
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterStatus {
    /// True while an output stream is open; false before the first resume, after a
    /// failed open, and while a lost stream waits for recovery.
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
    volume: f32,
    last_error: Option<AudioServiceError>,
    counters: AdapterCounters,
    /// Test-only injection: the next device open fails with this error instead of
    /// opening anything. The host backend always opens successfully and the Android
    /// path is not exercised by host tests, so this is the only way to drive the
    /// failed-open branch; see `failed_open_degrades_to_silence_and_resume_retries`.
    #[cfg(test)]
    open_failure: Option<AudioServiceError>,
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
            volume: 1.0,
            last_error: None,
            counters: AdapterCounters::default(),
            #[cfg(test)]
            open_failure: None,
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
    /// while the app is not in front.
    pub fn suspend(&mut self) {
        self.suspended = true;
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
    pub fn resume(&mut self) {
        self.suspended = false;
        if self.device.is_none() {
            self.open_device();
        }
        if let Some(device) = self.device.as_mut() {
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
        }
        self.maintain();
    }

    /// Per-frame upkeep: let the service recreate a lost stream, then reclaim finished
    /// voices, retry deferred stops and push deferred gain changes.
    ///
    /// This never opens a device that was never opened (that is
    /// [`AudioAdapter::resume`]'s job) and does nothing while suspended: a
    /// backgrounded app must not poke the device every frame.
    pub fn poll_device(&mut self) {
        if self.suspended {
            return;
        }
        if let Some(device) = self.device.as_mut() {
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

    /// True while an output stream is open.
    pub fn is_available(&self) -> bool {
        self.device
            .as_ref()
            .is_some_and(|device| device.service.stream_properties().is_some())
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
        AdapterStatus {
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
            Some(device) if device.service.stream_properties().is_some() => {}
            // No device at all, or a lost stream that `poll_device` has to recreate.
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

    fn health(adapter: &AudioAdapter) -> HealthSnapshot {
        adapter.device.as_ref().expect("device").service.health()
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
}
