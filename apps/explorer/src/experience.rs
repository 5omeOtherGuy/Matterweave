//! Normal entry point: choose the wetland, the existing sandbox, or Voxel Relay.
use crate::{voxel_relay::VoxelRelayApp, wetland::WetlandApp, Explorer};
use std::path::PathBuf;
use winit::{
    application::ApplicationHandler, event::WindowEvent, event_loop::ActiveEventLoop,
    window::WindowId,
};

pub struct Experience {
    wetland: Option<WetlandApp>,
    sandbox: Option<Explorer>,
    voxel_relay: Option<VoxelRelayApp>,
    legacy_path: PathBuf,
    frame_limit: Option<u64>,
}
impl Experience {
    pub fn new(legacy_path: PathBuf, auto_enter: bool, frame_limit: Option<u64>) -> Self {
        let directory = crate::data_directory(&legacy_path).to_path_buf();
        Self {
            wetland: Some(WetlandApp::new(directory, auto_enter, frame_limit)),
            sandbox: None,
            voxel_relay: None,
            legacy_path,
            frame_limit,
        }
    }
    pub fn failed(&self) -> bool {
        self.wetland.as_ref().is_some_and(|w| w.failed)
            || self.sandbox.as_ref().is_some_and(|s| s.failed)
            || self.voxel_relay.as_ref().is_some_and(|r| r.failed)
    }
    fn switch_if_requested(&mut self, event_loop: &ActiveEventLoop) {
        if self.wetland.as_ref().is_some_and(|w| w.sandbox_requested) {
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
            if let Some(mut wetland) = self.wetland.take() {
                wetland.suspended(event_loop);
            }
            let mut relay = VoxelRelayApp::new(self.legacy_path.clone(), self.frame_limit);
            relay.resumed(event_loop);
            self.voxel_relay = Some(relay);
        } else if self.voxel_relay.as_ref().is_some_and(|r| r.return_to_menu) {
            if let Some(mut relay) = self.voxel_relay.take() {
                relay.suspended(event_loop);
            }
            let directory = crate::data_directory(&self.legacy_path).to_path_buf();
            let mut wetland = WetlandApp::new(directory, false, self.frame_limit);
            wetland.resumed(event_loop);
            self.wetland = Some(wetland);
        }
    }
}
impl ApplicationHandler for Experience {
    fn resumed(&mut self, e: &ActiveEventLoop) {
        if let Some(w) = &mut self.wetland {
            w.resumed(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.resumed(e);
        }
        if let Some(r) = &mut self.voxel_relay {
            r.resumed(e);
        }
    }
    fn suspended(&mut self, e: &ActiveEventLoop) {
        if let Some(w) = &mut self.wetland {
            w.suspended(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.suspended(e);
        }
        if let Some(r) = &mut self.voxel_relay {
            r.suspended(e);
        }
    }
    fn window_event(&mut self, e: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(w) = &mut self.wetland {
            w.window_event(e, id, event);
        } else if let Some(s) = &mut self.sandbox {
            s.window_event(e, id, event);
        } else if let Some(r) = &mut self.voxel_relay {
            r.window_event(e, id, event);
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
    }
}
