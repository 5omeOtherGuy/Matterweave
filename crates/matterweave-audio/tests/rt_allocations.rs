//! Real-time allocation detector (DoD #4).
//!
//! This file is intentionally its own test binary: cargo runs every test file in a
//! separate process, so the counting global allocator observes exactly the
//! allocations of the invocations under test, uncontaminated by other tests or the
//! harness's parallel threads.
//!
//! Requirements exercised: zero heap allocations AND deallocations during 10,000
//! mixer callback invocations, including voice completion and stop handling.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

static ALLOCS: AtomicIsize = AtomicIsize::new(0);
static DEALLOCS: AtomicIsize = AtomicIsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

use matterweave_audio::{AudioService, ClipSpec, PlayOptions};

#[test]
fn ten_thousand_callback_invocations_allocate_nothing() {
    let mut service = AudioService::new().unwrap();
    // Two clips so the loop exercises both mono and stereo mixing plus completion.
    let mono = service
        .register_clip(ClipSpec::mono(&[0.25, -0.25, 0.5, -0.5]))
        .unwrap();
    let stereo = service
        .register_clip(ClipSpec::stereo(&[0.4, -0.4, 0.2, 0.2, 0.6, -0.6]))
        .unwrap();
    let retired = service.register_clip(ClipSpec::mono(&[0.125; 4])).unwrap();
    // Drain registration commands so the measured stretch is steady state.
    let mut buf = vec![0.0f32; 1920];
    service.mock_render(&mut buf).unwrap();
    service.play(retired, PlayOptions::default()).unwrap();

    // Zero the counters AFTER setup: everything below must allocate/deallocate
    // nothing at all.
    ALLOCS.store(0, Ordering::SeqCst);
    DEALLOCS.store(0, Ordering::SeqCst);
    // First measured callback applies Play then Unload and publishes its command
    // acknowledgment, including silencing without callback allocation/deallocation.
    service.unregister_clip(retired).unwrap();

    let mut invocations = 0usize;
    let mut rng_state = 0x12345678u32;
    let mut next_rand = || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 17;
        rng_state ^= rng_state << 5;
        rng_state
    };
    let mut active_voice: Option<matterweave_audio::VoiceHandle> = None;

    while invocations < 10_000 {
        // Interleave control activity deterministically: play, stop, gain, natural
        // completion — all while rendering.
        match next_rand() % 4 {
            0 => {
                if let Some(voice) = active_voice {
                    if service.is_voice_active(voice) {
                        service.set_voice_gain(voice, 0.5).unwrap();
                    }
                }
            }
            1 => {
                if let Some(voice) = active_voice {
                    service.stop_voice(voice).unwrap();
                    active_voice = None;
                }
            }
            2 => {
                let clip = if next_rand() % 2 == 0 { mono } else { stereo };
                active_voice = Some(service.play(clip, PlayOptions::default()).unwrap());
            }
            _ => {
                if let Some(voice) = active_voice {
                    if !service.is_voice_active(voice) {
                        active_voice = None; // completed naturally
                    }
                }
            }
        }
        service.mock_render(&mut buf).unwrap();
        invocations += 1;

        // Per-invocation assertion: the render pass itself must not allocate or
        // deallocate anything, ever.
        let allocs = ALLOCS.load(Ordering::SeqCst);
        let deallocs = DEALLOCS.load(Ordering::SeqCst);
        assert_eq!(
            (allocs, deallocs),
            (0, 0),
            "invocation {invocations}: allocations must be zero (allocs={allocs}, deallocs={deallocs})"
        );
    }

    // Global sanity across the whole measured stretch (implied by the per-call
    // assertion, but asserted independently): zero both ways.
    assert_eq!(ALLOCS.load(Ordering::SeqCst), 0);
    assert_eq!(DEALLOCS.load(Ordering::SeqCst), 0);

    // The loop really exercised completion and stop handling.
    let health = service.health();
    assert!(
        health.voices_completed > 0,
        "loop must include voice completions"
    );
    assert_eq!(health.voices_silenced, 1);
    assert_eq!(health.rt_rejected_commands, 0);
    assert!(
        health.commands_applied > 0,
        "loop must include command applications"
    );
}
