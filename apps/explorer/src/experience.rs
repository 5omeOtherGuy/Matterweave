//! Normal entry point: choose the wetland or the existing sandbox.
use crate::{wetland::WetlandApp, Explorer};
use std::path::PathBuf;
use winit::{
    application::ApplicationHandler, event::WindowEvent, event_loop::ActiveEventLoop,
    window::WindowId,
};

pub struct Experience {
    wetland: Option<WetlandApp>,
    sandbox: Option<Explorer>,
    legacy_path: PathBuf,
    frame_limit: Option<u64>,
}
impl Experience {
    pub fn new(legacy_path: PathBuf, auto_enter: bool, frame_limit: Option<u64>) -> Self {
        let directory = crate::data_directory(&legacy_path).to_path_buf();
        Self {
            wetland: Some(WetlandApp::new(directory, auto_enter, frame_limit)),
            sandbox: None,
            legacy_path,
            frame_limit,
        }
    }
    pub fn failed(&self) -> bool {
        self.wetland.as_ref().is_some_and(|w| w.failed)
            || self.sandbox.as_ref().is_some_and(|s| s.failed)
    }
    fn switch_if_requested(&mut self, event_loop: &ActiveEventLoop) {
        if self.wetland.as_ref().is_some_and(|w| w.sandbox_requested) {
            if let Some(mut wetland) = self.wetland.take() {
                wetland.suspended(event_loop);
            }
            let mut sandbox = Explorer::new(self.legacy_path.clone(), self.frame_limit);
            sandbox.resumed(event_loop);
            self.sandbox = Some(sandbox);
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
    }
    fn suspended(&mut self, e: &ActiveEventLoop) {
        if let Some(w) = &mut self.wetland {
            w.suspended(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.suspended(e);
        }
    }
    fn window_event(&mut self, e: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(w) = &mut self.wetland {
            w.window_event(e, id, event);
        } else if let Some(s) = &mut self.sandbox {
            s.window_event(e, id, event);
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
    }
    fn exiting(&mut self, e: &ActiveEventLoop) {
        if let Some(w) = &mut self.wetland {
            w.exiting(e);
        }
        if let Some(s) = &mut self.sandbox {
            s.exiting(e);
        }
    }
}
