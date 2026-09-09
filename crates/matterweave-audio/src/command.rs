//! Control commands flowing from the game thread to the render thread.
//!
//! The queue is a bounded SPSC ring buffer (`ringbuf`) of these commands. `try_push`
//! never overwrites and `try_pop` never blocks; both fail/lazy-fetch safely at the
//! boundaries. All mutations of mixer state happen by applying these commands at the
//! start of a render invocation, which keeps the render path allocation-free.

/// One control command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Command {
    /// Publish a clip into a slot (payload was written into the pool by the control
    /// thread before this command was queued; the range is provably unreferenced).
    LoadClip {
        /// Clip slot.
        slot: u8,
        /// New generation for the slot.
        generation: u32,
    },
    /// Retire a clip slot and silence every voice still playing it.
    UnloadClip {
        /// Clip slot.
        slot: u8,
        /// New (invalidating) generation.
        generation: u32,
    },
    /// Start a voice playing a registered clip.
    Play {
        /// Voice slot.
        voice_slot: u8,
        /// New generation for the voice slot.
        voice_generation: u32,
        /// Clip slot the voice plays from.
        clip_slot: u8,
        /// Clip generation the caller saw (rechecked).
        clip_generation: u32,
        /// First pool sample of the clip range.
        start: u32,
        /// Clip length in frames.
        frames: u32,
        /// Clip channels (1 or 2).
        clip_channels: u8,
        /// Initial voice gain (already validated finite and in range).
        gain: f32,
    },
    /// Stop a voice.
    Stop {
        /// Voice slot.
        voice_slot: u8,
        /// Voice generation (rechecked).
        voice_generation: u32,
    },
    /// Change a voice's gain.
    SetGain {
        /// Voice slot.
        voice_slot: u8,
        /// Voice generation (rechecked).
        voice_generation: u32,
        /// New gain (already validated).
        gain: f32,
    },
    /// Freeze the mixer clock and render silence.
    Suspend,
    /// Unfreeze the mixer clock.
    Resume,
}

/// Command queue type aliases built on `ringbuf`'s heap SPSC ring buffer.
pub(crate) mod queue {
    use ringbuf::{HeapCons, HeapProd, HeapRb};

    use super::Command;

    /// Bounded SPSC command queue capacity.
    pub const CAPACITY: usize = crate::MAX_COMMANDS;

    /// Producer half type (control thread side).
    pub type Producer = HeapProd<Command>;
    /// Consumer half type (render thread side).
    pub type Consumer = HeapCons<Command>;

    /// Create the queue and split it into (producer, consumer).
    pub fn split() -> (Producer, Consumer) {
        use ringbuf::traits::Split;
        HeapRb::<Command>::new(CAPACITY).split()
    }

    /// Number of commands the producer can enqueue right now (lower bound).
    pub fn vacant(prod: &Producer) -> usize {
        use ringbuf::traits::Observer;
        prod.vacant_len()
    }
}
