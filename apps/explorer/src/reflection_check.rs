//! Native Vulkan R09 reflection gate: quality phases on device, plus an
//! off/on frame-cost measurement. Writes a local report; Android screenshots are
//! captured externally with adb. Not a minimum-FPS or efficiency claim.
use glam::{Mat4, Vec3};
use matterweave_core::{Mesh, World};
use matterweave_render::reflection::{
    MaterialTable, ReflectionVolume, DEFAULT_TRACE_STEPS, SURFACE_OFFSET,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use std::{
    future::Future,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

const MIRROR: u8 = 7;
const STONE: u8 = 2;
const TARGET: u8 = 1;
const SOIL: u8 = 3;
const ORIGIN: [i32; 3] = [-4, -4, -4];
const DIMENSIONS: [u32; 3] = [24, 20, 24];
/// Warmup frames before any measurement, per DoD item 10.
const WARMUP: u32 = 120;
const MEASURED: u32 = 1000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Scripted responses for screenshots and log reading.
    Quality,
    /// Off/on frame-time measurement at one resolution and scene.
    Cost,
}

fn fill(world: &mut World, lo: [i32; 3], hi: [i32; 3], material: u8) {
    for x in lo[0]..hi[0] {
        for y in lo[1]..hi[1] {
            for z in lo[2]..hi[2] {
                world.set([x, y, z], material);
            }
        }
    }
}

/// Mirror wall with a reflected object placed outside the camera frustum.
fn scene() -> World {
    let mut world = World::new(20260907);
    fill(&mut world, [0, 0, -4], [1, 8, 8], MIRROR);
    fill(&mut world, [1, -1, -4], [12, 0, 8], STONE);
    fill(&mut world, [1, 0, -4], [12, 6, -3], STONE);
    fill(&mut world, [1, 0, 8], [12, 6, 9], STONE);
    fill(&mut world, [6, 2, 5], [8, 6, 8], TARGET);
    world
}

fn table() -> MaterialTable {
    let mut color = [[0.5; 3]; 256];
    color[0] = [0.; 3];
    color[MIRROR as usize] = [0.30, 0.34, 0.38];
    color[STONE as usize] = [0.62, 0.63, 0.60];
    color[TARGET as usize] = [0.90, 0.10, 0.05];
    color[SOIL as usize] = [0.35, 0.24, 0.17];
    let mut table = MaterialTable::new(color).unwrap();
    table.set_mirror(MIRROR, 1.0).unwrap();
    table
}

fn recolor(mesh: &mut Mesh, world: &World, table: &MaterialTable) {
    for vertex in &mut mesh.vertices {
        let cell = [
            (vertex.position[0] - vertex.normal[0] * SURFACE_OFFSET).floor() as i32,
            (vertex.position[1] - vertex.normal[1] * SURFACE_OFFSET).floor() as i32,
            (vertex.position[2] - vertex.normal[2] * SURFACE_OFFSET).floor() as i32,
        ];
        vertex.color = table.color(world.get(cell));
    }
}

fn eye(phase: usize) -> [f32; 3] {
    if phase == 2 {
        [11.0, 5.0, 2.0]
    } else {
        [11.0, 3.0, 0.0]
    }
}

fn camera(phase: usize) -> [[f32; 4]; 4] {
    let view = Mat4::look_at_rh(
        Vec3::from_array(eye(phase)),
        Vec3::new(-1.0, 3.0, 0.0),
        Vec3::Y,
    );
    let projection = Mat4::perspective_rh(35f32.to_radians(), 4. / 3., 0.1, 100.);
    (projection * view).to_cols_array_2d()
}

#[derive(Clone, Copy)]
enum Action {
    None,
    Publish,
    Occluder,
    RemoveObject,
    ReplaceScene,
    Disable,
}

#[derive(Clone, Copy)]
struct Phase {
    label: &'static str,
    warmup: u32,
    measured: u32,
    action: Action,
}

fn phases(mode: Mode) -> Vec<Phase> {
    match mode {
        Mode::Quality => vec![
            Phase {
                label: "off",
                warmup: 10,
                measured: 30,
                action: Action::None,
            },
            Phase {
                label: "on",
                warmup: 10,
                measured: 30,
                action: Action::Publish,
            },
            Phase {
                label: "camera-moved",
                warmup: 10,
                measured: 30,
                action: Action::None,
            },
            Phase {
                label: "occluder-added",
                warmup: 10,
                measured: 30,
                action: Action::Occluder,
            },
            Phase {
                label: "object-removed",
                warmup: 10,
                measured: 30,
                action: Action::RemoveObject,
            },
            Phase {
                label: "sun-moved",
                warmup: 10,
                measured: 30,
                action: Action::None,
            },
            Phase {
                label: "scene-replaced",
                warmup: 10,
                measured: 30,
                action: Action::ReplaceScene,
            },
            Phase {
                label: "disabled",
                warmup: 10,
                measured: 30,
                action: Action::Disable,
            },
        ],
        Mode::Cost => vec![
            Phase {
                label: "off",
                warmup: WARMUP,
                measured: MEASURED,
                action: Action::None,
            },
            Phase {
                label: "on",
                warmup: WARMUP,
                measured: MEASURED,
                action: Action::Publish,
            },
        ],
    }
}

struct Measurement {
    label: String,
    frames: u32,
    /// Wall time between consecutive presented frames.
    intervals: Vec<f64>,
    /// Wall time inside the `render_with_lighting` call, including its fence wait.
    calls: Vec<f64>,
    gpu: Option<f64>,
    material_bytes: usize,
    palette_bytes: usize,
    publications: usize,
    upload_ms: f64,
    fence_wait_ms: f64,
}

fn percentile(mut values: Vec<f64>, fraction: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((values.len() as f64 - 1.) * fraction).round() as usize;
    values[index.min(values.len() - 1)]
}

pub struct ReflectionCheck {
    mode: Mode,
    report_path: PathBuf,
    report: Vec<String>,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    world: World,
    epoch: u64,
    lighting: LightingSettings,
    phase: usize,
    frame_in_phase: u32,
    applied: Option<usize>,
    measurement: Option<Measurement>,
    last_present: Option<Instant>,
    resume_count: u32,
}

impl ReflectionCheck {
    pub fn new(report_path: PathBuf, mode: Mode) -> Self {
        Self {
            mode,
            report_path,
            report: Vec::new(),
            renderer: None,
            window: None,
            world: scene(),
            epoch: 0,
            lighting: LightingSettings {
                shadows: true,
                sun: Sun {
                    direction_to_sun: [0.4, 0.85, 0.3],
                    intensity: 0.8,
                },
                ..Default::default()
            },
            phase: 0,
            frame_in_phase: 0,
            applied: None,
            measurement: None,
            last_present: None,
            resume_count: 0,
        }
    }
    fn record(&mut self, entry: String) {
        log::info!("{entry}");
        self.report.push(entry);
        std::fs::write(&self.report_path, self.report.join("\n") + "\n")
            .expect("write reflection check report");
    }
    /// Upload current geometry, then publish a fresh volume for it.
    fn publish(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let t = table();
        let mut mesh = self.world.mesh();
        recolor(&mut mesh, &self.world, &t);
        renderer.upload(&mesh).unwrap();
        let volume = ReflectionVolume::pack(
            &self.world,
            self.epoch,
            ORIGIN,
            DIMENSIONS,
            &t,
            DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        let stats = renderer
            .upload_reflection(&volume, &self.world, self.epoch)
            .unwrap();
        if let Some(m) = self.measurement.as_mut() {
            m.publications += 1;
            m.upload_ms += stats.upload_ms;
            m.fence_wait_ms += stats.fence_wait_ms;
            let state = renderer.reflection_state();
            m.material_bytes = state.material_bytes;
            m.palette_bytes = state.palette_bytes;
        }
        let published = renderer.reflection_state().published_submission;
        self.record(format!(
            "published: {} bytes, fence wait {:.3}ms, upload {:.3}ms, submitted at {:?}",
            stats.bytes, stats.fence_wait_ms, stats.upload_ms, published
        ));
    }
    fn apply_phase(&mut self) {
        let phase = phases(self.mode);
        let phase = phase[self.phase];
        match phase.action {
            Action::None => {}
            Action::Publish => self.publish(),
            Action::Occluder => {
                fill(&mut self.world, [4, 1, 4], [6, 5, 8], STONE);
                self.epoch += 1;
                self.publish();
            }
            Action::RemoveObject => {
                fill(&mut self.world, [6, 2, 5], [8, 6, 8], 0);
                self.epoch += 1;
                self.publish();
            }
            Action::ReplaceScene => {
                let mut replacement = World::new(20260907);
                fill(&mut replacement, [0, 0, -4], [1, 8, 8], MIRROR);
                fill(&mut replacement, [1, -1, -4], [12, 0, 8], STONE);
                fill(&mut replacement, [3, 0, 1], [5, 4, 3], SOIL);
                self.world = replacement;
                self.epoch += 1;
                self.publish();
            }
            Action::Disable => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.disable_reflection();
                }
            }
        }
        self.record(format!(
            "phase {}: {} (reflection {})",
            self.phase,
            phase.label,
            self.renderer
                .as_ref()
                .map(|r| r.reflection_enabled().to_string())
                .unwrap_or_else(|| "no-renderer".into())
        ));
    }
    fn finish_phase(&mut self) {
        if let Some(m) = self.measurement.take() {
            if m.frames > 0 {
                self.record(format!(
                    "measurement {}: frames {} interval p50 {:.3} p95 {:.3} p99 {:.3} ms | render call p50 {:.3} p95 {:.3} p99 {:.3} ms | gpu {:?} | materials {} palette {} bytes | publications {} | upload {:.3}ms fence {:.3}ms",
                    m.label,
                    m.frames,
                    percentile(m.intervals.clone(), 0.50),
                    percentile(m.intervals.clone(), 0.95),
                    percentile(m.intervals.clone(), 0.99),
                    percentile(m.calls.clone(), 0.50),
                    percentile(m.calls.clone(), 0.95),
                    percentile(m.calls.clone(), 0.99),
                    m.gpu,
                    m.material_bytes,
                    m.palette_bytes,
                    m.publications,
                    m.upload_ms,
                    m.fence_wait_ms
                ));
            }
        }
    }
}

impl ApplicationHandler for ReflectionCheck {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title("Matterweave reflection gate")
                    .with_inner_size(winit::dpi::PhysicalSize::new(960, 720)),
            )
            .unwrap(),
        );
        let mut init = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(result) = init.as_mut().poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("renderer initialization suspended")
        };
        let mut renderer = result.unwrap();
        self.resume_count += 1;
        self.record(format!("capabilities: {}", renderer.capabilities));
        self.record(format!(
            "resume {}: gpu timestamps {}",
            self.resume_count,
            renderer.gpu_timestamps_supported()
        ));
        let t = table();
        let mut mesh = self.world.mesh();
        recolor(&mut mesh, &self.world, &t);
        renderer.upload(&mesh).unwrap();
        self.renderer = Some(renderer);
        self.window = Some(window);
        // A resumed renderer owns no reflection data: republish the current scene.
        if self.phase > 0 {
            self.publish();
        }
        self.applied = None;
        self.frame_in_phase = 0;
        self.last_present = None;
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.record("suspended: renderer released".into());
        self.renderer = None;
        self.window = None;
        self.applied = None;
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let WindowEvent::Resized(size) = event {
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(size.width, size.height);
            }
            return;
        }
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        let total = phases(self.mode).len();
        if self.phase >= total {
            self.renderer = None;
            self.window = None;
            self.record("reflection_check: complete".into());
            el.exit();
            return;
        }
        let phase = phases(self.mode)[self.phase];
        if self.applied != Some(self.phase) {
            // The measurement exists before the phase action so a publication
            // performed on entry is attributed to this phase.
            self.measurement = Some(Measurement {
                label: phase.label.to_string(),
                frames: 0,
                intervals: Vec::new(),
                calls: Vec::new(),
                gpu: None,
                material_bytes: 0,
                palette_bytes: 0,
                publications: 0,
                upload_ms: 0.,
                fence_wait_ms: 0.,
            });
            self.apply_phase();
            self.applied = Some(self.phase);
        }
        if self.phase == 5 {
            self.lighting.sun = Sun {
                direction_to_sun: [-0.6, 0.35, 0.7],
                intensity: 0.8,
            };
        }
        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or_default();
        let begin = Instant::now();
        let result = self.renderer.as_mut().unwrap().render_with_lighting(
            camera(self.phase),
            eye(self.phase),
            &Hud::new(size.width as f32, size.height as f32),
            &self.lighting,
        );
        let call_ms = begin.elapsed().as_secs_f64() * 1000.;
        match result {
            FrameResult::Presented => {}
            FrameResult::Retry => return,
            other => panic!("unexpected render result: {other:?}"),
        }
        let now = Instant::now();
        let interval = self
            .last_present
            .map(|last| now.duration_since(last).as_secs_f64() * 1000.)
            .unwrap_or(0.);
        self.last_present = Some(now);
        self.frame_in_phase += 1;
        let frame_in_phase = self.frame_in_phase;
        if let Some(m) = self.measurement.as_mut() {
            if frame_in_phase > phase.warmup {
                m.frames += 1;
                m.intervals.push(interval);
                m.calls.push(call_ms);
                if let Some(timing) = self.renderer.as_ref().unwrap().gpu_timings() {
                    m.gpu = Some(timing.render_ms);
                }
            }
            if m.frames >= phase.measured && phase.measured > 0 {
                self.finish_phase();
                self.phase += 1;
                self.frame_in_phase = 0;
                return;
            }
        }
        if frame_in_phase >= phase.warmup + phase.measured.max(1) {
            self.finish_phase();
            self.phase += 1;
            self.frame_in_phase = 0;
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Desktop entry used by `--reflection-check`.
pub fn run(report_path: PathBuf, mode: Mode) {
    let mut check = ReflectionCheck::new(report_path, mode);
    EventLoop::new()
        .expect("event loop")
        .run_app(&mut check)
        .expect("reflection check loop");
}
