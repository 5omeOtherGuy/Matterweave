//! The real-time mixer core.
//!
//! [`MixerCore`] is the only state the render thread touches. It is created on the
//! control thread, moved behind a raw pointer handed to the output backend's data
//! callback, and stays valid until the stream that owns the callback closures is
//! closed (the close joins all callback threads before it returns), so callbacks can
//! never observe freed state.
//!
//! Every state mutation on the render side happens through [`Command`]s applied at
//! the start of a render invocation. The render pass itself contains no allocation,
//! no locking, no I/O and no logging.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

use ringbuf::traits::Consumer;

use crate::command::{queue, Command};
use crate::config::{MAX_CLIPS, MAX_VOICES};

/// Shared diagnostics and progress signals written by the render thread with atomics
/// and read by the control thread. Lives in an `Arc` held by both sides.
#[derive(Debug, Default)]
pub(crate) struct SharedRt {
    /// Incremented at the end of every render invocation (diagnostic only).
    /// Callback completion alone does not acknowledge commands queued after drain.
    pub ack_epoch: AtomicU64,
    /// Data-callback invocations completed.
    pub callback_count: AtomicU64,
    /// Frames handed to the output device.
    pub frames_rendered: AtomicU64,
    /// Bitmask of voice slots whose last invocation ended with the voice active.
    pub live_mask: AtomicU32,
    /// Render thread's own suspended view as of the last completed invocation
    /// (mirror of the `Suspend`/`Resume` commands it applied). It cannot turn true
    /// while the backend stops delivering callbacks.
    pub suspended: AtomicBool,
    /// FIFO commands applied through a completed callback. Published with Release
    /// after PCM reads and live-mask stores; Acquire gates pool/voice reuse.
    pub commands_applied: AtomicU64,
    /// Commands rejected at the render thread (generation mismatch or invalid
    /// target). Under normal operation this stays zero.
    pub rt_rejected_commands: AtomicU64,
    /// Voices silenced because their clip was unregistered.
    pub voices_silenced: AtomicU64,
    /// Voices that finished playing their clip.
    pub voices_completed: AtomicU64,
    /// Last device error code reported by the error callback (0 = none).
    pub error_code: AtomicI32,
    /// Set by the error callback; cleared by the control thread after recreation.
    pub disconnected: AtomicBool,
}

/// A clip slot as seen by the render thread.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RtClipSlot {
    /// Current generation (matches the control mirror when both are up to date).
    pub generation: u32,
    /// Whether a clip is registered in this slot.
    pub active: bool,
}

impl RtClipSlot {
    const fn empty() -> Self {
        Self {
            generation: 0,
            active: false,
        }
    }
}

/// A voice slot as seen by the render thread.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RtVoice {
    /// Current generation (matches the handle held by the game while this voice
    /// lives).
    pub generation: u32,
    /// Whether this voice is audible.
    pub active: bool,
    /// Clip slot this voice plays from (used by `UnloadClip` to silence it).
    pub clip_slot: u8,
    /// Clip channels (1 or 2).
    pub clip_channels: u8,
    /// First pool sample of the clip range.
    pub start: u32,
    /// Remaining clip length in frames.
    pub frames: u32,
    /// Next clip frame to play.
    pub cursor: u32,
    /// Linear gain.
    pub gain: f32,
}

impl RtVoice {
    const fn empty() -> Self {
        Self {
            generation: 0,
            active: false,
            clip_slot: 0,
            clip_channels: 1,
            start: 0,
            frames: 0,
            cursor: 0,
            gain: 0.0,
        }
    }
}

/// Clamp a mixed sample to the output contract: finite and within [-1, 1].
#[inline]
fn clamp_sample(v: f32) -> f32 {
    // Inputs are validated finite and gains bounded, so this guard is
    // defense-in-depth for the output contract.
    if v.is_finite() {
        v.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// The real-time mixer state.
pub(crate) struct MixerCore {
    /// Consumer half of the bounded command queue.
    pub consumer: crate::command::queue::Consumer,
    /// The PCM pool. Allocated once; never resized; freed only after the stream that
    /// references it is closed.
    pub pool: Box<[f32]>,
    slots: [RtClipSlot; MAX_CLIPS],
    voices: [RtVoice; MAX_VOICES],
    suspended: bool,
    /// Local FIFO progress, published only after the callback finishes mixing.
    commands_applied: u64,
    pub shared: std::sync::Arc<SharedRt>,
}

impl MixerCore {
    /// Create the core for a freshly allocated pool.
    pub(crate) fn new(
        pool: Box<[f32]>,
        shared: std::sync::Arc<SharedRt>,
    ) -> (Self, queue::Producer) {
        let (producer, consumer) = queue::split();
        let core = Self {
            consumer,
            pool,
            slots: core::array::from_fn(|_| RtClipSlot::empty()),
            voices: core::array::from_fn(|_| RtVoice::empty()),
            suspended: false,
            commands_applied: 0,
            shared,
        };
        (core, producer)
    }

    /// Apply one control command to the render-side state.
    fn apply(&mut self, command: Command) {
        match command {
            Command::LoadClip { slot, generation } => {
                let slot = &mut self.slots[slot as usize];
                slot.generation = generation;
                slot.active = true;
            }
            Command::UnloadClip { slot, generation } => {
                let rt_slot = &mut self.slots[slot as usize];
                if rt_slot.active && rt_slot.generation == generation {
                    rt_slot.active = false;
                    let slot_index = slot;
                    let mut silenced = 0;
                    for voice in &mut self.voices {
                        if voice.active && voice.clip_slot == slot_index {
                            voice.active = false;
                            silenced += 1;
                        }
                    }
                    self.shared
                        .voices_silenced
                        .fetch_add(silenced, Ordering::Relaxed);
                } else {
                    self.shared
                        .rt_rejected_commands
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            Command::Play {
                voice_slot,
                voice_generation,
                clip_slot,
                clip_generation,
                start,
                frames,
                clip_channels,
                gain,
            } => {
                let clip_ok = self.slots[clip_slot as usize].active
                    && self.slots[clip_slot as usize].generation == clip_generation;
                let voice = &mut self.voices[voice_slot as usize];
                // The slot must be free; the new generation is assigned here (a
                // fresh generation never exists in the RT table before its Play).
                if clip_ok && !voice.active {
                    voice.generation = voice_generation;
                    voice.active = true;
                    voice.clip_slot = clip_slot;
                    voice.clip_channels = clip_channels;
                    voice.start = start;
                    voice.frames = frames;
                    voice.cursor = 0;
                    voice.gain = gain;
                } else {
                    self.shared
                        .rt_rejected_commands
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            Command::Stop {
                voice_slot,
                voice_generation,
            } => {
                let voice = &mut self.voices[voice_slot as usize];
                if voice.generation == voice_generation {
                    // Stopping an already-finished voice is a no-op, not a rejection.
                    voice.active = false;
                } else {
                    self.shared
                        .rt_rejected_commands
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            Command::SetGain {
                voice_slot,
                voice_generation,
                gain,
            } => {
                let voice = &mut self.voices[voice_slot as usize];
                if voice.generation == voice_generation && voice.active {
                    voice.gain = gain;
                } else if voice.generation != voice_generation {
                    self.shared
                        .rt_rejected_commands
                        .fetch_add(1, Ordering::Relaxed);
                }
                // Same generation but finished voice: no-op.
            }
            Command::Suspend => self.suspended = true,
            Command::Resume => self.suspended = false,
        }
    }

    /// One render invocation: drain commands, mix voices into `out` (interleaved,
    /// `channels` samples per frame), publish state, bump the ack epoch.
    ///
    /// Real-time contract: no allocation, no deallocation, no locks, no I/O, no
    /// logging, no stream shutdown inside this function.
    pub(crate) fn render(&mut self, out: &mut [f32], channels: usize) {
        self.render_with_after_drain(out, channels, || {});
    }

    // A no-op in production; tests can deterministically enqueue after the last
    // drain but before this callback mixes/publishes, without threads or locks.
    pub(crate) fn render_with_after_drain(
        &mut self,
        out: &mut [f32],
        channels: usize,
        after_drain: impl FnOnce(),
    ) {
        debug_assert!(channels == 1 || channels == 2);
        debug_assert!(out.len().is_multiple_of(channels.max(1)));

        // 1) Apply every queued command in FIFO order.
        while let Some(command) = self.consumer.try_pop() {
            self.apply(command);
            self.commands_applied += 1;
        }

        after_drain();

        // 2) Mix. Suspend renders silence and freezes all voice cursors.
        out.fill(0.0);
        if !self.suspended && channels > 0 {
            let frames = out.len() / channels;
            for frame in 0..frames {
                let mut left = 0.0f32;
                let mut right = 0.0f32;
                for slot in 0..MAX_VOICES {
                    let voice = &mut self.voices[slot];
                    if !voice.active {
                        continue;
                    }
                    if voice.cursor >= voice.frames {
                        voice.active = false;
                        self.shared.voices_completed.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    // Pool indexing is bounds-checked; ranges are provably inside the
                    // pool (clip registration validated frame completeness, and a
                    // range is never reused while a voice references it).
                    let base =
                        voice.start as usize + voice.cursor as usize * voice.clip_channels as usize;
                    let (cl, cr) = if voice.clip_channels == 2 {
                        (self.pool[base], self.pool[base + 1])
                    } else {
                        let s = self.pool[base];
                        (s, s)
                    };
                    voice.cursor += 1;
                    if voice.cursor >= voice.frames {
                        voice.active = false;
                        self.shared.voices_completed.fetch_add(1, Ordering::Relaxed);
                    }
                    left += cl * voice.gain;
                    right += cr * voice.gain;
                }
                let out_index = frame * channels;
                if channels == 2 {
                    out[out_index] = clamp_sample(left);
                    out[out_index + 1] = clamp_sample(right);
                } else {
                    // Mono device: mean of both contributions (see crate docs).
                    out[out_index] = clamp_sample((left + right) * 0.5);
                }
            }
        }

        // 3) Publish state, then release the completed command sequence. Callback
        //    epochs remain diagnostics, never evidence of a particular command.
        let mut live_mask = 0u32;
        for (index, voice) in self.voices.iter().enumerate() {
            if voice.active {
                live_mask |= 1 << index;
            }
        }
        self.shared.live_mask.store(live_mask, Ordering::Release);
        self.shared
            .suspended
            .store(self.suspended, Ordering::Release);
        self.shared.callback_count.fetch_add(1, Ordering::Relaxed);
        self.shared
            .frames_rendered
            .fetch_add(out.len() as u64 / channels.max(1) as u64, Ordering::Relaxed);
        // No PCM reads or voice-state writes follow this release. Control-side
        // Acquire readers may now reclaim unloaded PCM and trust acknowledged
        // voice generations. A later callback's drain cannot acknowledge early.
        self.shared
            .commands_applied
            .store(self.commands_applied, Ordering::Release);
        self.shared.ack_epoch.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::Producer;

    /// A real render thread churns the mixer while the control thread pushes the
    /// whole command set. Asserts no panic, no deadlock (test completes), full
    /// command application and sane diagnostics (DoD #6, core-level interleaving).
    #[test]
    fn concurrent_control_and_render_completes_without_deadlock() {
        let pool = vec![0.0f32; 4096].into_boxed_slice();
        let shared = std::sync::Arc::new(SharedRt::default());
        let (core, mut producer) = MixerCore::new(pool, shared.clone());

        let render_thread = std::thread::spawn(move || {
            let mut core = core;
            let mut buf = [0.0f32; 960];
            for _ in 0..5000 {
                core.render(&mut buf, 2);
            }
            core
        });

        let mut pushed = 0u64;
        for i in 0..2000u32 {
            let command = match i % 6 {
                0 => Command::Play {
                    voice_slot: (i % 8) as u8,
                    voice_generation: 1 + i,
                    clip_slot: 0,
                    clip_generation: 1,
                    start: 0,
                    frames: 4,
                    clip_channels: 1,
                    gain: 0.5,
                },
                1 => Command::Stop {
                    voice_slot: (i % 8) as u8,
                    voice_generation: 1 + i,
                },
                2 => Command::SetGain {
                    voice_slot: (i % 8) as u8,
                    voice_generation: 1 + i,
                    gain: 0.25,
                },
                3 => Command::Suspend,
                4 => Command::Resume,
                _ => Command::LoadClip {
                    slot: (i % 32) as u8,
                    generation: 1 + i,
                },
            };
            if producer.try_push(command).is_ok() {
                pushed += 1;
            }
            if i % 400 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        // One deliberately stale command: guaranteed RT-side rejection path.
        if producer
            .try_push(Command::Stop {
                voice_slot: 0,
                voice_generation: 999_999,
            })
            .is_ok()
        {
            pushed += 1;
        }

        let mut core = render_thread.join().expect("render thread must not panic");
        // Drain everything still queued; then every pushed command is applied.
        let mut buf = [0.0f32; 960];
        core.render(&mut buf, 2);
        core.render(&mut buf, 2);

        assert_eq!(core.shared.commands_applied.load(Ordering::Acquire), pushed);
        assert!(core.shared.callback_count.load(Ordering::Acquire) >= 5002);
        assert_eq!(
            core.shared.frames_rendered.load(Ordering::Acquire),
            5002 * 480
        );
        // The deliberate stale command hit the RT rejection path without harm;
        // adversarial generation churn may add more rejections (count is robust).
        assert!(core.shared.rt_rejected_commands.load(Ordering::Acquire) >= 1);
        // Queue must be fully drained.
        assert_eq!(core.consumer.try_pop(), None);
    }
}
