//! The wetland uses the same typed capture schema as the sandbox. Scene identity
//! belongs in the accompanying manifest; unavailable timing fields stay empty.
use crate::metrics::{self, FrameLog, FrameRow};
use matterweave_render::{FrameResult, Renderer};
use std::{path::Path, time::Instant};

pub struct Capture {
    log: Option<FrameLog>,
    epoch: u64,
    attempt: u64,
    presented: u64,
    completions: metrics::GpuCompletionTracker,
}
impl Capture {
    pub fn new(directory: &Path) -> Self {
        let log = FrameLog::requested(directory).unwrap_or_else(|e| {
            log::error!("Wetland capture request: {e}");
            None
        });
        Self {
            log,
            epoch: 0,
            attempt: 0,
            presented: 0,
            completions: Default::default(),
        }
    }
    pub fn enabled(&self) -> bool {
        self.log.is_some()
    }
    pub fn renderer_created(&mut self, renderer: &mut Renderer) {
        self.epoch += 1;
        renderer.set_diagnostics_enabled(self.enabled());
    }
    pub fn record(
        &mut self,
        renderer: &mut Renderer,
        result: &FrameResult,
        mut row: FrameRow,
        start: Instant,
        cpu: metrics::CpuBusySpan,
    ) {
        if self.log.is_none() {
            return;
        }
        self.attempt += 1;
        if matches!(result, FrameResult::Presented) {
            self.presented += 1;
        }
        let gpu = renderer
            .gpu_timings()
            .filter(|g| self.completions.accept(self.epoch, g.frame_id));
        let diagnostics = renderer.draw_diagnostics();
        row.draw_attempt_id = self.attempt;
        row.renderer_epoch = self.epoch;
        row.presented_count = self.presented;
        row.result = match result {
            FrameResult::Presented => metrics::DrawOutcome::Presented,
            FrameResult::OutOfMemory => metrics::DrawOutcome::OutOfMemory,
            _ => metrics::DrawOutcome::Retry,
        };
        row.main_wall_ms = Some(start.elapsed().as_secs_f64() * 1000.);
        row.main_cpu_busy_ms = cpu.finish();
        row.submitted_gpu_frame_id = diagnostics.and_then(|d| d.submitted_frame_id);
        row.completed_gpu_frame_id = gpu.map(|g| g.frame_id);
        row.completed_gpu_renderer_epoch = gpu.map(|_| self.epoch);
        row.gpu_prev_render_ms = gpu.map(|g| g.render_ms);
        row.gpu_prev_shadow_ms = gpu.and_then(|g| g.shadow_ms);
        row.gpu_prev_shadows = gpu.map(|g| g.shadows);
        row.gpu_prev_shadow_map_size = gpu.map(|g| g.shadow_map_size);
        row.mesh_sync_fence_wait_wall_ms = diagnostics.and_then(|d| d.upload_fence_wait_ms);
        row.mesh_sync_fence_waits = diagnostics.and_then(|d| d.upload_fence_waits);
        row.render_fence_wait_wall_ms = diagnostics.and_then(|d| d.render_fence_wait_ms);
        row.acquire_wall_ms = diagnostics.and_then(|d| d.acquire_ms);
        row.present_wall_ms = diagnostics.and_then(|d| d.present_ms);
        row.shadow_caster_meshes = metrics::shadow_casters_for_attempt(
            row.submitted_gpu_frame_id,
            u32::try_from(renderer.shadow_caster_meshes()).ok(),
        );
        match self.log.as_mut().unwrap().record(&row) {
            Ok(true) => {}
            result => {
                if let Err(e) = result {
                    log::error!("Wetland capture write: {e}");
                }
                if let Some(mut log) = self.log.take() {
                    let _ = log.flush();
                    log::info!("Wetland capture ended: {}", log.path.display());
                }
                renderer.set_diagnostics_enabled(false);
            }
        }
    }
    pub fn flush(&mut self) {
        if let Some(log) = &mut self.log {
            if let Err(e) = log.flush() {
                log::error!("Wetland capture flush: {e}");
            }
        }
    }
}
