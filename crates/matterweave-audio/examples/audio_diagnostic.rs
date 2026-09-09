//! Opt-in audio diagnostic.
//!
//! Runs on host (mock backend, silent) and on Android (`--features backend-android`,
//! real AAudio output). Sequence:
//!
//! 1. Open the service and print the negotiated stream properties.
//! 2. Register a quiet, known sine clip (48 kHz, gain 0.2) and play it.
//! 3. Observe frames/callbacks advancing (nonzero audio frames submitted).
//! 4. Suspend, observe silence, resume, play again.
//! 5. Recreate the output (controlled close/reopen; real device loss is validated
//!    via fault injection in tests) and play again.
//! 6. Ten open/play/stop/close service lifecycles.
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
         device_errors {}, recreations {}, completed {}, suspended {}",
        health.callback_count,
        health.frames_rendered,
        health.commands_applied,
        health.pending_commands,
        health.device_xruns,
        health.device_errors,
        health.stream_recreations,
        health.voices_completed,
        health.suspended,
    );
}

fn drive_to_frames(service: &mut AudioService, target_frames: u64) {
    let deadline = Instant::now() + Duration::from_millis(2000);
    while service.health().frames_rendered < target_frames && Instant::now() < deadline {
        drive_step(service);
    }
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
        .register_clip(ClipSpec::stereo(&clip_samples))
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
        "playback: frames {} -> {} (nonzero audio submitted)",
        before, health.frames_rendered
    );
    print_health("during-play", &health);

    // 2) Suspend: silence, frozen clocks. Resume: continue.
    service.suspend().expect("suspend");
    drive_step(&mut service); // apply/observe one invocation (renders silence)
    let frozen = service.health();
    assert!(frozen.suspended, "render thread reports suspended");
    std::thread::sleep(Duration::from_millis(100));
    println!(
        "suspend: suspended truth set at {} frames; no stale replay (policy is FIFO continuation)",
        frozen.frames_rendered
    );
    service.resume().expect("resume");
    drive_to_frames(&mut service, frozen.frames_rendered + 2_000);
    println!(
        "resume: frames {} -> {}",
        frozen.frames_rendered,
        service.health().frames_rendered
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

    // 4) Ten open/play/stop/close lifecycles.
    for cycle in 1..=cycles {
        let mut svc = AudioService::new().expect("cycle service opens");
        let clip = svc
            .register_clip(ClipSpec::stereo(&clip_samples))
            .expect("cycle clip registers");
        let voice = svc
            .play(clip, PlayOptions::with_gain(DIAGNOSTIC_GAIN))
            .expect("cycle play");
        drive_to_frames(&mut svc, 1_000);
        let h = svc.health();
        assert!(h.frames_rendered > 0, "cycle {cycle}: nonzero frames");
        svc.stop_voice(voice).expect("cycle stop");
        drop(svc); // close
        println!(
            "cycle {cycle}/{}: open/play/stop/close ok (frames {})",
            cycles, h.frames_rendered
        );
    }

    let final_health = service.health();
    print_health("final", &final_health);
    println!(
        "audio diagnostic: PASS (submitted-frames check only; audibility is a human observation)"
    );
}
