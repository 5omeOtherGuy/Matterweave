//! Shared typed frame capture for the opt-in samples.
//!
//! `metrics::FrameLog` owns the schema and the row format; this wrapper owns the
//! identity bookkeeping every sample needs to fill it honestly:
//!
//! - a draw attempt advances for **every** recorded attempt, including retries,
//!   so two rows never share an attempt id;
//! - a renderer epoch advances on every renderer creation, so a counter reset is
//!   visible rather than read as a duplicate;
//! - `presented_count` counts successful present calls only, and only rows whose
//!   result is `presented` advance it;
//! - a GPU completion is joined to a submission **this capture recorded**, never
//!   to one it did not observe.
//!
//! `label` names the sample in log lines; it never reaches a CSV cell. Scene
//! identity belongs in the accompanying manifest, and a field this sample does
//! not measure stays an empty cell rather than borrowing another sample's number.
use crate::metrics::{self, FrameLog, FrameRow};
use matterweave_render::{FrameResult, Renderer};
use std::{path::Path, time::Instant};

pub struct Capture {
    log: Option<FrameLog>,
    /// Sample name for log messages only.
    label: &'static str,
    epoch: u64,
    attempt: u64,
    presented: u64,
    completions: metrics::GpuCompletionTracker,
    last_submission: Option<(u64, u64)>,
}

impl Capture {
    /// Takes the pending capture request in `directory`, if any.
    pub fn new(label: &'static str, directory: &Path) -> Self {
        let log = FrameLog::requested(directory).unwrap_or_else(|e| {
            log::error!("{label} capture request: {e}");
            None
        });
        Self {
            log,
            label,
            epoch: 0,
            attempt: 0,
            presented: 0,
            completions: Default::default(),
            last_submission: None,
        }
    }

    pub fn enabled(&self) -> bool {
        self.log.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.log.as_ref().map(|log| log.path.as_path())
    }

    /// One renderer creation or recreation: a new epoch and the diagnostics the
    /// row's optional timing fields need.
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
        let diagnostics = renderer.draw_diagnostics();
        let submission = diagnostics
            .and_then(|d| d.submitted_frame_id)
            .map(|id| (self.epoch, id));
        // The renderer has one in-flight frame. Only join a completion to a
        // submission actually recorded by this capture: a sample can render
        // before its first row (a menu, or a warm-up frame), and that
        // submission's id must not be read as this row's predecessor.
        let gpu = renderer.gpu_timings().filter(|g| {
            let identity = (self.epoch, g.frame_id);
            (self.last_submission == Some(identity) || submission == Some(identity))
                && self.completions.accept(self.epoch, g.frame_id)
        });
        if submission.is_some() {
            self.last_submission = submission;
        }
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
        // Shadow work belongs only to an attempt that submitted; a retry must not
        // inherit the previous pass's caster count.
        row.shadow_caster_meshes = metrics::shadow_casters_for_attempt(
            row.submitted_gpu_frame_id,
            u32::try_from(renderer.shadow_caster_meshes()).ok(),
        );
        match self.log.as_mut().unwrap().record(&row) {
            Ok(true) => {}
            result => {
                if let Err(e) = result {
                    log::error!("{} capture write: {e}", self.label);
                }
                if let Some(mut log) = self.log.take() {
                    let _ = log.flush();
                    log::info!("{} capture ended: {}", self.label, log.path.display());
                }
                renderer.set_diagnostics_enabled(false);
            }
        }
    }

    pub fn flush(&mut self) {
        if let Some(log) = &mut self.log {
            if let Err(e) = log.flush() {
                log::error!("{} capture flush: {e}", self.label);
            }
        }
    }
}
