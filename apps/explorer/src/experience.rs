//! Normal entry point: choose the wetland, the existing sandbox, or Voxel Relay.
//!
//! [`Experience`] owns the one shared [`AudioAdapter`]. Samples queue plain
//! [`GameplayEvent`](crate::audio_service::GameplayEvent)s in a bounded [`EventQueue`](crate::audio_service::EventQueue);
//! the app drains the active sample once per frame, so neither sample opens a
//! device or owns a service.
use crate::{
    audio_service::{AudioAdapter, AudioScope},
    settings::{SharedSettings, SETTINGS_FILE_NAME},
    terrain_lab::TerrainLab,
    voxel_relay::VoxelRelayApp,
    wetland::WetlandApp,
    Explorer,
};
use std::path::PathBuf;
use winit::{
    application::ApplicationHandler, event::WindowEvent, event_loop::ActiveEventLoop,
    window::WindowId,
};

pub struct Experience {
    wetland: Option<WetlandApp>,
    sandbox: Option<Explorer>,
    voxel_relay: Option<VoxelRelayApp>,
    terrain_lab: Option<TerrainLab>,
    legacy_path: PathBuf,
    frame_limit: Option<u64>,
    /// The one audio adapter shared by every sample. No device is opened here;
    /// `resumed` opens it, and a sample launched without an `Experience` never
    /// opens one at all (its bounded queue is simply never drained).
    audio: AudioAdapter,
    /// The shared, bounded preferences every sample renders. This owner is the
    /// only writer of the settings file and the only audio-policy decision point.
    settings: SharedSettings,
    /// Sibling of `world.json`, never a game save.
    settings_path: PathBuf,
    /// True while the window has focus. Audio is only active when this and
    /// `resumed` are both true; focus gain alone never overrides a suspended
    /// lifecycle.
    focused: bool,
    /// True between the lifecycle `resumed` and `suspended` callbacks. Android
    /// pauses a backgrounded app without a focus event, so this is tracked
    /// separately from window focus.
    resumed: bool,
    /// Failure counters already reported, so a persistent failure is logged once
    /// per change instead of once per frame. `(device, registration)`.
    audio_failures_seen: (u64, u64),
}
impl Experience {
    pub fn new(legacy_path: PathBuf, auto_enter: bool, frame_limit: Option<u64>) -> Self {
        let directory = crate::data_directory(&legacy_path).to_path_buf();
        let settings_path = directory.join(SETTINGS_FILE_NAME);
        let settings = SharedSettings::load(&settings_path);
        let mut wetland = WetlandApp::new(directory, auto_enter, frame_limit);
        wetland.set_shared_settings(settings);
        let mut audio = AudioAdapter::new(AudioScope::Wetland);
        // Mute is owner policy, not a gameplay event: apply it before any sample
        // can queue feedback.
        audio.set_muted(settings.muted);
        Self {
            wetland: Some(wetland),
            sandbox: None,
            voxel_relay: None,
            terrain_lab: None,
            legacy_path,
            frame_limit,
            audio,
            settings,
            settings_path,
            // The samples treat the initial resume as focused; a real focus-loss
            // event then takes it away.
            focused: true,
            resumed: false,
            audio_failures_seen: (0, 0),
        }
    }
    pub fn failed(&self) -> bool {
        self.wetland.as_ref().is_some_and(|w| w.failed)
            || self.sandbox.as_ref().is_some_and(|s| s.failed)
            || self.voxel_relay.as_ref().is_some_and(|r| r.failed)
            || self.terrain_lab.as_ref().is_some_and(|t| t.failed)
    }
    /// Drop every queued gameplay event from whichever sample is active.
    ///
    /// Called on focus loss, suspension and scope switches: feedback produced
    /// before a pause or a switch must not play afterwards.
    fn clear_sample_events(&mut self) {
        if let Some(wetland) = &mut self.wetland {
            wetland.clear_events();
        } else if let Some(relay) = &mut self.voxel_relay {
            relay.clear_events();
        }
    }
    /// Retire the leaving scope before the arriving one can play anything: drop
    /// its queued events and queue stops for its sounding voices.
    fn retire_leaving_scope(&mut self, arriving: AudioScope) {
        self.clear_sample_events();
        self.audio.switch_to(arriving);
    }
    /// Enter the arriving scope for a pending switch: flush the leaving
    /// sample's one-shot preference change, then retire its audio scope.
    ///
    /// [`Self::sync_settings`] normally runs in `about_to_wait`, but a scope
    /// switch is processed in `window_event`: a toggle, DONE and a launch tap
    /// can share one event batch. Without this flush the leaving sample is
    /// dropped with its change unpersisted, mute unapplied and the arriving
    /// sample stale.
    fn begin_scope_switch(&mut self, arriving: AudioScope) {
        self.sync_settings();
        self.retire_leaving_scope(arriving);
    }
    /// True while both the lifecycle and the window allow playback.
    fn audio_active(&self) -> bool {
        self.resumed && self.focused
    }
    /// Focus loss silences playback immediately: queued events are dropped and
    /// sounding voices are stopped and suspended, not frozen to resume later.
    fn on_focus_lost(&mut self) {
        self.focused = false;
        self.clear_sample_events();
        self.audio.stop_all();
        self.audio.suspend();
    }
    /// Focus gain restores audio only when the lifecycle is also resumed. It must
    /// never undo a lifecycle suspension that arrived while unfocused.
    fn on_focus_gained(&mut self) {
        self.focused = true;
        if self.resumed {
            self.audio.resume();
        }
    }
    /// Per-frame audio pump: recover a lost device, then play whatever the active
    /// sample queued since the last frame.
    ///
    /// An app that is not both resumed and focused never polls or plays; queued
    /// events are dropped instead of accumulating behind the pause. Events the
    /// service refuses (voice pressure, command backpressure, no device) are
    /// counted by the adapter and keep the app running.
    fn pump_audio(&mut self) {
        if !self.audio_active() || self.audio.is_suspended() {
            self.clear_sample_events();
            return;
        }
        self.audio.poll_device();
        loop {
            let event = if let Some(wetland) = &mut self.wetland {
                wetland.pop_event()
            } else if let Some(relay) = &mut self.voxel_relay {
                relay.pop_event()
            } else {
                None
            };
            let Some(event) = event else {
                break;
            };
            let outcome = self.audio.trigger(event);
            if let Some(reason) = outcome.dropped() {
                log::debug!("audio dropped {}: {reason:?}", event.name());
            }
            let status = self.audio.status();
            log::info!(
                "Sample audio {:?}: event={} outcome={outcome:?} callbacks={} frames={}",
                status.scope,
                event.name(),
                status.callback_count,
                status.frames_rendered,
            );
        }
        self.report_audio_failures();
    }
    /// Keep adapter failures visible without spamming the log. Audio failure is
    /// never fatal: the app runs silently with `last_error` recorded.
    fn report_audio_failures(&mut self) {
        let status = self.audio.status();
        let seen = (
            status.counters.device_failures,
            status.counters.registration_failures,
        );
        if seen == self.audio_failures_seen {
            return;
        }
        self.audio_failures_seen = seen;
        match &status.last_error {
            Some(error) => log::warn!(
                "audio degraded (device failures {}, registration failures {}): {error}",
                seen.0,
                seen.1
            ),
            None => log::warn!("audio reported failures without a recorded error ({seen:?})"),
        }
    }
    /// Take the one preference change the live sample queued, persist it and
    /// apply the owner's audio policy. The file is written only on a real
    /// change, so an open menu never rewrites the disk every frame.
    fn sync_settings(&mut self) {
        let change = self
            .wetland
            .as_mut()
            .and_then(WetlandApp::take_settings_change)
            .or_else(|| {
                self.voxel_relay
                    .as_mut()
                    .and_then(VoxelRelayApp::take_settings_change)
            });
        let Some(settings) = change else {
            return;
        };
        if settings == self.settings {
            return;
        }
        self.settings = settings;
        self.audio.set_muted(self.settings.muted);
        if let Err(error) = self.settings.save(&self.settings_path) {
            log::warn!(
                "Shared settings could not be saved to {}: {error}",
                self.settings_path.display()
            );
        }
    }
    fn switch_if_requested(&mut self, event_loop: &ActiveEventLoop) {
        if self.wetland.as_ref().is_some_and(|w| w.sandbox_requested) {
            self.begin_scope_switch(AudioScope::Sandbox);
            if let Some(mut wetland) = self.wetland.take() {
                wetland.suspended(event_loop);
            }
            let mut sandbox = Explorer::new(self.legacy_path.clone(), self.frame_limit);
            sandbox.resumed(event_loop);
            self.sandbox = Some(sandbox);
        } else if self
            .wetland
            .as_ref()
            .is_some_and(|w| w.voxel_relay_requested)
        {
            self.begin_scope_switch(AudioScope::VoxelRelay);
            if let Some(mut wetland) = self.wetland.take() {
                wetland.suspended(event_loop);
            }
            let mut relay = VoxelRelayApp::for_chooser(&self.legacy_path, self.frame_limit);
            relay.set_shared_settings(self.settings);
            relay.resumed(event_loop);
            self.voxel_relay = Some(relay);
        } else if self
            .wetland
            .as_ref()
            .is_some_and(|w| w.terrain_lab_requested)
        {
            self.begin_scope_switch(AudioScope::Sandbox);
            if let Some(mut wetland) = self.wetland.take() {
                wetland.suspended(event_loop);
            }
            let directory = crate::data_directory(&self.legacy_path).to_path_buf();
            let mut lab = TerrainLab::new(directory, self.frame_limit);
            lab.resumed(event_loop);
            self.terrain_lab = Some(lab);
        } else if self.terrain_lab.as_ref().is_some_and(|t| t.return_to_menu) {
            self.begin_scope_switch(AudioScope::Wetland);
            if let Some(mut lab) = self.terrain_lab.take() {
                lab.suspended(event_loop);
            }
            let directory = crate::data_directory(&self.legacy_path).to_path_buf();
            let mut wetland = WetlandApp::new(directory, false, self.frame_limit);
            wetland.set_shared_settings(self.settings);
            wetland.resumed(event_loop);
            self.wetland = Some(wetland);
        } else if self.voxel_relay.as_ref().is_some_and(|r| r.return_to_menu) {
            self.begin_scope_switch(AudioScope::Wetland);
            if let Some(mut relay) = self.voxel_relay.take() {
                relay.suspended(event_loop);
            }
            let directory = crate::data_directory(&self.legacy_path).to_path_buf();
            let mut wetland = WetlandApp::new(directory, false, self.frame_limit);
            wetland.set_shared_settings(self.settings);
            wetland.resumed(event_loop);
            self.wetland = Some(wetland);
        }
    }
}
impl ApplicationHandler for Experience {
    fn resumed(&mut self, e: &ActiveEventLoop) {
        // The one place the output device is opened, and only when the window is
        // focused too. A failed open leaves the app running silently and retries
        // here on the next resume.
        self.resumed = true;
        if self.focused {
            self.audio.resume();
        }
        // Re-assert the persisted policy after every resume; the adapter itself
        // keeps its mute across sample switches and lifecycle pauses.
        self.audio.set_muted(self.settings.muted);
        if let Some(w) = &mut self.wetland {
            w.resumed(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.resumed(e);
        }
        if let Some(r) = &mut self.voxel_relay {
            r.resumed(e);
        }
        if let Some(t) = &mut self.terrain_lab {
            t.resumed(e);
        }
    }
    fn suspended(&mut self, e: &ActiveEventLoop) {
        // Refuse new events before the samples tear down, stop sounding voices
        // and drop anything already queued: nothing from before the pause may
        // sound on resume.
        self.resumed = false;
        self.audio.stop_all();
        self.audio.suspend();
        if let Some(w) = &mut self.wetland {
            w.suspended(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.suspended(e);
        }
        if let Some(r) = &mut self.voxel_relay {
            r.suspended(e);
        }
        if let Some(t) = &mut self.terrain_lab {
            t.suspended(e);
        }
        self.clear_sample_events();
    }
    fn window_event(&mut self, e: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let focus = match event {
            WindowEvent::Focused(focused) => Some(focused),
            _ => None,
        };
        if let Some(w) = &mut self.wetland {
            w.window_event(e, id, event);
        } else if let Some(s) = &mut self.sandbox {
            s.window_event(e, id, event);
        } else if let Some(r) = &mut self.voxel_relay {
            r.window_event(e, id, event);
        } else if let Some(t) = &mut self.terrain_lab {
            t.window_event(e, id, event);
        }
        match focus {
            Some(false) => self.on_focus_lost(),
            Some(true) => self.on_focus_gained(),
            None => {}
        }
        self.switch_if_requested(e);
    }
    fn about_to_wait(&mut self, e: &ActiveEventLoop) {
        if let Some(w) = &mut self.wetland {
            w.about_to_wait(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.about_to_wait(e);
        }
        if let Some(r) = &mut self.voxel_relay {
            r.about_to_wait(e);
        }
        if let Some(t) = &mut self.terrain_lab {
            t.about_to_wait(e);
        }
        self.sync_settings();
        self.pump_audio();
    }
    fn exiting(&mut self, e: &ActiveEventLoop) {
        if let Some(w) = &mut self.wetland {
            w.exiting(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.exiting(e);
        }
        if let Some(r) = &mut self.voxel_relay {
            r.exiting(e);
        }
        if let Some(t) = &mut self.terrain_lab {
            t.exiting(e);
        }
        self.audio.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_service::{DropReason, GameplayEvent, TriggerOutcome};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "matterweave-experience-{}-{}-{name}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// An `Experience` in the state the app is in while running and focused, with
    /// a silent adapter: the owner state can be driven without a window or event
    /// loop.
    fn experience() -> Experience {
        Experience {
            wetland: None,
            sandbox: None,
            voxel_relay: None,
            terrain_lab: None,
            legacy_path: temp_path("world.json"),
            frame_limit: None,
            audio: AudioAdapter::new(AudioScope::Wetland),
            settings: SharedSettings::default(),
            settings_path: temp_path("matterweave-settings.json"),
            focused: true,
            resumed: true,
            audio_failures_seen: (0, 0),
        }
    }

    /// A relay holding one real queued gameplay event: a successful obstacle edit.
    fn relay_with_queued_edit() -> VoxelRelayApp {
        let mut relay = VoxelRelayApp::new(temp_path("relay.json"), None);
        assert!(
            relay.physics.teleport([6.0, 2.55, 13.8]),
            "the corridor pose must be collision-free"
        );
        assert!(relay.try_remove_obstacle());
        relay
    }

    #[test]
    fn no_device_before_resume_and_one_adapter_across_lifecycle() {
        let mut experience = experience();
        assert!(!experience.audio.is_available());
        assert_eq!(
            experience.audio.trigger(GameplayEvent::BlockEdit),
            TriggerOutcome::Dropped(DropReason::NoDevice),
            "construction must not open a device"
        );

        experience.audio.resume();
        assert!(experience.audio.is_available());
        let opened = experience.audio.status().registered_clips;
        assert_eq!(opened, GameplayEvent::ALL.len());

        experience.audio.suspend();
        assert_eq!(
            experience.audio.trigger(GameplayEvent::BlockEdit),
            TriggerOutcome::Dropped(DropReason::Suspended)
        );
        experience.audio.resume();
        assert!(experience.audio.is_available());
        assert_eq!(
            experience.audio.status().registered_clips,
            opened,
            "resume must reuse the same adapter and device, not replace either"
        );
        assert_eq!(experience.audio.status().counters.device_failures, 0);
    }

    #[test]
    fn pump_plays_active_sample_events_and_suspension_drops_them() {
        let mut experience = experience();
        experience.audio.resume();
        experience.audio.switch_to(AudioScope::VoxelRelay);
        experience.voxel_relay = Some(relay_with_queued_edit());

        experience.pump_audio();
        let played = experience.audio.status();
        assert_eq!(played.counters.triggered, 1);
        assert_eq!(played.counters.started, 1);
        assert_eq!(played.tracked_voices, 1);
        assert!(
            experience
                .voxel_relay
                .as_mut()
                .unwrap()
                .pop_event()
                .is_none(),
            "the pump must drain the active sample"
        );

        // A suspended app must neither poll nor play; a stray queued event is
        // dropped, not held for resume.
        experience.audio.suspend();
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.pump_audio();
        let backgrounded = experience.audio.status();
        assert_eq!(
            backgrounded.counters.triggered, 1,
            "a suspended pump must trigger nothing"
        );
        assert!(
            experience
                .voxel_relay
                .as_mut()
                .unwrap()
                .pop_event()
                .is_none(),
            "backgrounded events must be dropped, not queued behind the pause"
        );
    }

    #[test]
    fn focus_loss_silences_and_focus_gain_cannot_override_lifecycle_suspension() {
        let mut experience = experience();
        experience.audio.resume();
        experience.audio.switch_to(AudioScope::VoxelRelay);
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.pump_audio();
        assert_eq!(experience.audio.status().tracked_voices, 1);

        // Focus loss stops the sounding voice (it is not frozen to resume later),
        // drops queued events and refuses new playback.
        experience.on_focus_lost();
        assert!(!experience.focused);
        assert!(experience.audio.is_suspended());
        assert_eq!(
            experience.audio.status().tracked_voices,
            0,
            "focus loss must stop sounding voices, not freeze them"
        );
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.pump_audio();
        assert_eq!(
            experience.audio.status().counters.triggered,
            1,
            "an unfocused pump must trigger nothing"
        );

        // Focus regained while the lifecycle is suspended must not resume audio.
        experience.resumed = false;
        experience.on_focus_gained();
        assert!(experience.focused);
        assert!(
            experience.audio.is_suspended(),
            "focus gain must not override a lifecycle suspension"
        );
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.pump_audio();
        assert_eq!(experience.audio.status().counters.triggered, 1);

        // A lifecycle resume while focused restores audio on the same adapter.
        experience.resumed = true;
        experience.on_focus_gained();
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.pump_audio();
        let restored = experience.audio.status();
        assert_eq!(restored.counters.triggered, 2);
        assert_eq!(restored.counters.started, 2);
        assert_eq!(restored.tracked_voices, 1);
        assert_eq!(restored.counters.device_failures, 0);
    }

    #[test]
    fn scope_switch_drops_leaving_events_and_retires_its_voices_first() {
        let mut experience = experience();
        experience.audio.resume();
        // The relay is the arriving sample: its scope is set before it plays.
        experience.audio.switch_to(AudioScope::VoxelRelay);
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.pump_audio();
        assert_eq!(experience.audio.status().tracked_voices, 1);

        // A real event queued after the pump must not survive the scope switch.
        experience.voxel_relay = Some(relay_with_queued_edit());
        experience.retire_leaving_scope(AudioScope::Wetland);
        assert_eq!(
            experience.audio.status().tracked_voices,
            0,
            "the leaving scope's voices must be retired at the switch"
        );
        assert!(
            experience
                .voxel_relay
                .as_mut()
                .unwrap()
                .pop_event()
                .is_none(),
            "the leaving scope's queued events must be dropped at the switch"
        );

        // The arriving scope plays on the same adapter.
        assert_eq!(
            experience.audio.trigger(GameplayEvent::Objective),
            TriggerOutcome::Started
        );
        assert_eq!(experience.audio.status().scope, AudioScope::Wetland);
        assert_eq!(experience.audio.status().counters.device_failures, 0);
    }

    #[test]
    fn sample_settings_change_propagates_mute_and_persists() {
        let mut experience = experience();
        experience.audio.resume();
        let mut relay = VoxelRelayApp::new(temp_path("relay-settings.json"), None);
        relay.set_shared_settings(experience.settings);
        experience.audio.switch_to(AudioScope::VoxelRelay);
        experience.voxel_relay = Some(relay);

        // Drive the real overlay path: the registered SETTINGS zone opens the
        // panel, then the SOUND row toggles mute.
        let settings_hit = {
            let layout = crate::controls::relay_layout(&experience.settings);
            [layout.settings[0] + 8., layout.settings[1] + 8.]
        };
        let mute_row = crate::controls::settings_panel().rows[2].0;
        let mute_click = [
            mute_row[0] + mute_row[2] / 2.,
            mute_row[1] + mute_row[3] / 2.,
        ];
        let toggle_mute = |experience: &mut Experience| {
            let relay = experience.voxel_relay.as_mut().unwrap();
            if !relay.settings_open {
                assert_eq!(relay.input.pointer_down(1, settings_hit), Some(5));
                relay.activate_button(5);
            }
            relay.settings_click(mute_click);
        };

        toggle_mute(&mut experience);
        experience.sync_settings();
        assert!(experience.settings.muted);
        assert!(
            experience.audio.status().muted,
            "mute must reach the one adapter"
        );
        assert_eq!(
            SharedSettings::load(&experience.settings_path),
            experience.settings,
            "the change must survive a restart through the settings file"
        );
        assert_eq!(
            experience.audio.trigger(GameplayEvent::BlockEdit),
            TriggerOutcome::Dropped(DropReason::Muted),
            "muted playback must not start a voice"
        );

        // Unmuting is the same path in reverse and reopens nothing.
        let device_failures = experience.audio.status().counters.device_failures;
        toggle_mute(&mut experience);
        experience.sync_settings();
        assert!(!experience.settings.muted);
        assert!(!experience.audio.status().muted);
        assert_eq!(
            experience.audio.status().counters.device_failures,
            device_failures,
            "the mute toggle must not disturb the open device"
        );

        // A sample switch keeps the adapter and the last mute policy.
        toggle_mute(&mut experience);
        experience.sync_settings();
        assert!(experience.settings.muted);
        experience.retire_leaving_scope(AudioScope::Sandbox);
        assert!(experience.audio.status().muted);
        assert!(experience.settings_path.exists());
    }

    #[test]
    fn same_batch_preference_change_survives_the_scope_switch_without_about_to_wait() {
        // Regression: `switch_if_requested` runs in `window_event`, but
        // `sync_settings` only ran in `about_to_wait`. A toggle, DONE and a
        // sample-launch tap can share one event batch, so the leaving sample
        // was retired and taken with its change still pending: nothing was
        // persisted, mute was not applied and the arriving sample was stale.
        let mut experience = experience();
        experience.audio.resume();
        let mut relay = VoxelRelayApp::new(temp_path("relay-same-batch.json"), None);
        relay.set_shared_settings(experience.settings);
        experience.audio.switch_to(AudioScope::VoxelRelay);
        experience.voxel_relay = Some(relay);

        // The real HUD path, in one batch with no `about_to_wait`: the SETTINGS
        // zone opens the panel, the CONTROL SIZE and SOUND rows toggle, DONE
        // closes the overlay and the MENU zone queues the scope switch.
        let panel = crate::controls::settings_panel();
        let center = |rect: [f32; 4]| [rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.];
        let settings_hit = {
            let layout = crate::controls::relay_layout(&experience.settings);
            [layout.settings[0] + 8., layout.settings[1] + 8.]
        };
        let menu_hit = {
            let layout = crate::controls::relay_layout(&experience.settings);
            [layout.menu[0] + 8., layout.menu[1] + 8.]
        };
        {
            let relay = experience.voxel_relay.as_mut().unwrap();
            assert_eq!(relay.input.pointer_down(1, settings_hit), Some(5));
            relay.activate_button(5);
            assert!(relay.settings_open);
            relay.settings_click(center(panel.rows[1].0));
            relay.settings_click(center(panel.rows[2].0));
            relay.settings_click(center(panel.done));
            assert!(!relay.settings_open, "DONE closes the overlay");
            assert_eq!(relay.input.pointer_down(2, menu_hit), Some(4));
            relay.activate_button(4);
            assert!(relay.return_to_menu, "the launch tap queues the switch");
        }
        // Nothing has synced yet: this is the state the same batch leaves
        // behind when `switch_if_requested` starts.
        assert_eq!(experience.settings, SharedSettings::default());
        assert!(!experience.settings_path.exists());
        assert!(!experience.audio.status().muted);

        // The production switch prelude must flush the queued change before the
        // leaving sample can be retired or taken.
        experience.begin_scope_switch(AudioScope::Wetland);
        let expected = SharedSettings {
            large_controls: true,
            muted: true,
            ..SharedSettings::default()
        };
        assert_eq!(
            experience.settings, expected,
            "the owner must take the queued change before the sample leaves"
        );
        assert_eq!(
            SharedSettings::load(&experience.settings_path),
            expected,
            "the change must survive a restart through the settings file"
        );
        assert!(
            experience.audio.status().muted,
            "mute must reach the one audio owner at the switch"
        );
        assert_eq!(experience.audio.status().scope, AudioScope::Wetland);
        assert_eq!(experience.audio.status().counters.device_failures, 0);

        // Every `switch_if_requested` branch installs the arriving sample from
        // the owner's settings with `set_shared_settings(self.settings)`; a
        // fresh relay makes the installed preferences observable through its
        // configured move zone.
        let expected_layout = crate::controls::relay_layout(&expected);
        assert_ne!(
            expected_layout.move_zone,
            crate::controls::relay_layout(&SharedSettings::default()).move_zone,
            "the toggled preference must change the layout for this check to bite"
        );
        let mut arriving =
            VoxelRelayApp::for_chooser(&experience.legacy_path, experience.frame_limit);
        arriving.set_shared_settings(experience.settings);
        assert_eq!(
            arriving.input.move_zone(),
            Some((expected_layout.move_zone, 70.0)),
            "the arriving sample must receive the flushed preferences, not the stale ones"
        );
    }
}
