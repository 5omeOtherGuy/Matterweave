//! Bounded, opt-in developer frame capture (CSV schema v2).
//!
//! Rows are typed: identities stay exact integers and are never routed through
//! `f64`; durations are milliseconds; unavailable or invalid measurements stay
//! empty cells. Wall times are elapsed time, not proof of work; `main_cpu_busy_ms`
//! is the only busy-time field and only exists where a thread CPU clock is
//! supported. See `docs/performance/measurement-v2.md`.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_FRAMES: u32 = 240_000;

/// Capture schema version. Increment for any column meaning/order change.
pub const SCHEMA_VERSION: u32 = 2;

/// Column order of a v2 capture; `FrameRow::write` writes exactly these cells.
pub const COLUMNS: &[&str] = &[
    "draw_attempt_id",
    "renderer_epoch",
    "presented_count",
    "result",
    "submitted_gpu_frame_id",
    "completed_gpu_frame_id",
    "completed_gpu_renderer_epoch",
    "draw_interval_wall_ms",
    "main_wall_ms",
    "main_cpu_busy_ms",
    "stream_request_elapsed_ms",
    "physics_wall_ms",
    "mesh_sync_wall_ms",
    "mesh_sync_fence_wait_wall_ms",
    "dynamic_mesh_build_wall_ms",
    "dynamic_upload_wall_ms",
    "render_wall_ms",
    "save_wall_ms",
    "render_fence_wait_wall_ms",
    "acquire_wall_ms",
    "present_wall_ms",
    "gpu_prev_render_ms",
    "gpu_prev_shadow_ms",
    "gpu_prev_shadows",
    "gpu_prev_shadow_map_size",
    "mesh_sync_fence_waits",
    "physics_fixed_steps",
    "voxel_bodies_total",
    "voxel_bodies_active",
    "voxel_bodies_sleeping",
    "voxel_bodies_not_simulated",
    "chunk_mesh_uploads",
    "dynamic_mesh_builds",
    "dynamic_mesh_uploads",
    "save_attempts",
    "save_failures",
    "shadow_caster_meshes",
];

/// Outcome of one draw attempt. Retries and out-of-memory attempts stay visible
/// as their own rows; only `Presented` follows a successful present API call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawOutcome {
    Presented,
    #[default]
    Retry,
    OutOfMemory,
}

impl DrawOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Presented => "presented",
            Self::Retry => "retry",
            Self::OutOfMemory => "out_of_memory",
        }
    }
}

/// One draw attempt. Identity fields are exact integers; every optional field is
/// missing when the measurement was unavailable, unsupported or invalid.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameRow {
    /// Unique, monotonically increasing per instrumented draw attempt.
    pub draw_attempt_id: u64,
    /// Increments on every renderer creation/recreation; disambiguates counter resets.
    pub renderer_epoch: u64,
    /// Successful present API calls so far. Not a compositor scanout count.
    pub presented_count: u64,
    pub result: DrawOutcome,
    /// GPU submission made by this attempt, if one was submitted.
    pub submitted_gpu_frame_id: Option<u64>,
    /// Previously completed GPU submission observed during this attempt.
    pub completed_gpu_frame_id: Option<u64>,
    /// Renderer epoch owning `completed_gpu_frame_id`.
    pub completed_gpu_renderer_epoch: Option<u64>,
    pub draw_interval_wall_ms: Option<f64>,
    pub main_wall_ms: Option<f64>,
    /// Main-thread CPU busy time for this attempt; missing where unsupported.
    pub main_cpu_busy_ms: Option<f64>,
    /// Elapsed time in the streaming request/poll section: elapsed, not work.
    pub stream_request_elapsed_ms: Option<f64>,
    pub physics_wall_ms: Option<f64>,
    pub mesh_sync_wall_ms: Option<f64>,
    /// Blocking fence waits inside this frame's upload/retain calls, summed.
    /// This is where the frame normally blocks on the previous submission.
    pub mesh_sync_fence_wait_wall_ms: Option<f64>,
    pub dynamic_mesh_build_wall_ms: Option<f64>,
    pub dynamic_upload_wall_ms: Option<f64>,
    pub render_wall_ms: Option<f64>,
    pub save_wall_ms: Option<f64>,
    /// Fence wait inside the draw itself, normally already signalled by the
    /// mesh-sync waits above. Not the frame's total fence wait.
    pub render_fence_wait_wall_ms: Option<f64>,
    pub acquire_wall_ms: Option<f64>,
    pub present_wall_ms: Option<f64>,
    pub gpu_prev_render_ms: Option<f64>,
    pub gpu_prev_shadow_ms: Option<f64>,
    pub gpu_prev_shadows: Option<bool>,
    pub gpu_prev_shadow_map_size: Option<u32>,
    /// Number of upload/retain fence waits summed in
    /// `mesh_sync_fence_wait_wall_ms`; 0 when the frame made none.
    pub mesh_sync_fence_waits: Option<u32>,
    pub physics_fixed_steps: Option<u32>,
    pub voxel_bodies_total: Option<u32>,
    pub voxel_bodies_active: Option<u32>,
    pub voxel_bodies_sleeping: Option<u32>,
    pub voxel_bodies_not_simulated: Option<u32>,
    pub chunk_mesh_uploads: Option<u32>,
    pub dynamic_mesh_builds: Option<u32>,
    pub dynamic_mesh_uploads: Option<u32>,
    /// Saves started since the previous row, successful or not.
    pub save_attempts: Option<u32>,
    /// Subset of `save_attempts` that failed.
    pub save_failures: Option<u32>,
    pub shadow_caster_meshes: Option<u32>,
}

/// Shadow-pass work belongs to a row only when that attempt actually submitted;
/// the renderer keeps reporting the previous pass's count on a retry.
pub fn shadow_casters_for_attempt(
    submitted_gpu_frame_id: Option<u64>,
    casters: Option<u32>,
) -> Option<u32> {
    submitted_gpu_frame_id.and(casters)
}

/// Main-thread CPU busy time for one span. Reads the clock only while enabled,
/// so an uninstrumented frame adds no readings.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuBusySpan(Option<Duration>);

impl CpuBusySpan {
    pub fn begin(enabled: bool) -> Self {
        Self(enabled.then(thread_cpu_time).flatten())
    }
    /// Milliseconds of busy time since `begin`; `None` when disabled or when
    /// either reading was unavailable or invalid.
    pub fn finish(&self) -> Option<f64> {
        cpu_busy_ms(self.0, self.0.and_then(|_| thread_cpu_time()))
    }
}

fn write_id(writer: &mut impl Write, value: Option<u64>) -> io::Result<()> {
    match value {
        Some(value) => write!(writer, "{value}"),
        None => Ok(()),
    }
}

fn write_count(writer: &mut impl Write, value: Option<u32>) -> io::Result<()> {
    match value {
        Some(value) => write!(writer, "{value}"),
        None => Ok(()),
    }
}

/// Negative, infinite or NaN durations are failed measurements, not zeroes.
fn write_ms(writer: &mut impl Write, value: Option<f64>) -> io::Result<()> {
    match value.filter(|value| value.is_finite() && *value >= 0.) {
        Some(value) => write!(writer, "{value:.4}"),
        None => Ok(()),
    }
}

impl FrameRow {
    fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        write!(
            writer,
            "{},{},{},{},",
            self.draw_attempt_id,
            self.renderer_epoch,
            self.presented_count,
            self.result.label()
        )?;
        write_id(writer, self.submitted_gpu_frame_id)?;
        write!(writer, ",")?;
        write_id(writer, self.completed_gpu_frame_id)?;
        write!(writer, ",")?;
        write_id(writer, self.completed_gpu_renderer_epoch)?;
        for value in [
            self.draw_interval_wall_ms,
            self.main_wall_ms,
            self.main_cpu_busy_ms,
            self.stream_request_elapsed_ms,
            self.physics_wall_ms,
            self.mesh_sync_wall_ms,
            self.mesh_sync_fence_wait_wall_ms,
            self.dynamic_mesh_build_wall_ms,
            self.dynamic_upload_wall_ms,
            self.render_wall_ms,
            self.save_wall_ms,
            self.render_fence_wait_wall_ms,
            self.acquire_wall_ms,
            self.present_wall_ms,
            self.gpu_prev_render_ms,
            self.gpu_prev_shadow_ms,
        ] {
            write!(writer, ",")?;
            write_ms(writer, value)?;
        }
        write!(writer, ",")?;
        write_count(writer, self.gpu_prev_shadows.map(u32::from))?;
        for value in [
            self.gpu_prev_shadow_map_size,
            self.mesh_sync_fence_waits,
            self.physics_fixed_steps,
            self.voxel_bodies_total,
            self.voxel_bodies_active,
            self.voxel_bodies_sleeping,
            self.voxel_bodies_not_simulated,
            self.chunk_mesh_uploads,
            self.dynamic_mesh_builds,
            self.dynamic_mesh_uploads,
            self.save_attempts,
            self.save_failures,
            self.shadow_caster_meshes,
        ] {
            write!(writer, ",")?;
            write_count(writer, value)?;
        }
        writeln!(writer)
    }
}

/// Deduplicates repeated reads of one completed GPU submission. The renderer
/// epoch is part of the identity: a recreated renderer restarts its submission
/// counter, and identical ids from different epochs are different submissions.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuCompletionTracker {
    last: Option<(u64, u64)>,
}

impl GpuCompletionTracker {
    /// True when this completion has not been reported yet.
    pub fn accept(&mut self, renderer_epoch: u64, frame_id: u64) -> bool {
        let identity = (renderer_epoch, frame_id);
        if self.last == Some(identity) {
            return false;
        }
        self.last = Some(identity);
        true
    }
}

/// Main-thread CPU busy time, independent of wall time. `None` where the clock
/// is unsupported or the call fails; an unsupported clock never reads as zero.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn thread_cpu_time() -> Option<Duration> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `value` is a live, writable, correctly aligned timespec owned by
    // this frame; clock_gettime writes only through that pointer and returns a
    // checked status. No other Rust state is aliased.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) };
    if status != 0 {
        return None;
    }
    let seconds = u64::try_from(value.tv_sec).ok()?;
    let nanoseconds = u32::try_from(value.tv_nsec)
        .ok()
        .filter(|n| *n < 1_000_000_000)?;
    Some(Duration::new(seconds, nanoseconds))
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn thread_cpu_time() -> Option<Duration> {
    None
}

/// Milliseconds of CPU busy time between two readings of the same clock.
/// Missing or backwards readings stay missing instead of becoming zero.
pub fn cpu_busy_ms(start: Option<Duration>, end: Option<Duration>) -> Option<f64> {
    let elapsed = end?.checked_sub(start?)?;
    Some(elapsed.as_secs_f64() * 1000.)
}

pub struct FrameLog {
    writer: BufWriter<File>,
    remaining: u32,
    pub path: PathBuf,
}

impl FrameLog {
    /// Place an integer frame count in `profile-frames.txt` beside the world save.
    /// Consume the request only after a fresh output file is successfully opened.
    /// Normal gameplay does not create a capture or write per-frame data.
    pub fn requested(directory: &Path) -> io::Result<Option<Self>> {
        let request = directory.join("profile-frames.txt");
        let file = match File::open(&request) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut text = String::new();
        file.take(33).read_to_string(&mut text)?;
        if text.len() > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "profile request exceeds 32 bytes",
            ));
        }
        let count = text
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=MAX_FRAMES).contains(n))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "profile count must be 1..240000",
                )
            })?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = directory.join(format!("frame-profile-v2-{stamp}.csv"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        writeln!(
            writer,
            "#matterweave-frame-capture schema_version={SCHEMA_VERSION}"
        )?;
        writeln!(writer, "{}", COLUMNS.join(","))?;
        writer.flush()?;
        fs::remove_file(request)?;
        Ok(Some(Self {
            writer,
            remaining: count,
            path,
        }))
    }

    /// Records one draw attempt. GPU completion fields describe a prior
    /// submission, not this attempt's work. Returns false once the bounded
    /// capture is complete.
    pub fn record(&mut self, row: &FrameRow) -> io::Result<bool> {
        if self.remaining == 0 {
            return Ok(false);
        }
        row.write(&mut self.writer)?;
        self.remaining -= 1;
        if self.remaining == 0 {
            self.writer.flush()?;
        }
        Ok(self.remaining != 0)
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(draw_attempt_id: u64) -> FrameRow {
        FrameRow {
            draw_attempt_id,
            ..FrameRow::default()
        }
    }

    #[test]
    fn header_declares_schema_version_two_and_every_column() {
        let dir = directory("header");
        fs::write(dir.join("profile-frames.txt"), "1").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        log.record(&row(1)).unwrap();
        let text = fs::read_to_string(&log.path).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap(),
            "#matterweave-frame-capture schema_version=2"
        );
        assert_eq!(lines.next().unwrap(), COLUMNS.join(","));
        // Every row must carry exactly one cell per declared column.
        assert_eq!(
            lines.next().unwrap().split(',').count(),
            COLUMNS.len(),
            "row width must match the header"
        );
        assert!(log
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("frame-profile-v2-"));
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn identifiers_are_exact_beyond_float_precision() {
        let dir = directory("ids");
        fs::write(dir.join("profile-frames.txt"), "1").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        // 2^53 + 1 and 2^53 + 3 are indistinguishable as f64; a typed row must keep them.
        log.record(&FrameRow {
            draw_attempt_id: 9_007_199_254_740_993,
            renderer_epoch: 9_007_199_254_740_995,
            presented_count: 9_007_199_254_740_997,
            submitted_gpu_frame_id: Some(18_446_744_073_709_551_615),
            completed_gpu_frame_id: Some(9_007_199_254_740_999),
            completed_gpu_renderer_epoch: Some(9_007_199_254_740_995),
            ..FrameRow::default()
        })
        .unwrap();
        let text = fs::read_to_string(&log.path).unwrap();
        let cells: Vec<&str> = text.lines().nth(2).unwrap().split(',').collect();
        let cell = |name: &str| cells[COLUMNS.iter().position(|c| *c == name).unwrap()];
        assert_eq!(cell("draw_attempt_id"), "9007199254740993");
        assert_eq!(cell("renderer_epoch"), "9007199254740995");
        assert_eq!(cell("presented_count"), "9007199254740997");
        assert_eq!(cell("submitted_gpu_frame_id"), "18446744073709551615");
        assert_eq!(cell("completed_gpu_frame_id"), "9007199254740999");
        assert_eq!(cell("completed_gpu_renderer_epoch"), "9007199254740995");
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_or_invalid_measurements_stay_empty_cells() {
        let dir = directory("missing");
        fs::write(dir.join("profile-frames.txt"), "1").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        log.record(&FrameRow {
            draw_attempt_id: 4,
            main_wall_ms: Some(0.),
            main_cpu_busy_ms: None,
            physics_wall_ms: Some(f64::NAN),
            render_wall_ms: Some(-1.),
            save_wall_ms: Some(f64::INFINITY),
            present_wall_ms: Some(0.5),
            ..FrameRow::default()
        })
        .unwrap();
        let text = fs::read_to_string(&log.path).unwrap();
        let cells: Vec<&str> = text.lines().nth(2).unwrap().split(',').collect();
        let cell = |name: &str| cells[COLUMNS.iter().position(|c| *c == name).unwrap()];
        assert_eq!(cell("main_wall_ms"), "0.0000");
        assert_eq!(cell("main_cpu_busy_ms"), "");
        assert_eq!(cell("physics_wall_ms"), "");
        assert_eq!(cell("render_wall_ms"), "");
        assert_eq!(cell("save_wall_ms"), "");
        assert_eq!(cell("present_wall_ms"), "0.5000");
        assert_eq!(cell("submitted_gpu_frame_id"), "");
        assert_eq!(cell("physics_fixed_steps"), "");
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retry_attempts_stay_visible_before_and_after_a_submission() {
        let dir = directory("retry");
        fs::write(dir.join("profile-frames.txt"), "3").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        log.record(&FrameRow {
            draw_attempt_id: 1,
            presented_count: 1,
            result: DrawOutcome::Presented,
            submitted_gpu_frame_id: Some(1),
            ..FrameRow::default()
        })
        .unwrap();
        // Retry before any submission: an acquire failure never invents an id.
        log.record(&FrameRow {
            draw_attempt_id: 2,
            presented_count: 1,
            result: DrawOutcome::Retry,
            ..FrameRow::default()
        })
        .unwrap();
        // Retry after a real submission: an out-of-date present keeps the
        // submission identity while the presentation count stands still.
        log.record(&FrameRow {
            draw_attempt_id: 3,
            presented_count: 1,
            result: DrawOutcome::Retry,
            submitted_gpu_frame_id: Some(2),
            ..FrameRow::default()
        })
        .unwrap();
        let text = fs::read_to_string(&log.path).unwrap();
        let rows: Vec<Vec<&str>> = text
            .lines()
            .skip(2)
            .map(|line| line.split(',').collect())
            .collect();
        let index = |name: &str| COLUMNS.iter().position(|c| *c == name).unwrap();
        assert_eq!(rows.len(), 3, "a retry attempt is its own row");
        assert_eq!(rows[0][index("result")], "presented");
        assert_eq!(rows[1][index("result")], "retry");
        assert_eq!(rows[2][index("result")], "retry");
        assert_eq!(rows[1][index("draw_attempt_id")], "2");
        assert_eq!(rows[1][index("presented_count")], "1");
        assert_eq!(rows[1][index("submitted_gpu_frame_id")], "");
        assert_eq!(rows[2][index("presented_count")], "1");
        assert_eq!(rows[2][index("submitted_gpu_frame_id")], "2");
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Every column, filled with a value distinguishable from all its
    /// neighbours, checked as a name-to-cell map. Catches order drift between
    /// `COLUMNS` and `FrameRow::write` that a width check cannot see.
    #[test]
    fn every_column_carries_its_own_value_in_the_declared_order() {
        let dir = directory("contract");
        fs::write(dir.join("profile-frames.txt"), "1").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        log.record(&FrameRow {
            draw_attempt_id: 101,
            renderer_epoch: 102,
            presented_count: 103,
            result: DrawOutcome::OutOfMemory,
            submitted_gpu_frame_id: Some(104),
            completed_gpu_frame_id: Some(105),
            completed_gpu_renderer_epoch: Some(106),
            draw_interval_wall_ms: Some(1.),
            main_wall_ms: Some(2.),
            main_cpu_busy_ms: Some(3.),
            stream_request_elapsed_ms: Some(4.),
            physics_wall_ms: Some(5.),
            mesh_sync_wall_ms: Some(6.),
            mesh_sync_fence_wait_wall_ms: Some(7.),
            dynamic_mesh_build_wall_ms: Some(8.),
            dynamic_upload_wall_ms: Some(9.),
            render_wall_ms: Some(10.),
            save_wall_ms: Some(11.),
            render_fence_wait_wall_ms: Some(12.),
            acquire_wall_ms: Some(13.),
            present_wall_ms: Some(14.),
            gpu_prev_render_ms: Some(15.),
            gpu_prev_shadow_ms: Some(16.),
            gpu_prev_shadows: Some(true),
            gpu_prev_shadow_map_size: Some(2048),
            mesh_sync_fence_waits: Some(21),
            physics_fixed_steps: Some(22),
            voxel_bodies_total: Some(23),
            voxel_bodies_active: Some(24),
            voxel_bodies_sleeping: Some(25),
            voxel_bodies_not_simulated: Some(26),
            chunk_mesh_uploads: Some(27),
            dynamic_mesh_builds: Some(28),
            dynamic_mesh_uploads: Some(29),
            save_attempts: Some(30),
            save_failures: Some(31),
            shadow_caster_meshes: Some(32),
        })
        .unwrap();
        let text = fs::read_to_string(&log.path).unwrap();
        let cells: Vec<&str> = text.lines().nth(2).unwrap().split(',').collect();
        assert_eq!(cells.len(), COLUMNS.len());
        let written: Vec<(&str, &str)> = COLUMNS.iter().copied().zip(cells).collect();
        assert_eq!(
            written,
            vec![
                ("draw_attempt_id", "101"),
                ("renderer_epoch", "102"),
                ("presented_count", "103"),
                ("result", "out_of_memory"),
                ("submitted_gpu_frame_id", "104"),
                ("completed_gpu_frame_id", "105"),
                ("completed_gpu_renderer_epoch", "106"),
                ("draw_interval_wall_ms", "1.0000"),
                ("main_wall_ms", "2.0000"),
                ("main_cpu_busy_ms", "3.0000"),
                ("stream_request_elapsed_ms", "4.0000"),
                ("physics_wall_ms", "5.0000"),
                ("mesh_sync_wall_ms", "6.0000"),
                ("mesh_sync_fence_wait_wall_ms", "7.0000"),
                ("dynamic_mesh_build_wall_ms", "8.0000"),
                ("dynamic_upload_wall_ms", "9.0000"),
                ("render_wall_ms", "10.0000"),
                ("save_wall_ms", "11.0000"),
                ("render_fence_wait_wall_ms", "12.0000"),
                ("acquire_wall_ms", "13.0000"),
                ("present_wall_ms", "14.0000"),
                ("gpu_prev_render_ms", "15.0000"),
                ("gpu_prev_shadow_ms", "16.0000"),
                ("gpu_prev_shadows", "1"),
                ("gpu_prev_shadow_map_size", "2048"),
                ("mesh_sync_fence_waits", "21"),
                ("physics_fixed_steps", "22"),
                ("voxel_bodies_total", "23"),
                ("voxel_bodies_active", "24"),
                ("voxel_bodies_sleeping", "25"),
                ("voxel_bodies_not_simulated", "26"),
                ("chunk_mesh_uploads", "27"),
                ("dynamic_mesh_builds", "28"),
                ("dynamic_mesh_uploads", "29"),
                ("save_attempts", "30"),
                ("save_failures", "31"),
                ("shadow_caster_meshes", "32"),
            ]
        );
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shadow_flag_distinguishes_unknown_from_disabled_and_enabled() {
        let dir = directory("shadow-flag");
        fs::write(dir.join("profile-frames.txt"), "3").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        for shadows in [None, Some(false), Some(true)] {
            log.record(&FrameRow {
                gpu_prev_shadows: shadows,
                ..FrameRow::default()
            })
            .unwrap();
        }
        let text = fs::read_to_string(&log.path).unwrap();
        let index = COLUMNS
            .iter()
            .position(|c| *c == "gpu_prev_shadows")
            .unwrap();
        let cells: Vec<&str> = text
            .lines()
            .skip(2)
            .map(|line| line.split(',').nth(index).unwrap())
            .collect();
        assert_eq!(cells, vec!["", "0", "1"]);
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shadow_work_belongs_only_to_attempts_that_submitted() {
        // A retry that recorded no command buffer must not inherit the previous
        // pass's caster count, even though the renderer still reports it.
        assert_eq!(shadow_casters_for_attempt(None, Some(37)), None);
        assert_eq!(shadow_casters_for_attempt(Some(9), Some(37)), Some(37));
        assert_eq!(shadow_casters_for_attempt(Some(9), None), None);
        // A submitted attempt whose presentation retried still did shadow work.
        assert_eq!(shadow_casters_for_attempt(Some(9), Some(0)), Some(0));
    }

    #[test]
    fn cpu_busy_span_reads_the_clock_only_when_enabled() {
        assert_eq!(CpuBusySpan::begin(false).finish(), None);
        let enabled = CpuBusySpan::begin(true).finish();
        assert_eq!(
            enabled.is_some(),
            cfg!(any(target_os = "linux", target_os = "android")),
            "a span reports a value exactly where the clock is supported"
        );
    }

    #[test]
    fn completed_gpu_frames_deduplicate_within_but_not_across_renderer_epochs() {
        let mut tracker = GpuCompletionTracker::default();
        assert!(tracker.accept(1, 1));
        assert!(!tracker.accept(1, 1), "same submission must report once");
        assert!(tracker.accept(1, 2));
        // Renderer recreation resets the GPU submission counter; identical ids from
        // a new epoch are different submissions, not a repeat of the old one.
        assert!(tracker.accept(2, 1));
        assert!(!tracker.accept(2, 1));
        assert!(tracker.accept(1, 1), "epoch is part of the identity");
    }

    #[test]
    fn cpu_busy_needs_two_valid_readings_from_the_same_clock() {
        let start = Duration::from_millis(10);
        let end = Duration::from_micros(11_500);
        assert_eq!(cpu_busy_ms(Some(start), Some(end)), Some(1.5));
        assert_eq!(cpu_busy_ms(None, Some(end)), None);
        assert_eq!(cpu_busy_ms(Some(start), None), None);
        assert_eq!(cpu_busy_ms(None, None), None);
        // A backwards reading is a failed measurement, not zero busy time.
        assert_eq!(cpu_busy_ms(Some(end), Some(start)), None);
    }

    #[test]
    fn thread_cpu_time_is_available_only_where_the_clock_is_supported() {
        let reading = thread_cpu_time();
        if cfg!(any(target_os = "linux", target_os = "android")) {
            let first = reading.expect("CLOCK_THREAD_CPUTIME_ID is supported here");
            let mut sum = 0_u64;
            for i in 0..2_000_000_u64 {
                sum = sum.wrapping_add(i * 3);
            }
            assert_ne!(sum, u64::MAX);
            let second = thread_cpu_time().expect("second reading");
            assert!(second > first, "busy work must increase thread CPU time");
            assert!(
                cpu_busy_ms(Some(first), Some(second)).is_some_and(|ms| ms > 0.),
                "delta must be positive"
            );
        } else {
            assert!(
                reading.is_none(),
                "unsupported platforms report missing, not zero"
            );
        }
    }

    fn directory(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("matterweave-metrics-{}-{name}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn capture_is_opt_in_bounded_and_preserves_earlier_captures() {
        let dir = directory("capture");
        // Normal operation writes nothing and creates no capture.
        assert!(FrameLog::requested(&dir).unwrap().is_none());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let earlier = dir.join("frame-profile-1.csv");
        fs::write(&earlier, "presented_count,draw_interval_wall_ms\n1,16\n").unwrap();
        fs::write(dir.join("profile-frames.txt"), "2").unwrap();
        let mut log = FrameLog::requested(&dir).unwrap().unwrap();
        assert!(!dir.join("profile-frames.txt").exists());
        assert!(log.record(&row(1)).unwrap());
        assert!(!log.record(&row(2)).unwrap());
        assert!(!log.record(&row(3)).unwrap(), "capture stays bounded");
        let text = fs::read_to_string(&log.path).unwrap();
        assert_eq!(text.lines().count(), 4, "two header lines and two rows");
        assert_ne!(log.path, earlier);
        assert_eq!(
            fs::read_to_string(&earlier).unwrap(),
            "presented_count,draw_interval_wall_ms\n1,16\n",
            "an older schema capture must survive untouched"
        );
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_or_excessive_requests_are_retained_without_opening_a_capture() {
        let dir = directory("invalid");
        for request in [
            "0",
            "240001",
            "not-a-number",
            "999999999999999999999999999999999999999",
        ] {
            fs::write(dir.join("profile-frames.txt"), request).unwrap();
            assert!(FrameLog::requested(&dir).is_err());
            assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
            assert_eq!(
                fs::read_to_string(dir.join("profile-frames.txt")).unwrap(),
                request
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
