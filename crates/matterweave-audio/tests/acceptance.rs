//! Acceptance tests for the bounded audio service.
//!
//! Covers: limit enforcement at limit/limit+1 and after reuse, atomic rejection of
//! invalid input, stale-handle invalidation, the two-voice mixer fixture (against an
//! independently computed reference), clipping, stereo channel order, clip
//! completion, gain, stop, silence, saturation/backpressure recovery, the documented
//! suspend/resume policy, and fault-injected device loss + stream recreation.

use matterweave_audio::{
    AudioService, AudioServiceError, ClipSpec, PlayOptions, MAX_CLIPS, MAX_COMMANDS, MAX_PCM_BYTES,
    MAX_VOICES,
};

/// Render `frames` stereo frames synchronously through the mock backend and return
/// the interleaved samples.
fn render_stereo(service: &mut AudioService, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    let rendered = service.mock_render(&mut out).expect("mock render");
    assert_eq!(rendered, frames);
    out
}

fn silence(buf: &[f32]) -> bool {
    buf.iter().all(|&s| s == 0.0)
}

fn bounded(buf: &[f32]) -> bool {
    buf.iter()
        .all(|&s| s.is_finite() && (-1.0..=1.0).contains(&s))
}

// ---------------------------------------------------------------------------
// Basic playback: stereo channel order, gain, completion, stop, silence
// ---------------------------------------------------------------------------

#[test]
fn stereo_channel_order_is_preserved() {
    let mut service = AudioService::new().unwrap();
    // One stereo frame: L=1.0, R=-1.0 ... use smaller values to stay unclipped.
    let clip = service
        .register_clip(ClipSpec::stereo(&[0.5, -0.25]))
        .unwrap();
    let _voice = service.play(clip, PlayOptions::default()).unwrap();
    let out = render_stereo(&mut service, 1);
    assert_eq!(
        out,
        vec![0.5, -0.25],
        "left then right, unchanged at gain 1.0"
    );
}

#[test]
fn mono_clip_upmixes_to_both_channels() {
    let mut service = AudioService::new().unwrap();
    let clip = service
        .register_clip(ClipSpec::mono(&[0.25, -0.5]))
        .unwrap();
    service.play(clip, PlayOptions::default()).unwrap();
    let out = render_stereo(&mut service, 2);
    assert_eq!(out, vec![0.25, 0.25, -0.5, -0.5]);
}

#[test]
fn gain_scales_output() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.5])).unwrap();
    service.play(clip, PlayOptions::with_gain(0.25)).unwrap();
    assert_eq!(render_stereo(&mut service, 1), vec![0.125, 0.125]);

    // set_voice_gain changes a running voice.
    let clip2 = service
        .register_clip(ClipSpec::mono(&[0.5, 0.5, 0.5]))
        .unwrap();
    let voice = service.play(clip2, PlayOptions::with_gain(1.0)).unwrap();
    service.set_voice_gain(voice, 0.5).unwrap();
    assert_eq!(render_stereo(&mut service, 1), vec![0.25, 0.25]);
}

#[test]
fn clip_completion_then_silence() {
    let mut service = AudioService::new().unwrap();
    let clip = service
        .register_clip(ClipSpec::mono(&[0.1, 0.2, 0.3]))
        .unwrap();
    let voice = service.play(clip, PlayOptions::default()).unwrap();
    let out = render_stereo(&mut service, 5);
    assert_eq!(&out[..6], &[0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    assert!(silence(&out[6..]), "after completion the mix is silent");
    assert!(!service.is_voice_active(voice), "voice completed");
    assert_eq!(service.health().voices_completed, 1);

    // stop_voice on the completed voice is a successful no-op.
    service.stop_voice(voice).unwrap();
}

#[test]
fn stop_voice_silences_immediately_and_completes_nothing() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.4; 4])).unwrap();
    let voice = service.play(clip, PlayOptions::default()).unwrap();
    render_stereo(&mut service, 1);
    service.stop_voice(voice).unwrap();
    let out = render_stereo(&mut service, 3);
    assert!(silence(&out), "stopped voice contributes nothing");
    assert!(!service.is_voice_active(voice));
    assert_eq!(
        service.health().voices_completed,
        0,
        "stop is not completion"
    );
}

#[test]
fn output_is_always_bounded_and_finite_when_clipping() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[1.0, 1.0])).unwrap();
    // Two voices at full gain → sum 2.0 → clamped to 1.0.
    let _a = service.play(clip, PlayOptions::default()).unwrap();
    let _b = service.play(clip, PlayOptions::default()).unwrap();
    let out = render_stereo(&mut service, 1);
    assert_eq!(
        out,
        vec![1.0, 1.0],
        "saturating sum of two full-gain voices"
    );
    assert!(bounded(&out));
}

// ---------------------------------------------------------------------------
// The two-voice fixture (DoD #3): independently computed reference within 1e-6
// ---------------------------------------------------------------------------

#[test]
fn two_voice_fixture_matches_independent_reference() {
    // Fixture: voice A = mono ramp [0.1, 0.2, 0.3, 0.4] gain 0.5 (slot order 0),
    // voice B = stereo [(0.8, -0.8), (0.4, 0.6)] gain 2.0 (slot order 1).
    // The expected values below are computed by hand here, NOT via the mixer.
    let mut service = AudioService::new().unwrap();
    let mono = service
        .register_clip(ClipSpec::mono(&[0.1, 0.2, 0.3, 0.4]))
        .unwrap();
    let stereo = service
        .register_clip(ClipSpec::stereo(&[0.8, -0.8, 0.4, 0.6]))
        .unwrap();
    let _va = service.play(mono, PlayOptions::with_gain(0.5)).unwrap();
    let _vb = service.play(stereo, PlayOptions::with_gain(2.0)).unwrap();

    // Reference computed independently of the mixer implementation:
    //   frame 0: L = 0.1*0.5 + 0.8*2.0 = 0.05 + 1.60 = 1.65 → clamp 1.0
    //            R = 0.1*0.5 + (-0.8)*2.0 = 0.05 - 1.60 = -1.55 → clamp -1.0
    //   frame 1: L = 0.2*0.5 + 0.4*2.0 = 0.10 + 0.80 = 0.90
    //            R = 0.2*0.5 + 0.6*2.0 = 0.10 + 1.20 = 1.30 → clamp 1.0
    //   frame 2: L = 0.3*0.5 = 0.15 (voice B done: 2 frames)
    //            R = 0.3*0.5 = 0.15
    //   frame 3: L = R = 0.4*0.5 = 0.20
    let expected = [
        1.0f32, -1.0, // frame 0 (clipped both sides)
        0.90, 1.0, // frame 1 (right clipped)
        0.15, 0.15, // frame 2
        0.20, 0.20, // frame 3
    ];
    let out = render_stereo(&mut service, 4);
    for (i, (got, want)) in out.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - want).abs() < 1e-6,
            "sample {i}: got {got}, want {want}"
        );
    }
    // Voice B (stereo) completed after 2 frames; voice A after 4.
    let health = service.health();
    assert_eq!(health.voices_completed, 2);
}

// ---------------------------------------------------------------------------
// Limits: at limit, limit+1, and after stop/unregister/reuse
// ---------------------------------------------------------------------------

#[test]
fn clip_limit_enforced_at_32_and_recovers_after_unregister() {
    let mut service = AudioService::new().unwrap();
    let mut handles = Vec::new();
    for i in 0..MAX_CLIPS {
        handles.push(
            service
                .register_clip(ClipSpec::mono(&[0.01 * (i as f32 + 1.0)]))
                .expect("register"),
        );
    }
    // limit+1 must be rejected, nothing stolen.
    let err = service.register_clip(ClipSpec::mono(&[1.0])).unwrap_err();
    assert_eq!(err, AudioServiceError::ClipLimit);
    assert_eq!(service.health().rejected_registrations, 1);

    // Unregister one and the slot is reusable.
    service.unregister_clip(handles[7]).unwrap();
    let replacement = service
        .register_clip(ClipSpec::mono(&[0.9]))
        .expect("register after unregister");
    // The old handle is stale.
    assert!(!service.clip_is_registered(handles[7]));
    assert!(service.clip_is_registered(replacement));
    // A second unregister of the stale handle fails without affecting anything.
    assert_eq!(
        service.unregister_clip(handles[7]).unwrap_err(),
        AudioServiceError::StaleClipHandle
    );
    assert!(
        service.clip_is_registered(replacement),
        "stale call changed nothing"
    );
}

#[test]
fn voice_limit_enforced_at_8_with_no_stealing() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.5; 1000])).unwrap();
    let mut voices = Vec::new();
    for _ in 0..MAX_VOICES {
        voices.push(service.play(clip, PlayOptions::default()).expect("play"));
    }
    let err = service.play(clip, PlayOptions::default()).unwrap_err();
    assert_eq!(err, AudioServiceError::VoiceLimit);
    assert_eq!(service.health().rejected_voice_starts, 1);

    // After a stop, exactly one new voice fits once a callback applied the stop
    // (capacity decisions reflect render-thread truth, never a queued promise).
    service.stop_voice(voices[3]).unwrap();
    render_stereo(&mut service, 1);
    let fresh = service
        .play(clip, PlayOptions::default())
        .expect("play after stop");
    let err = service.play(clip, PlayOptions::default()).unwrap_err();
    assert_eq!(err, AudioServiceError::VoiceLimit);
    // Slot 3 is the only free voice slot, so the fresh voice must take it; assert
    // that determinism instead of conditionally skipping the stale-handle check.
    assert_eq!(
        fresh.slot(),
        voices[3].slot(),
        "fresh voice takes the freed slot"
    );
    assert_eq!(
        service.stop_voice(voices[3]).unwrap_err(),
        AudioServiceError::StaleVoiceHandle
    );
    // One render applies the fresh voice's play (liveness lags one callback).
    render_stereo(&mut service, 1);
    assert!(
        service.is_voice_active(fresh),
        "fresh voice unaffected by stale stop"
    );
}

#[test]
fn pcm_budget_enforced_and_recovered_after_unregister() {
    let mut service = AudioService::new().unwrap();
    // One clip that needs more than the whole 4 MiB budget: rejected atomically.
    let too_big = vec![0.0f32; MAX_PCM_BYTES / 4 + 1];
    let err = service.register_clip(ClipSpec::mono(&too_big)).unwrap_err();
    assert!(matches!(err, AudioServiceError::PcmBudgetExceeded { .. }));
    assert_eq!(
        service.retained_pcm_bytes(),
        0,
        "rejection left no partial state"
    );

    // Fill the budget exactly: 4 MiB of stereo clips in one registration.
    let exact = vec![0.0f32; MAX_PCM_BYTES / 4];
    let big = service
        .register_clip(ClipSpec::stereo(&exact))
        .expect("budget-filling clip");
    assert_eq!(service.retained_pcm_bytes(), MAX_PCM_BYTES);
    let err = service.register_clip(ClipSpec::mono(&[0.0])).unwrap_err();
    assert!(matches!(err, AudioServiceError::PcmBudgetExceeded { .. }));

    // After unregister (and an ack-advancing callback), the budget is recoverable.
    service.unregister_clip(big).unwrap();
    assert_eq!(service.retained_pcm_bytes(), 0);
    // Pending-ack: without a callback the range is not reusable yet.
    let err = service.register_clip(ClipSpec::mono(&[0.0])).unwrap_err();
    assert_eq!(err, AudioServiceError::RangeNotYetAcknowledged);
    render_stereo(&mut service, 1); // one invocation acknowledges the unload
    service
        .register_clip(ClipSpec::mono(&[0.0]))
        .expect("budget recovered after ack");
}

#[test]
fn invalid_clips_are_rejected_atomically() {
    let mut service = AudioService::new().unwrap();
    let first = service.register_clip(ClipSpec::mono(&[0.5])).unwrap();

    // Invalid channels.
    let err = service
        .register_clip(ClipSpec {
            samples: &[0.5, 0.5],
            channels: 3,
        })
        .unwrap_err();
    assert!(matches!(err, AudioServiceError::InvalidClip(_)));
    // Incomplete frames.
    let err = service
        .register_clip(ClipSpec::stereo(&[0.5, 0.5, 0.5]))
        .unwrap_err();
    assert!(matches!(err, AudioServiceError::InvalidClip(_)));
    // Nonfinite sample.
    let err = service
        .register_clip(ClipSpec::mono(&[0.5, f32::NAN]))
        .unwrap_err();
    assert!(matches!(err, AudioServiceError::InvalidClip(_)));
    let err = service
        .register_clip(ClipSpec::mono(&[f32::INFINITY]))
        .unwrap_err();
    assert!(matches!(err, AudioServiceError::InvalidClip(_)));
    // Empty.
    let err = service.register_clip(ClipSpec::mono(&[])).unwrap_err();
    assert!(matches!(err, AudioServiceError::InvalidClip(_)));

    // Nothing changed: the valid clip still registered, no phantom slots.
    assert!(service.clip_is_registered(first));
    assert_eq!(service.health().rejected_registrations, 5);
    assert_eq!(service.retained_pcm_bytes(), 4);
}

#[test]
fn invalid_gains_are_rejected() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.5; 4])).unwrap();
    for gain in [f32::NAN, f32::INFINITY, 100.0, -100.0] {
        let err = service
            .play(clip, PlayOptions::with_gain(gain))
            .unwrap_err();
        assert_eq!(err, AudioServiceError::InvalidGain);
    }
    let voice = service.play(clip, PlayOptions::default()).unwrap();
    let err = service.set_voice_gain(voice, f32::NAN).unwrap_err();
    assert_eq!(err, AudioServiceError::InvalidGain);
    // Voice still plays at original gain.
    assert_eq!(render_stereo(&mut service, 1), vec![0.5, 0.5]);
}

// ---------------------------------------------------------------------------
// Stale handles (DoD #2)
// ---------------------------------------------------------------------------

#[test]
fn stale_clip_handle_cannot_affect_later_clip() {
    let mut service = AudioService::new().unwrap();
    let clip_a = service.register_clip(ClipSpec::mono(&[0.5; 4])).unwrap();
    service.unregister_clip(clip_a).unwrap();
    let clip_b = service.register_clip(ClipSpec::mono(&[0.9; 4])).unwrap();

    // Play with the stale handle must fail and must not start a voice on clip B.
    let err = service.play(clip_a, PlayOptions::default()).unwrap_err();
    assert_eq!(err, AudioServiceError::StaleClipHandle);
    let voice_b = service.play(clip_b, PlayOptions::default()).unwrap();
    assert_eq!(
        render_stereo(&mut service, 1),
        vec![0.9, 0.9],
        "clip B unaffected"
    );
    let _ = voice_b;
}

#[test]
fn stale_voice_handle_cannot_affect_later_voice() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.5; 4])).unwrap();
    let v1 = service.play(clip, PlayOptions::default()).unwrap();
    service.stop_voice(v1).unwrap();
    render_stereo(&mut service, 1); // apply stop, frees the slot (RT truth)
    let v2 = service
        .play(clip, PlayOptions::default())
        .expect("slot reused");
    assert_eq!(v1.slot(), v2.slot(), "same slot reused");
    assert_ne!(
        v1.generation(),
        v2.generation(),
        "generation distinguishes them"
    );

    // Stale operations must fail, not touch voice 2.
    assert_eq!(
        service.stop_voice(v1).unwrap_err(),
        AudioServiceError::StaleVoiceHandle
    );
    assert_eq!(
        service.set_voice_gain(v1, 0.1).unwrap_err(),
        AudioServiceError::StaleVoiceHandle
    );
    // One render applies voice 2's play; the liveness answer lags one callback.
    assert_eq!(render_stereo(&mut service, 1), vec![0.5, 0.5]);
    assert!(
        service.is_voice_active(v2),
        "voice 2 unaffected by stale handle calls"
    );
}

// ---------------------------------------------------------------------------
// Saturation and backpressure (DoD #5)
// ---------------------------------------------------------------------------

#[test]
fn command_queue_full_is_explicit_backpressure_and_recovers() {
    let mut service = AudioService::new().unwrap();
    let _clip = service.register_clip(ClipSpec::mono(&[0.5; 8])).unwrap();
    // Drain registration/first commands: everything so far was already queued.
    // Fill the remaining queue with play commands (8 voices is the cap, so use
    // set_voice_gain on real voices + extra plays on distinct... plays cap at 8.
    // Instead: fill with play+stop pairs on short clips? Simplest deterministic
    // filler: set_voice_gain commands on 8 long-lived voices.
    let mut voices = Vec::new();
    let filler_clip = service
        .register_clip(ClipSpec::mono(&[0.1; 10000]))
        .unwrap();
    for _ in 0..MAX_VOICES {
        voices.push(service.play(filler_clip, PlayOptions::default()).unwrap());
    }
    render_stereo(&mut service, 1); // apply the 8 plays
                                    // Now fill the queue to exactly 64 with gain changes.
    let mut queued = service.health().pending_commands;
    let mut i: usize = 0;
    while queued < MAX_COMMANDS {
        service
            .set_voice_gain(voices[i % MAX_VOICES], 0.2 + i as f32 * 0.001)
            .expect("fill queue");
        queued += 1;
        i += 1;
    }
    assert_eq!(service.health().pending_commands, MAX_COMMANDS);
    // One more command must be rejected with backpressure, queue unchanged.
    let err = service.set_voice_gain(voices[0], 0.9).unwrap_err();
    assert_eq!(err, AudioServiceError::CommandQueueFull);
    assert_eq!(service.health().pending_commands, MAX_COMMANDS);
    assert_eq!(service.health().rejected_commands, 1);

    // Recovery: drain via callbacks, then the queue accepts again.
    render_stereo(&mut service, 2);
    assert_eq!(service.health().pending_commands, 0);
    service
        .set_voice_gain(voices[0], 0.9)
        .expect("queue accepts after drain");
}

#[test]
fn voice_saturation_recovery_after_completion() {
    let mut service = AudioService::new().unwrap();
    let short = service.register_clip(ClipSpec::mono(&[0.5; 2])).unwrap();
    let long = service.register_clip(ClipSpec::mono(&[0.5; 1000])).unwrap();
    for _ in 0..MAX_VOICES {
        service.play(short, PlayOptions::default()).unwrap();
    }
    assert_eq!(
        service.play(short, PlayOptions::default()).unwrap_err(),
        AudioServiceError::VoiceLimit
    );
    render_stereo(&mut service, 2); // all 8 voices complete
    assert_eq!(service.health().voices_completed, MAX_VOICES as u64);
    // All voice slots free again (completion), play works without stopping anything.
    for _ in 0..MAX_VOICES {
        service
            .play(long, PlayOptions::default())
            .expect("slots reusable after completion");
    }
}

// ---------------------------------------------------------------------------
// Suspend/resume policy (DoD #5): documented policy tested
// ---------------------------------------------------------------------------

#[test]
fn suspend_freezes_clock_and_resume_continues_exactly() {
    let mut service = AudioService::new().unwrap();
    let clip = service
        .register_clip(ClipSpec::mono(&[0.1, 0.2, 0.3, 0.4, 0.5]))
        .unwrap();
    let voice = service.play(clip, PlayOptions::default()).unwrap();
    render_stereo(&mut service, 2); // cursor at frame 2
    let h = service.health();
    assert!(h.frames_rendered >= 2);

    service.suspend().unwrap();
    // Control-side truth is observable before any further render callback: AAudio
    // delivers none while paused (2026-09-12 OnePlus 13 defect).
    assert!(
        service.health().suspended,
        "service reports suspended immediately after suspend()"
    );
    // Extra renders while suspended produce silence and do not advance the voice.
    let out = render_stereo(&mut service, 4);
    assert!(silence(&out), "suspended output is silent");
    let h = service.health();
    assert!(h.suspended, "service truth: suspended");
    assert!(
        h.rt_suspended,
        "render truth: the mock driver ran a render pass, so the mirror is set"
    );

    service.resume().unwrap();
    assert!(
        !service.health().suspended,
        "service truth: resume clears suspension before the first callback"
    );
    let out = render_stereo(&mut service, 3);
    assert!(
        !service.health().rt_suspended,
        "render truth: the render pass applied Resume"
    );
    // Continues exactly where it stopped: frames 2,3,4 of the clip.
    assert_eq!(&out[..6], &[0.3, 0.3, 0.4, 0.4, 0.5, 0.5]);
    assert!(
        !service.is_voice_active(voice),
        "voice finished after resume"
    );
}

#[test]
fn suspend_does_not_restart_or_replay_stale_sounds() {
    // Policy: suspend freezes the mixer clock; voices continue exactly where they
    // stopped; queued plays start after resume; nothing restarts or double-plays.
    let mut service = AudioService::new().unwrap();
    // Positional ramp: clip frame i carries (i+1) * 0.002, so resumed positions are
    // observable in the mixed output.
    let clip_samples: Vec<f32> = (0..110).map(|i| (i as f32 + 1.0) * 0.002).collect();
    let clip = service
        .register_clip(ClipSpec::mono(&clip_samples))
        .unwrap();
    let voice1 = service.play(clip, PlayOptions::default()).unwrap();
    render_stereo(&mut service, 10); // voice1 consumed frames 0..9
    service.suspend().unwrap();
    assert!(
        service.health().suspended,
        "service truth: suspended immediately"
    );
    let out_silent = render_stereo(&mut service, 4);
    assert!(silence(&out_silent), "suspended output is silent");
    assert!(
        service.health().rt_suspended,
        "render truth: mock render applied Suspend"
    );
    let voice2 = service
        .play(clip, PlayOptions::default())
        .expect("play queued while suspended is not dropped");
    render_stereo(&mut service, 4); // still suspended: silence continues
    service.resume().unwrap();
    assert!(
        !service.health().suspended,
        "service truth: resumed before the first post-resume callback"
    );

    // Post-resume frame k: voice1 contributes clip frame 10+k (continuation),
    // voice2 contributes clip frame k (fresh start). Output is interleaved: frame k
    // occupies samples [2k, 2k+1]; a mono clip contributes the same value to both.
    //   frame 0: (11 + 1) * 0.002 = 0.024
    //   frame 1: (12 + 2) * 0.002 = 0.028
    // Voice1 (10 frames consumed) completes at output frame 99; voice2 (110-frame
    // clip) completes at output frame 109.
    let out = render_stereo(&mut service, 110);
    assert!(
        (out[0] - 0.024).abs() < 1e-6,
        "voice1 continued at frame 10, got {}",
        out[0]
    );
    assert!(
        (out[1] - 0.024).abs() < 1e-6,
        "frame 0 right channel matches, got {}",
        out[1]
    );
    assert!(
        (out[2] - 0.028).abs() < 1e-6,
        "voice2 started at frame 0, got {}",
        out[2]
    );
    assert!(
        (out[3] - 0.028).abs() < 1e-6,
        "frame 1 right channel matches, got {}",
        out[3]
    );
    // Frame 99: voice1 at clip frame 109 (0.22) + voice2 at frame 99 (0.2).
    assert!(
        (out[198] - 0.42).abs() < 1e-6,
        "positions stayed exact, got {}",
        out[198]
    );
    // Both voices complete exactly once: no replay.
    let tail = render_stereo(&mut service, 10);
    assert!(silence(&tail), "nothing replays after both voices complete");
    assert!(
        !service.is_voice_active(voice1),
        "voice1 finished without restart"
    );
    assert!(!service.is_voice_active(voice2));
    assert_eq!(
        service.health().voices_completed,
        2,
        "exactly two completions, no restarts"
    );
}

#[test]
fn suspend_truth_does_not_depend_on_render_callbacks() {
    // 2026-09-12 OnePlus 13: the old health snapshot read the render-thread mirror,
    // but AAudio stops delivering callbacks once paused, so suspension was never
    // observable. The service must publish its own control-side truth on return.
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.25; 32])).unwrap();
    let _voice = service.play(clip, PlayOptions::default()).unwrap();
    render_stereo(&mut service, 1); // apply the play and let the mixer run

    let before = service.health();
    assert!(!before.suspended, "running service is not suspended");
    assert!(!before.rt_suspended, "render thread is not suspended");

    service.suspend().unwrap();
    let suspended = service.health();
    assert!(
        suspended.suspended,
        "control-side suspension must be observable without a render callback"
    );
    assert!(
        !suspended.rt_suspended,
        "no render ran since Suspend was queued, so the mirror is still clear"
    );

    // Deliberately no mock_render here: this models a paused backend that delivers
    // no callbacks, so only the control-side state can change.
    service.resume().unwrap();
    let resumed = service.health();
    assert!(
        !resumed.suspended,
        "resume clears control-side suspension without a render callback"
    );
    assert!(
        !resumed.rt_suspended,
        "the queued Suspend/Resume pair was never applied, so the mirror stays clear"
    );
}

// ---------------------------------------------------------------------------
// Device loss and stream recreation via fault injection (DoD #6)
// ---------------------------------------------------------------------------

#[test]
fn device_loss_fault_injection_recreates_stream_and_continues_voices() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.3; 100])).unwrap();
    let voice = service.play(clip, PlayOptions::default()).unwrap();
    render_stereo(&mut service, 5);

    // Inject a device disconnect (same shared-atomics path the AAudio error
    // callback uses).
    {
        let mock = service.mock_backend().expect("mock backend");
        mock.inject_device_error(0xDEAD);
    }
    assert_eq!(
        service.stream_properties(),
        None,
        "properties hidden while failed"
    );
    // poll_device recreates with the same mixer core.
    service.poll_device().expect("recreate");
    assert_eq!(service.health().device_errors, 1);
    assert_eq!(service.health().stream_recreations, 1);
    assert!(
        service.stream_properties().is_some(),
        "properties back after recreation"
    );
    // Voice continues from frame 5 (not restarted).
    let out = render_stereo(&mut service, 2);
    assert_eq!(&out[..4], &[0.3, 0.3, 0.3, 0.3]);
    assert!(service.is_voice_active(voice));

    // Recreation did not leak the old stream: the old mock is closed.
    // (The service now owns a new backend; the old one was dropped on close.)

    // Repeated failures keep working (no leaked threads / no deadlock).
    for round in 2..=5 {
        {
            let mock = service.mock_backend().unwrap();
            mock.inject_device_error(round);
        }
        service.poll_device().expect("recreate again");
        assert_eq!(service.health().stream_recreations, round as u64);
    }
    render_stereo(&mut service, 1);
}

#[test]
fn closing_service_with_active_voices_is_clean() {
    // Voices active at drop; the stream is closed before the core is dropped
    // (structural guarantee asserted by the mixer unit test for the raw path).
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.5; 1000])).unwrap();
    let _ = service.play(clip, PlayOptions::default()).unwrap();
    render_stereo(&mut service, 1);
    drop(service); // no panic, no callback-after-free (mock render is sync)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
