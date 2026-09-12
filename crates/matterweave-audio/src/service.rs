//! The game-facing audio service.
//!
//! [`AudioService`] is the single owner of the mixer core, the command producer, the
//! handle mirrors and the output backend. It is designed for one controlling thread
//! (`&mut self` API) and never exposes backend or real-time primitives.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::backend::{self, BackendError, CoreOwner, CorePtr, OutputBackend};
use crate::clip::ClipSpec;
use crate::command::{queue, Command};
use crate::config::{MAX_CLIPS, MAX_GAIN, MAX_PCM_BYTES, MAX_VOICES, SAMPLE_RATE};
use crate::error::AudioServiceError;
use crate::handle::{ClipHandle, VoiceHandle};
use crate::mixer::{MixerCore, SharedRt};
use crate::pcm::PcmPool;

#[cfg(all(test, feature = "backend-mock"))]
mod tests;

/// Negotiated output stream properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamProperties {
    /// Actual sample rate in Hz.
    pub sample_rate: u32,
    /// Actual channel count (1 or 2 after verification).
    pub channels: u8,
    /// True iff the stream format is 32-bit float.
    pub float_format: bool,
    /// Device burst size in frames.
    pub frames_per_burst: u32,
    /// Total buffer capacity in frames.
    pub buffer_capacity_frames: u32,
    /// Fixed callback batch size, if the device provides one.
    pub frames_per_data_callback: Option<u32>,
    /// Platform device id (negative = unspecified/mock).
    pub device_id: i32,
    /// Platform session id, if allocated.
    pub session_id: Option<i32>,
    /// True iff low-latency performance mode was granted.
    pub low_latency: bool,
    /// True iff the stream is exclusive (not mixed by the platform).
    pub sharing_exclusive: bool,
}

/// Options for starting a voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayOptions {
    /// Linear gain applied to every sample of the voice. Must be finite and within
    /// ±[`MAX_GAIN`].
    pub gain: f32,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self { gain: 1.0 }
    }
}

impl PlayOptions {
    /// Options with an explicit gain.
    pub fn with_gain(gain: f32) -> Self {
        Self { gain }
    }
}

/// Operational counters and diagnostics.
///
/// Counters are monotonic for the service lifetime. RT-side counters reflect the
/// last completed render invocation; control-side counters are exact at return time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthSnapshot {
    /// Completed render-callback invocations.
    pub callback_count: u64,
    /// Frames handed to the output device.
    pub frames_rendered: u64,
    /// Commands applied through the last completed render invocation.
    pub commands_applied: u64,
    /// Commands pushed but not yet acknowledged by a completed callback. Includes
    /// the in-flight batch as well as commands still in the bounded FIFO.
    pub pending_commands: usize,
    /// Commands rejected because the queue was full (backpressure, never overwrite).
    pub rejected_commands: u64,
    /// Playback requests rejected because all voices were active.
    pub rejected_voice_starts: u64,
    /// Clip registrations rejected (limits, budget, invalid data, full queue).
    pub rejected_registrations: u64,
    /// Voices silenced because their clip was unregistered.
    pub voices_silenced: u64,
    /// Voices that finished playing their clip.
    pub voices_completed: u64,
    /// Commands rejected by the render thread (generation mismatch). Expected zero.
    pub rt_rejected_commands: u64,
    /// Device-reported underruns/overruns for the current stream.
    pub device_xruns: u32,
    /// Device errors observed since creation (each normally followed by recreation).
    pub device_errors: u64,
    /// Times the output stream was closed and reopened (device loss, diagnostic).
    pub stream_recreations: u64,
    /// Render callback epoch (advances once per completed invocation). Diagnostic
    /// only: it does not acknowledge commands queued after that invocation's drain.
    pub ack_epoch: u64,
    /// Service suspension truth: `true` after a successful [`AudioService::suspend`]
    /// and `false` again after a successful [`AudioService::resume`]. Preserved
    /// across recreation and failed recovery, even while no stream is open. This
    /// reports service intent, not synchronous device quiescence or PCM progress.
    pub suspended: bool,
    /// Render thread's own suspended view as of its last completed invocation.
    /// Unlike [`HealthSnapshot::suspended`] it changes only when a render pass runs,
    /// so it may stay `false` while paused. An in-flight callback may still apply
    /// Suspend after an asynchronous device pause request.
    pub rt_suspended: bool,
}

/// Control-side mirror of one clip slot.
#[derive(Debug, Clone, Copy)]
struct ClipMirror {
    generation: u32,
    active: bool,
    /// First pool sample.
    start: u32,
    /// Length in samples.
    samples: u32,
    /// 1 or 2.
    channels: u8,
}

impl ClipMirror {
    const fn empty() -> Self {
        Self {
            generation: 0,
            active: false,
            start: 0,
            samples: 0,
            channels: 1,
        }
    }
}

/// Control-side mirror of one voice slot.
#[derive(Debug, Clone, Copy)]
struct VoiceMirror {
    generation: u32,
    /// Whether the control thread believes the voice is active (the RT live mask is
    /// the authoritative truth once the play command was applied).
    live: bool,
    /// Clip slot the voice plays from.
    clip_slot: u8,
    /// FIFO sequence of this voice's Play. Until the completed command count
    /// reaches it, the live mask may belong to an older generation or callback.
    play_sequence: u64,
}

impl VoiceMirror {
    const fn empty() -> Self {
        Self {
            generation: 0,
            live: false,
            clip_slot: 0,
            play_sequence: 0,
        }
    }
}

/// A PCM range in the clip pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PoolRange {
    start: u32,
    samples: u32,
}

/// The bounded audio service.
///
/// One instance owns one output stream and one mixer core. All operations validate
/// completely before mutating clip/voice state. Device open/start failures instead
/// leave output closed and retryable, preserving suspension and queued commands.
///
/// # Drop order
///
/// The backend (owning the stream and the callback closures) is dropped before the
/// mixer core. AAudio closes the stream when the stream object drops and joins all
/// callback threads before returning, so no callback can observe the freed core.
pub struct AudioService {
    backend: Option<Box<dyn OutputBackend>>,
    // Field order matters: backend closes first, then core, then the service's
    // independent PCM owner. CoreOwner never dereferences a live mixer.
    core: CoreOwner,
    pool: Arc<PcmPool>,
    shared: Arc<SharedRt>,
    producer: queue::Producer,

    clip_mirror: [ClipMirror; MAX_CLIPS],
    voice_mirror: [VoiceMirror; MAX_VOICES],
    free_ranges: Vec<PoolRange>,
    /// Retired PCM ranges paired with the FIFO sequence of their Unload command.
    /// Reuse requires a completed callback acknowledging that command, not merely
    /// a callback that happened to finish after enqueue.
    pending_ranges: Vec<(PoolRange, u64)>,
    /// Sum of registered clip samples (the "retained PCM payload").
    used_samples: usize,
    running: bool,
    /// Successful pause intent survives output loss until resume succeeds.
    suspended: bool,
    /// A consumed device error must remain retryable after open/start failure.
    recovery_pending: bool,
    /// Queue Resume at most once across failed starts (no retry queue saturation).
    resume_queued: bool,

    commands_pushed: u64,
    rejected_commands: u64,
    rejected_voice_starts: u64,
    rejected_registrations: u64,
    device_errors: u64,
    stream_recreations: u64,
}

// Send is derived: OutputBackend requires Send, CoreOwner transfers raw ownership
// without accessing the core, and PcmPool's unsafe cell access contract gates PCM.

impl AudioService {
    /// Create the service and open the platform output stream.
    ///
    /// Allocates the fixed PCM pool (4 MiB), the bounded command queue (64 commands)
    /// and the mixer core, then opens and starts the output stream. The stream
    /// properties are verified after open.
    pub fn new() -> Result<Self, AudioServiceError> {
        let pool = Arc::new(PcmPool::new(MAX_PCM_BYTES / 4));
        let shared = Arc::new(SharedRt::default());
        let (core, producer) = MixerCore::new(pool.clone(), shared.clone());
        Self::assemble(Box::new(core), pool, producer, shared)
    }

    fn assemble(
        core: Box<MixerCore>,
        pool: Arc<PcmPool>,
        producer: queue::Producer,
        shared: Arc<SharedRt>,
    ) -> Result<Self, AudioServiceError> {
        let mut free_ranges = Vec::with_capacity(256);
        free_ranges.push(PoolRange {
            start: 0,
            samples: pool.len() as u32,
        });
        let mut service = Self {
            backend: None,
            core: CoreOwner::new(core),
            pool,
            shared,
            producer,
            clip_mirror: core::array::from_fn(|_| ClipMirror::empty()),
            voice_mirror: core::array::from_fn(|_| VoiceMirror::empty()),
            free_ranges,
            pending_ranges: Vec::with_capacity(256),
            used_samples: 0,
            running: false,
            suspended: false,
            recovery_pending: false,
            resume_queued: false,
            commands_pushed: 0,
            rejected_commands: 0,
            rejected_voice_starts: 0,
            rejected_registrations: 0,
            device_errors: 0,
            stream_recreations: 0,
        };
        service.open_output()?;
        service.start_output()?;
        Ok(service)
    }

    fn open_output(&mut self) -> Result<(), AudioServiceError> {
        let backend = backend::open_backend(self.core.ptr(), self.shared.clone())?;
        self.backend = Some(backend);
        Ok(())
    }

    fn start_output(&mut self) -> Result<(), AudioServiceError> {
        let backend = self
            .backend
            .as_mut()
            .ok_or(AudioServiceError::StreamStartFailed { code: 0 })?;
        if let Err(error) = backend.start() {
            // A failed start is not a running stream. Drop closes/joins callbacks
            // before retrying around the same core; preserve pause/Resume intent.
            let _ = backend.close();
            self.backend = None;
            self.running = false;
            self.recovery_pending = true;
            return Err(error);
        }
        self.running = true;
        self.suspended = false;
        self.resume_queued = false;
        Ok(())
    }

    /// Register a clip. Validates completely, then copies the samples into the pool.
    ///
    /// Rejections are atomic: the clip limit (32), the PCM budget (4 MiB), invalid
    /// data and a full command queue all leave the service unchanged.
    pub fn register_clip(&mut self, spec: ClipSpec<'_>) -> Result<ClipHandle, AudioServiceError> {
        let validated = match spec.validate() {
            Ok(validated) => validated,
            Err(e) => {
                self.rejected_registrations += 1;
                return Err(e);
            }
        };
        let slot = match self.clip_mirror.iter().position(|mirror| !mirror.active) {
            Some(slot) => slot,
            None => {
                self.rejected_registrations += 1;
                return Err(AudioServiceError::ClipLimit);
            }
        };
        if queue::vacant(&self.producer) == 0 {
            self.rejected_registrations += 1;
            self.rejected_commands += 1;
            return Err(AudioServiceError::CommandQueueFull);
        }
        // First-fit range search (deterministic). A range is only reused after the
        // audio thread's completed command count acknowledged the unload, so no
        // voice can still reference it.
        let range = match self.take_free_range(spec.samples.len()) {
            Ok(range) => range,
            Err(e) => {
                self.rejected_registrations += 1;
                return Err(e);
            }
        };
        let generation = next_generation(self.clip_mirror[slot].generation);
        let start = range.start;
        let samples = spec.samples.len();
        // SAFETY: take_free_range grants an unpublished range, or one whose
        // completed Unload acknowledgment was acquired. No callback can read it.
        // The external source cannot alias our private pool. Publish only after
        // this write; neither core nor its Box is borrowed by the control thread.
        unsafe { self.pool.write(start as usize, spec.samples) };
        self.used_samples += samples;

        self.push(Command::LoadClip {
            slot: slot as u8,
            generation,
        })
        .expect("queue space checked above");

        self.clip_mirror[slot] = ClipMirror {
            generation,
            active: true,
            start,
            samples: samples as u32,
            channels: validated.channels,
        };
        Ok(ClipHandle {
            slot: slot as u8,
            generation,
        })
    }

    /// Unregister a clip. Every voice still playing it is silenced (their handles
    /// become inactive; `stop_voice` on them stays a successful no-op).
    ///
    /// The freed PCM range becomes reusable only after the render thread applied the
    /// unload command and finished that callback, which makes reusing pool memory
    /// safe without locks. If the render thread is suspended, freed ranges stay
    /// pending until resume; a registration needing that memory then returns
    /// [`AudioServiceError::RangeNotYetAcknowledged`].
    pub fn unregister_clip(&mut self, clip: ClipHandle) -> Result<(), AudioServiceError> {
        let slot = self.validate_clip(clip)?;
        let generation = next_generation(clip.generation);
        if queue::vacant(&self.producer) == 0 {
            self.rejected_commands += 1;
            return Err(AudioServiceError::CommandQueueFull);
        }
        self.push(Command::UnloadClip {
            slot: clip.slot,
            generation: clip.generation,
        })
        .expect("queue space checked above");
        let range = PoolRange {
            start: self.clip_mirror[slot].start,
            samples: self.clip_mirror[slot].samples,
        };
        let clip_slot = clip.slot;
        self.clip_mirror[slot] = ClipMirror {
            generation,
            ..ClipMirror::empty()
        };
        self.used_samples -= range.samples as usize;
        // Retire the voices pointing at this clip on the control side; the render
        // thread silences them when it applies the command.
        for voice in self.voice_mirror.iter_mut() {
            if voice.clip_slot == clip_slot {
                voice.live = false;
            }
        }
        // This producer owns the FIFO sequence. A callback completing after this
        // enqueue might already have drained; only this command's ack permits reuse.
        self.pending_ranges.push((range, self.commands_pushed));
        Ok(())
    }

    /// Start a new voice playing `clip`. Voices are never stolen: if all 8 slots are
    /// active, [`AudioServiceError::VoiceLimit`] is returned and nothing changes.
    ///
    /// Capacity reflects render-thread applied state: a play immediately after a
    /// stop (or after a previous play whose slot is being reused) may be rejected
    /// with `VoiceLimit` until the next callback applies the pending commands.
    /// Retry after the next callback on a live device.
    pub fn play(
        &mut self,
        clip: ClipHandle,
        options: PlayOptions,
    ) -> Result<VoiceHandle, AudioServiceError> {
        if !options.gain.is_finite() || options.gain.abs() > MAX_GAIN {
            return Err(AudioServiceError::InvalidGain);
        }
        let clip_slot = self.validate_clip(clip)?;
        let voice_slot = match self.find_free_voice_slot() {
            Some(slot) => slot,
            None => {
                self.rejected_voice_starts += 1;
                return Err(AudioServiceError::VoiceLimit);
            }
        };
        if queue::vacant(&self.producer) == 0 {
            self.rejected_commands += 1;
            return Err(AudioServiceError::CommandQueueFull);
        }
        let clip_mirror = self.clip_mirror[clip_slot];
        let voice_generation = next_generation(self.voice_mirror[voice_slot].generation);
        self.push(Command::Play {
            voice_slot: voice_slot as u8,
            voice_generation,
            clip_slot: clip.slot,
            clip_generation: clip.generation,
            start: clip_mirror.start,
            frames: clip_mirror.samples / clip_mirror.channels as u32,
            clip_channels: clip_mirror.channels,
            gain: options.gain,
        })
        .expect("queue space checked above");
        // The completed command sequence (not the callback epoch) determines when
        // the RT live mask can describe this generation.
        let play_sequence = self.commands_pushed;
        self.voice_mirror[voice_slot] = VoiceMirror {
            generation: voice_generation,
            live: true,
            clip_slot: clip.slot,
            play_sequence,
        };
        Ok(VoiceHandle {
            slot: voice_slot as u8,
            generation: voice_generation,
        })
    }

    /// Stop a voice. Stopping an already-finished voice is a successful no-op; a
    /// stale handle is rejected and cannot affect the voice that now owns the slot.
    pub fn stop_voice(&mut self, voice: VoiceHandle) -> Result<(), AudioServiceError> {
        let slot = self.validate_voice(voice)?;
        if self.voice_definitely_finished(slot) {
            self.voice_mirror[slot].live = false;
            return Ok(());
        }
        if queue::vacant(&self.producer) == 0 {
            self.rejected_commands += 1;
            return Err(AudioServiceError::CommandQueueFull);
        }
        self.push(Command::Stop {
            voice_slot: slot as u8,
            voice_generation: voice.generation,
        })
        .expect("queue space checked above");
        Ok(())
    }

    /// Change a voice's gain. The voice must still be active.
    pub fn set_voice_gain(
        &mut self,
        voice: VoiceHandle,
        gain: f32,
    ) -> Result<(), AudioServiceError> {
        if !gain.is_finite() || gain.abs() > MAX_GAIN {
            return Err(AudioServiceError::InvalidGain);
        }
        let slot = self.validate_voice(voice)?;
        if self.voice_definitely_finished(slot) {
            return Err(AudioServiceError::VoiceNotActive);
        }
        if queue::vacant(&self.producer) == 0 {
            self.rejected_commands += 1;
            return Err(AudioServiceError::CommandQueueFull);
        }
        self.push(Command::SetGain {
            voice_slot: slot as u8,
            voice_generation: voice.generation,
            gain,
        })
        .expect("queue space checked above");
        Ok(())
    }

    /// Suspend output: freeze the mixer clock mid-sample and silence the device.
    ///
    /// On success [`AudioService::health`] reports `suspended == true` immediately;
    /// no render invocation is required (a paused AAudio stream delivers none).
    ///
    /// Nothing is dropped or restarted: voices active at suspend continue exactly
    /// where they stopped on resume, and commands queued during suspension are
    /// applied in FIFO order when the mixer resumes (a queued play that had not yet
    /// started begins after resume). Idempotent while already suspended.
    pub fn suspend(&mut self) -> Result<(), AudioServiceError> {
        if self.suspended {
            return Ok(());
        }
        // A failed recovery may have no running stream. Still record pause
        // intent and queue Suspend so a later poll cannot restart background audio.
        // Reserve both Suspend and its compensating Resume before changing any
        // state. Only this thread produces commands; the consumer can only free
        // capacity, so compensation remains possible even if pause fails.
        if queue::vacant(&self.producer) < 2 {
            self.rejected_commands += 1;
            return Err(AudioServiceError::CommandQueueFull);
        }
        self.push(Command::Suspend)
            .expect("queue space checked above");
        // Pause the device after the mixer command is queued; in-flight callbacks
        // render silence because the core is suspended first. If the device pause
        // fails, compensate by un-suspending the core so device and mixer agree.
        if let Some(backend) = self.backend.as_mut() {
            if let Err(device_error) = backend.suspend() {
                self.push(Command::Resume)
                    .expect("compensation slot reserved before Suspend");
                return Err(device_error);
            }
        }
        self.running = false;
        self.suspended = true;
        self.resume_queued = false;
        Ok(())
    }

    /// Resume output after [`AudioService::suspend`]. Idempotent while running.
    /// Clears `suspended` when the start request succeeds; output progress is
    /// observed separately through callback/frame counters. Failed open/start is
    /// retryable by calling resume again without duplicating the Resume command.
    pub fn resume(&mut self) -> Result<(), AudioServiceError> {
        if self.running {
            return Ok(());
        }
        if self.recovery_pending || self.backend.is_none() {
            self.recreate_output()?;
            if self.running {
                return Ok(());
            }
        }
        // Queue Resume before starting so even a buffered Suspend is applied
        // before Resume in the first callback. Retain it across failed starts.
        if !self.resume_queued {
            self.push(Command::Resume)?;
            self.resume_queued = true;
        }
        self.start_output()?;
        Ok(())
    }

    /// Check for device loss and recreate the output stream around the same mixer
    /// core. Voices continue where they stopped. Call this from the game loop; after
    /// a device loss [`AudioService::stream_properties`] returns `None` until a
    /// successful recreation. Suspension survives recreation: the replacement is
    /// opened but not started until resume. Failed open/start is retried by later
    /// polls; a failed resume still requires resume again to leave suspension.
    pub fn poll_device(&mut self) -> Result<(), AudioServiceError> {
        let error = self
            .backend
            .as_mut()
            .and_then(|backend| backend.take_error());
        if let Some(BackendError { code: _code }) = error {
            self.device_errors += 1;
            self.recovery_pending = true;
        }
        if self.recovery_pending {
            self.recreate_output()?;
        }
        Ok(())
    }

    /// Last negotiated stream properties, or `None` while the stream is closed or
    /// lost.
    pub fn stream_properties(&self) -> Option<StreamProperties> {
        self.backend
            .as_ref()
            .and_then(|backend| backend.properties())
    }

    /// Whether a clip is registered with this exact handle.
    pub fn clip_is_registered(&self, clip: ClipHandle) -> bool {
        let slot = clip.slot as usize;
        slot < MAX_CLIPS
            && self.clip_mirror[slot].active
            && self.clip_mirror[slot].generation == clip.generation
    }

    /// Whether a voice is currently audible (generation-checked).
    ///
    /// On a device the answer can lag one render callback behind reality.
    pub fn is_voice_active(&self, voice: VoiceHandle) -> bool {
        let slot = voice.slot as usize;
        slot < MAX_VOICES
            && self.voice_mirror[slot].generation == voice.generation
            && self.voice_mirror[slot].live
            && self.shared.commands_applied.load(Ordering::Acquire)
                >= self.voice_mirror[slot].play_sequence
            && self.rt_voice_live(slot)
    }

    /// Operational counters (see [`HealthSnapshot`]).
    pub fn health(&self) -> HealthSnapshot {
        let xruns = self.backend.as_ref().map(|b| b.xrun_count()).unwrap_or(0);
        let ack = self.shared.ack_epoch.load(Ordering::Acquire);
        let applied = self.shared.commands_applied.load(Ordering::Acquire);
        HealthSnapshot {
            callback_count: self.shared.callback_count.load(Ordering::Acquire),
            frames_rendered: self.shared.frames_rendered.load(Ordering::Acquire),
            commands_applied: applied,
            pending_commands: self.commands_pushed.saturating_sub(applied) as usize,
            rejected_commands: self.rejected_commands,
            rejected_voice_starts: self.rejected_voice_starts,
            rejected_registrations: self.rejected_registrations,
            voices_silenced: self.shared.voices_silenced.load(Ordering::Acquire),
            voices_completed: self.shared.voices_completed.load(Ordering::Acquire),
            rt_rejected_commands: self.shared.rt_rejected_commands.load(Ordering::Acquire),
            device_xruns: xruns,
            device_errors: self.device_errors,
            stream_recreations: self.stream_recreations,
            ack_epoch: ack,
            suspended: self.suspended,
            rt_suspended: self.shared.suspended.load(Ordering::Acquire),
        }
    }

    /// Total bytes of PCM currently retained by registered clips.
    pub fn retained_pcm_bytes(&self) -> usize {
        self.used_samples * 4
    }

    /// The required output sample rate.
    pub const SAMPLE_RATE: u32 = SAMPLE_RATE;

    /// Typed access to the mock backend, if the current backend is one (test and
    /// diagnostic fault injection).
    #[cfg(feature = "backend-mock")]
    pub fn mock_backend(&mut self) -> Option<&mut crate::backend::mock::MockOutput> {
        self.backend.as_mut().and_then(|backend| backend.as_mock())
    }

    /// Drive one render invocation synchronously through the mock backend (host
    /// tests and the host diagnostic). Returns frames rendered.
    #[cfg(feature = "backend-mock")]
    pub fn mock_render(
        &mut self,
        out: &mut [f32],
    ) -> Result<usize, crate::backend::mock::MockRenderError> {
        match self.backend.as_mut().and_then(|backend| backend.as_mock()) {
            Some(mock) => mock.render(out),
            // Not a mock (e.g. Android build with real output enabled).
            None => Err(crate::backend::mock::MockRenderError::Closed),
        }
    }

    // ---- internals ---------------------------------------------------------

    fn recreate_output(&mut self) -> Result<(), AudioServiceError> {
        self.recreate_output_with(backend::open_backend)
    }

    // Reuse OutputBackend; injecting only the opener lets unit tests exercise
    // open/start failures without a public factory API or callback-side seam.
    fn recreate_output_with(
        &mut self,
        open: impl FnOnce(CorePtr, Arc<SharedRt>) -> Result<Box<dyn OutputBackend>, AudioServiceError>,
    ) -> Result<(), AudioServiceError> {
        self.recovery_pending = true;
        self.running = false;
        if let Some(backend) = self.backend.as_mut() {
            backend.close()?;
        }
        self.backend = None;
        // Old callbacks are now joined. Clear their notifications before open:
        // clearing after open/start could erase a new stream's error callback.
        self.shared.disconnected.store(false, Ordering::Relaxed);
        self.shared.error_code.store(0, Ordering::Relaxed);
        self.backend = Some(open(self.core.ptr(), self.shared.clone())?);
        // Opening is not starting: a suspended replacement receives no callbacks.
        // The retained core and its FIFO may contain either an applied or buffered
        // Suspend. Only resume() may arrange Resume and restart output.
        if !self.suspended {
            self.start_output()?;
        }
        self.recovery_pending = false;
        self.stream_recreations += 1;
        Ok(())
    }

    fn validate_clip(&self, clip: ClipHandle) -> Result<usize, AudioServiceError> {
        let slot = clip.slot as usize;
        if slot >= MAX_CLIPS {
            return Err(AudioServiceError::StaleClipHandle);
        }
        let mirror = &self.clip_mirror[slot];
        if !mirror.active || mirror.generation != clip.generation {
            return Err(AudioServiceError::StaleClipHandle);
        }
        Ok(slot)
    }

    fn validate_voice(&self, voice: VoiceHandle) -> Result<usize, AudioServiceError> {
        let slot = voice.slot as usize;
        if slot >= MAX_VOICES || self.voice_mirror[slot].generation != voice.generation {
            return Err(AudioServiceError::StaleVoiceHandle);
        }
        Ok(slot)
    }

    /// RT truth for voice liveness (as of the last completed invocation).
    fn rt_voice_live(&self, slot: usize) -> bool {
        self.shared.live_mask.load(Ordering::Acquire) & (1 << slot) != 0
    }

    /// First voice slot that is either unused on the control side or already
    /// reported inactive by the render thread (completion). Reuse bumps the
    /// generation, which invalidates the old handle.
    fn find_free_voice_slot(&self) -> Option<usize> {
        let ack = self.shared.commands_applied.load(Ordering::Acquire);
        for (index, mirror) in self.voice_mirror.iter().enumerate() {
            if !mirror.live {
                return Some(index);
            }
            // Acquire of the completed sequence precedes reading the live mask.
            if ack >= mirror.play_sequence && !self.rt_voice_live(index) {
                return Some(index);
            }
        }
        None
    }

    /// A voice is finished when the control side retired it or when its play was
    /// applied in a completed callback and the RT reported it inactive.
    fn voice_definitely_finished(&self, slot: usize) -> bool {
        let mirror = &self.voice_mirror[slot];
        !mirror.live
            || (self.shared.commands_applied.load(Ordering::Acquire) >= mirror.play_sequence
                && !self.rt_voice_live(slot))
    }

    fn take_free_range(&mut self, samples: usize) -> Result<PoolRange, AudioServiceError> {
        self.reclaim_acknowledged_ranges();
        let largest = self
            .free_ranges
            .iter()
            .map(|r| r.samples)
            .max()
            .unwrap_or(0);
        let Some(index) = self
            .free_ranges
            .iter()
            .position(|r| r.samples as usize >= samples)
        else {
            if !self.pending_ranges.is_empty() {
                return Err(AudioServiceError::RangeNotYetAcknowledged);
            }
            return Err(AudioServiceError::PcmBudgetExceeded {
                required_samples: samples,
                largest_free_range_samples: largest as usize,
            });
        };
        let range = self.free_ranges[index];
        let rest = range.samples - samples as u32;
        if rest == 0 {
            self.free_ranges.remove(index);
        } else {
            self.free_ranges[index] = PoolRange {
                start: range.start + samples as u32,
                samples: rest,
            };
        }
        Ok(range)
    }

    /// Reclaim only ranges whose Unload sequence is covered by the render thread's
    /// end-of-callback release. Entries form a prefix because the FIFO is ordered.
    fn reclaim_acknowledged_ranges(&mut self) {
        let ack = self.shared.commands_applied.load(Ordering::Acquire);
        let mut count = 0;
        while count < self.pending_ranges.len() && ack >= self.pending_ranges[count].1 {
            let (range, _) = self.pending_ranges[count];
            self.free_ranges.push(range);
            count += 1;
        }
        self.pending_ranges.drain(..count);
    }

    fn push(&mut self, command: Command) -> Result<(), AudioServiceError> {
        use ringbuf::traits::Producer;
        match self.producer.try_push(command) {
            Ok(()) => {
                self.commands_pushed += 1;
                Ok(())
            }
            Err(_) => {
                self.rejected_commands += 1;
                Err(AudioServiceError::CommandQueueFull)
            }
        }
    }
}

/// Bump a generation, skipping zero (zero means "never registered").
fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

#[cfg(feature = "diagnostic")]
impl AudioService {
    /// Diagnostic helper: close and reopen the output stream around the same mixer
    /// core without a real device disconnect. Used by the audio diagnostic to prove
    /// recreation on-device; the real device-loss path is validated through fault
    /// injection in tests. Preserves suspension, just like device-loss recovery.
    pub fn diagnostic_recreate_output(&mut self) -> Result<(), AudioServiceError> {
        self.recreate_output()
    }
}

impl Drop for AudioService {
    fn drop(&mut self) {
        // 1) Drop the backend: closes the stream, which joins all render callbacks
        //    (platform contract) before returning, and drops the callback closures
        //    holding the core pointer.
        self.backend = None;
        // 2) Field destruction now drops CoreOwner (reconstructing its Box only
        //    after the join), then the service's pool Arc. The mixer held the
        //    other pool Arc; no callback allocates, clones or drops pool owners.
    }
}
