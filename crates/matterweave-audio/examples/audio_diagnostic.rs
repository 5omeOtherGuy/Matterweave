//! Opt-in audio diagnostic.
//!
//! Runs on host (mock backend, silent) and on Android (`--features backend-android`,
//! real AAudio output). Sequence:
//!
//! 1. Open the service and print the negotiated stream properties.
//! 2. Register a quiet, known sine clip (48 kHz, gain 0.2) and play it.
//! 3. Observe frames/callbacks advancing (not proof of nonzero PCM or audibility).
//! 4. Suspend, observe silence, resume, play again.
//! 5. Recreate the output (controlled close/reopen; real device loss is validated
//!    via fault injection in tests) and play again.
//! 6. Repeated open/play/suspend/recreate-while-suspended/resume/stop/close cycles.
//!
//! On-device audible verification is a human observation; submitted frame counts
//! alone do not prove audibility.
//!
//! Usage: `audio_diagnostic [--cycles N]`

use matterweave_audio::{AudioService, ClipSpec, HealthSnapshot, PlayOptions, StreamProperties};
use std::time::{Duration, Instant};

const SINE_SECONDS: f32 = 0.5;
const SINE_FREQ: f32 = 440.0;
const DIAGNOSTIC_GAIN: f32 = 0.2;

fn make_sine_clip() -> Vec<f32> {
    let frames = (SINE_SECONDS * 48_000.0) as usize;
    (0..frames)
        .map(|i| (2.0 * std::f32::consts::PI * SINE_FREQ * i as f32 / 48_000.0).sin() * 0.9)
        .collect()
}

fn print_properties(prefix: &str, props: Option<StreamProperties>) {
    match props {
        Some(p) => println!(
            "{prefix}: rate {} Hz, channels {}, f32 {}, burst {}, capacity {}, \
             frames_per_callback {:?}, device {}, session {:?}, low_latency {}, exclusive {}",
            p.sample_rate,
            p.channels,
            p.float_format,
            p.frames_per_burst,
            p.buffer_capacity_frames,
            p.frames_per_data_callback,
            p.device_id,
            p.session_id,
            p.low_latency,
            p.sharing_exclusive,
        ),
        None => println!("{prefix}: no stream open"),
    }
}

fn print_health(prefix: &str, health: &HealthSnapshot) {
    println!(
        "{prefix}: callbacks {}, frames {}, applied {}, pending {}, xruns {}, \
         device_errors {}, recreations {}, completed {}, suspended {}, rt_suspended {}",
        health.callback_count,
        health.frames_rendered,
        health.commands_applied,
        health.pending_commands,
        health.device_xruns,
        health.device_errors,
        health.stream_recreations,
        health.voices_completed,
        health.suspended,
        health.rt_suspended,
    );
}

fn drive_to_frames(service: &mut AudioService, target_frames: u64) {
    let deadline = Instant::now() + Duration::from_millis(2000);
    while service.health().frames_rendered < target_frames && Instant::now() < deadline {
        drive_step(service);
    }
    assert!(
        service.health().frames_rendered >= target_frames,
        "output failed to reach {target_frames} frames within 2 seconds: {:?}",
        service.health()
    );
}

#[cfg(all(
    feature = "backend-mock",
    not(all(target_os = "android", feature = "backend-android"))
))]
fn drive_step(service: &mut AudioService) {
    let mut buf = [0.0f32; 1920]; // 960 stereo frames, mock-rendered synchronously
    let _ = service.mock_render(&mut buf);
}

#[cfg(not(all(
    feature = "backend-mock",
    not(all(target_os = "android", feature = "backend-android"))
)))]
fn drive_step(service: &mut AudioService) {
    let _ = service.health(); // touch state, then yield: real callbacks drive frames
    std::thread::sleep(Duration::from_millis(5));
}

/// The mock driver renders synchronously on the control thread, so the render-side
/// mirror must advance. Asserting this keeps host coverage at least as strong as
/// before; on a real backend the mirror is printed as an observation instead,
/// because AAudio delivers no callbacks while paused.
#[cfg(all(
    feature = "backend-mock",
    not(all(target_os = "android", feature = "backend-android"))
))]
fn assert_mock_render_mirror_suspended(health: &HealthSnapshot) {
    assert!(
        health.rt_suspended,
        "mock render ran, so the render-thread mirror must report suspended"
    );
}

#[cfg(not(all(
    feature = "backend-mock",
    not(all(target_os = "android", feature = "backend-android"))
)))]
fn assert_mock_render_mirror_suspended(_health: &HealthSnapshot) {}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cycles: usize = args
        .iter()
        .position(|a| a == "--cycles")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    println!("audio diagnostic: opening service");
    let mut service = AudioService::new().expect("service opens");
    print_properties("negotiated", service.stream_properties());
    let props = service.stream_properties().expect("properties after open");

    // 1) Register a quiet known signal and play it.
    let clip_samples = make_sine_clip();
    let clip = service
        .register_clip(ClipSpec::mono(&clip_samples))
        .expect("sine clip registers");
    let voice = service
        .play(clip, PlayOptions::with_gain(DIAGNOSTIC_GAIN))
        .expect("play");
    let before = service.health().frames_rendered;
    drive_to_frames(&mut service, before + 5_000);
    let health = service.health();
    assert!(
        health.frames_rendered > before,
        "frames must advance while playing"
    );
    println!(
        "playback: frames {} -> {} (submitted frames, not proof of audible PCM)",
        before, health.frames_rendered
    );
    print_health("during-play", &health);

    // 2) Suspend: silence, frozen clocks. Resume: continue.
    service.suspend().expect("suspend");
    // Service truth must be observable immediately: AAudio delivers no data
    // callbacks once paused, so no later invocation can publish it.
    let suspended = service.health();
    assert!(
        suspended.suspended,
        "service reports suspended immediately after suspend()"
    );
    drive_step(&mut service); // host: one render invocation; device: yield
    let frozen = service.health();
    assert!(
        frozen.suspended,
        "service remains suspended after one further step"
    );
    assert_mock_render_mirror_suspended(&frozen);
    std::thread::sleep(Duration::from_millis(100));
    println!(
        "suspend: service-suspended true at {} frames; render-side mirror {} (AAudio \
         delivers no callbacks while paused); no stale replay (policy is FIFO continuation)",
        frozen.frames_rendered, frozen.rt_suspended
    );
    service.resume().expect("resume");
    assert!(
        !service.health().suspended,
        "service reports running immediately after resume()"
    );
    drive_to_frames(&mut service, frozen.frames_rendered + 2_000);
    let resumed = service.health();
    assert!(!resumed.suspended, "service is running after resume");
    println!(
        "resume: frames {} -> {}, render-side mirror {}",
        frozen.frames_rendered, resumed.frames_rendered, resumed.rt_suspended
    );

    // 3) Controlled output recreation (no real device disconnect here).
    service.diagnostic_recreate_output().expect("recreate");
    print_properties("recreated", service.stream_properties());
    assert_eq!(
        service.stream_properties().unwrap().sample_rate,
        props.sample_rate,
        "recreated stream keeps the negotiated rate"
    );
    let voice2 = service
        .play(clip, PlayOptions::with_gain(DIAGNOSTIC_GAIN * 0.5))
        .expect("play after recreate");
    let target = service.health().frames_rendered + 2_000;
    drive_to_frames(&mut service, target);
    service.stop_voice(voice2).expect("stop after recreate");
    service.stop_voice(voice).expect("stop first voice");
    print_health("after-recreate", &service.health());

    // 4) Repeated paused recreation and shutdown. Do not assume request_pause is
    // synchronous: only close/reopen guarantees the old callbacks have quiesced.
    for cycle in 1..=cycles {
        let mut svc = AudioService::new().expect("cycle service opens");
        let clip = svc
            .register_clip(ClipSpec::mono(&clip_samples))
            .expect("cycle clip registers");
        let voice = svc
            .play(clip, PlayOptions::with_gain(DIAGNOSTIC_GAIN))
            .expect("cycle play");
        drive_to_frames(&mut svc, 1_000);
        let h = svc.health();
        assert!(h.frames_rendered > 0, "cycle {cycle}: nonzero frames");
        svc.suspend().expect("cycle suspend");
        assert!(svc.health().suspended, "cycle {cycle}: pause intent");
        svc.diagnostic_recreate_output()
            .expect("cycle recreate while suspended");
        let paused = svc.health();
        assert!(
            paused.suspended,
            "cycle {cycle}: recreation preserves pause"
        );
        assert_eq!(paused.stream_recreations, 1);
        // No mock render here either: the recreated output must remain unstarted.
        std::thread::sleep(Duration::from_millis(100));
        let held = svc.health();
        assert_eq!(held.callback_count, paused.callback_count);
        assert_eq!(held.frames_rendered, paused.frames_rendered);
        assert_eq!(held.voices_completed, paused.voices_completed);
        print_health(&format!("cycle {cycle} paused-recreated"), &held);
        svc.resume().expect("cycle resume");
        assert!(!svc.health().suspended, "cycle {cycle}: resume accepted");
        drive_to_frames(&mut svc, held.frames_rendered + 2_000);
        let resumed = svc.health();
        assert!(resumed.callback_count > held.callback_count);
        assert!(!resumed.rt_suspended, "cycle {cycle}: mixer resumed");
        print_health(&format!("cycle {cycle} resumed-progress"), &resumed);
        svc.stop_voice(voice).expect("cycle stop");
        drop(svc); // close joins outstanding callbacks before freeing the core
        println!("cycle {cycle}/{cycles}: paused recreation/resume/shutdown ok");
    }

    let final_health = service.health();
    print_health("final", &final_health);
    drop(service);
    println!("audio diagnostic: final service shutdown complete");
    println!(
        "audio diagnostic: PASS (submitted-frames check only; audibility is a human observation)"
    );
}
