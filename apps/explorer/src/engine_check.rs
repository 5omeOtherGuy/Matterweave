//! Native Vulkan R09 first-bounce gate, not a mobile performance benchmark.
//! Writes a local report; Android screenshots are captured externally with adb.
use glam::{Mat4, Vec3};
use matterweave_core::World;
use matterweave_render::{
    async_indirect::{AsyncIndirectConfig, AsyncIndirectLight},
    indirect::{IndirectVolume, UpdateBudget},
    FrameResult, Hud, LightingSettings, Renderer, Sun,
};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

fn roof(w: &mut World, closed: bool) {
    for x in -3..=3 {
        for z in -3..=3 {
            w.set([x, 3, z], if closed { 2 } else { 0 });
        }
    }
}
fn scene() -> World {
    let mut w = World::new(9);
    for x in -3i32..=3 {
        for z in -3i32..=3 {
            w.set([x, -1, z], 1);
            for y in 0..3 {
                if x.abs() == 3 || z.abs() == 3 {
                    w.set([x, y, z], 2);
                }
            }
        }
    }
    w
}
fn palette() -> [[f32; 3]; 256] {
    let mut p = [[0.5; 3]; 256];
    p[1] = [0.9, 0.05, 0.02];
    p[2] = [0.65; 3];
    p
}
fn upload_world(r: &mut Renderer, w: &World) {
    let mut mesh = w.mesh();
    // Fixture palette is explicit and identical for the renderer and producer.
    for v in &mut mesh.vertices {
        v.color = if v.color == [0.29, 0.48, 0.27] {
            palette()[1]
        } else {
            palette()[2]
        };
    }
    r.upload(&mesh).unwrap();
}
pub(crate) struct IndirectCheck {
    report_path: std::path::PathBuf,
    report: Vec<String>,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    world: World,
    cache: IndirectVolume,
    frame: u32,
    prepared: Option<u32>,
    phase_applied: Option<u32>,
    lighting: LightingSettings,
    worker: Option<AsyncIndirectLight>,
    pending_started: Option<std::time::Instant>,
    request_ms: f64,
    waiting_frames: u32,
    total_waiting_frames: u32,
}
impl IndirectCheck {
    pub fn new(report_path: std::path::PathBuf) -> Self {
        Self::with_background(report_path, false)
    }
    pub fn new_async(report_path: std::path::PathBuf) -> Self {
        Self::with_background(report_path, true)
    }
    fn with_background(report_path: std::path::PathBuf, background: bool) -> Self {
        let worker = background.then(|| {
            AsyncIndirectLight::new(
                AsyncIndirectConfig::new([-3, -1, -3], [7, 5, 7], 64, 32., palette()).unwrap(),
            )
        });
        if let Some(worker) = &worker {
            assert!(worker.available());
        }
        Self {
            worker,
            pending_started: None,
            request_ms: 0.0,
            waiting_frames: 0,
            total_waiting_frames: 0,
            report_path,
            report: Vec::new(),
            renderer: None,
            window: None,
            world: scene(),
            cache: IndirectVolume::new([-3, -1, -3], [7, 5, 7], 64, 32., palette()).unwrap(),
            frame: 0,
            prepared: None,
            phase_applied: None,
            lighting: LightingSettings {
                sun: Sun {
                    direction_to_sun: [0., 1., 0.],
                    intensity: 1.,
                },
                ..Default::default()
            },
        }
    }
}
impl IndirectCheck {
    fn record(&mut self, entry: String) {
        log::info!("{entry}");
        self.report.push(entry);
        std::fs::write(&self.report_path, self.report.join("\n") + "\n")
            .expect("write engine check report");
    }
}
#[cfg(target_os = "android")]
const PHASE_FRAMES: u32 = 120;
#[cfg(not(target_os = "android"))]
const PHASE_FRAMES: u32 = 6;
impl ApplicationHandler for IndirectCheck {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title("Matterweave indirect engine gate")
                    .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
            )
            .unwrap(),
        );
        let mut init = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(r) = init.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
            panic!("init suspended")
        };
        let mut r = r.unwrap();
        self.record(format!("capabilities: {}", r.capabilities));
        assert!(!r.indirect_enabled());
        upload_world(&mut r, &self.world);
        self.prepared = None;
        self.renderer = Some(r);
        self.window = Some(window);
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let WindowEvent::Resized(size) = event {
            self.prepared = None;
            if let Some(r) = &mut self.renderer {
                r.resize(size.width, size.height);
            }
            return;
        }
        if matches!(event, WindowEvent::CloseRequested) {
            el.exit();
            return;
        }
        if !matches!(event, WindowEvent::RedrawRequested) || self.renderer.is_none() {
            return;
        }
        let phase = self.frame / PHASE_FRAMES;
        let mut phase_report = None;
        let mut ready = true;
        let r = self.renderer.as_mut().unwrap();
        if self.prepared != Some(phase) {
            if self.phase_applied != Some(phase) {
                match phase {
                    2 => self.lighting.sun.direction_to_sun = [0.65, 1., 0.2],
                    3 => {
                        roof(&mut self.world, true);
                        upload_world(r, &self.world);
                        assert!(!r.indirect_enabled());
                        assert!(
                            r.upload_indirect(&self.cache, &self.world, 0, None).is_err(),
                            "old edit result rejected"
                        );
                    }
                    4 => {
                        roof(&mut self.world, false);
                        upload_world(r, &self.world);
                        self.lighting.sun.direction_to_sun = [0., 1., 0.];
                    }
                    _ => {}
                }
                self.phase_applied = Some(phase);
            }
            if phase == 6 {
                r.upload_indirect(&self.cache, &self.world, 0, None).unwrap();
                assert!(r.indirect_enabled());
                self.lighting.sun.intensity = 0.;
            } else if phase == 0 || phase == 5 {
                r.disable_indirect();
                phase_report = Some(format!("phase={phase} indirect=off"));
            } else if let Some(worker) = &mut self.worker {
                if !self.cache.valid_for(&self.world, 0, self.lighting.sun) {
                    r.disable_indirect();
                    if self.pending_started.is_none() {
                        let start = std::time::Instant::now();
                        assert!(worker.request(&self.world, 0, self.lighting.sun).unwrap());
                        self.request_ms = start.elapsed().as_secs_f64() * 1000.;
                        self.pending_started = Some(start);
                        self.waiting_frames = 0;
                    }
                    if let Some(result) = worker.poll(&self.world, 0, self.lighting.sun) {
                        self.cache = result.expect("background indirect preparation");
                    } else {
                        assert!(worker.available(), "lighting worker exited");
                        assert!(
                            self.pending_started.unwrap().elapsed().as_secs() < 30,
                            "background preparation did not complete"
                        );
                        ready = false;
                    }
                }
                if ready {
                    let latency_ms = self
                        .pending_started
                        .take()
                        .map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.);
                    let sample = self.cache.sample([-3, 1, 0], 0);
                    if phase == 3 {
                        assert_eq!(sample, [0.; 3]);
                    } else {
                        assert!(sample[0] > 0.02);
                    }
                    let start = std::time::Instant::now();
                    r.upload_indirect(&self.cache, &self.world, 0, None).unwrap();
                    phase_report = Some(format!(
                        "phase={phase} async=on request_ms={:.3} worker_to_poll_ms={latency_ms:.3} upload_ms={:.3} waiting_presentations={} bytes={} sample={sample:?}",
                        self.request_ms, start.elapsed().as_secs_f64() * 1000.,
                        self.waiting_frames, self.cache.resident_bytes()
                    ));
                    self.request_ms = 0.0;
                    self.waiting_frames = 0;
                }
            } else {
                let start = std::time::Instant::now();
                let mut rays = 0;
                let mut complete = false;
                for _ in 0..100 {
                    let s = self
                        .cache
                        .update(
                            &self.world,
                            0,
                            self.lighting.sun,
                            UpdateBudget {
                                rays: 4096,
                                work: 4096,
                            },
                        )
                        .unwrap();
                    assert!(s.rays <= 4096 && s.work <= 4096);
                    rays += s.rays;
                    if s.complete {
                        complete = true;
                        break;
                    }
                }
                assert!(complete);
                let sample = self.cache.sample([-3, 1, 0], 0);
                if phase == 3 {
                    assert_eq!(sample, [0.; 3]);
                } else {
                    assert!(sample[0] > 0.02);
                }
                r.upload_indirect(&self.cache, &self.world, 0, None).unwrap();
                phase_report = Some(format!(
                    "phase={phase} cpu_prepare_upload_ms={:.3} rays={rays} bytes={} sample={sample:?}",
                    start.elapsed().as_secs_f64() * 1000.,
                    self.cache.resident_bytes()
                ));
            }
            if ready {
                self.prepared = Some(phase);
            }
        }
        let size = self.window.as_ref().unwrap().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let eye = Vec3::new(1.8, 1.6, 1.8);
        let view = Mat4::perspective_rh(
            70f32.to_radians(),
            size.width as f32 / size.height as f32,
            0.05,
            100.,
        ) * Mat4::look_at_rh(eye, Vec3::new(-2., 0.9, -1.), Vec3::Y);
        match r.render_with_lighting(
            view.to_cols_array_2d(),
            eye.to_array(),
            &Hud::new(size.width as f32, size.height as f32),
            &self.lighting,
        ) {
            FrameResult::Presented => {
                assert_eq!(r.indirect_enabled(), ready && (1..=4).contains(&phase));
                if ready {
                    self.frame += 1;
                } else {
                    self.waiting_frames += 1;
                    self.total_waiting_frames += 1;
                }
            }
            FrameResult::Retry => {
                self.prepared = None;
                return;
            }
            other => panic!("{other:?}"),
        }
        if phase == 6 {
            self.renderer = None;
            self.window = None;
            if self.worker.is_some() {
                assert!(
                    self.total_waiting_frames > 0,
                    "no presentation during background work"
                );
                self.record(format!("PASS async indirect: off/on, moving sun, closed/open enclosure, stale edit and light invalidation; {} presentations continued during background preparation", self.total_waiting_frames));
            } else {
                self.record("PASS indirect: off/on, moving sun, closed/open enclosure, stale edit and light invalidation".into());
            }
            el.exit();
        }
        if let Some(report) = phase_report {
            self.record(report);
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
        self.prepared = None;
        self.record("suspended: renderer released".into());
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
