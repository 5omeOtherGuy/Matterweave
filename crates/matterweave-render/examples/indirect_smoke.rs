//! Native Vulkan R09 first-bounce gate, not a mobile performance benchmark.
//! Optional MW_INDIRECT_CAPTURE_DIR uses host xwd for phase image evidence.
use glam::{Mat4, Vec3};
use matterweave_core::World;
use matterweave_render::{
    indirect::{IndirectVolume, UpdateBudget},
    FrameResult, Hud, LightingSettings, Renderer, Sun,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
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
fn capture(window: &Window, phase: u32) {
    let Some(dir) = std::env::var_os("MW_INDIRECT_CAPTURE_DIR") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    let RawWindowHandle::Xlib(handle) = window.window_handle().unwrap().as_raw() else {
        panic!("optional xwd capture requires WINIT_UNIX_BACKEND=x11");
    };
    let path = std::path::PathBuf::from(dir).join(format!("phase-{phase}.xwd"));
    let status = std::process::Command::new("xwd")
        .args(["-silent", "-id", &handle.window.to_string(), "-out"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}
struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    world: World,
    cache: IndirectVolume,
    frame: u32,
    prepared: Option<u32>,
    lighting: LightingSettings,
}
impl Default for App {
    fn default() -> Self {
        Self {
            renderer: None,
            window: None,
            world: scene(),
            cache: IndirectVolume::new([-3, -1, -3], [7, 5, 7], 64, 32., palette()).unwrap(),
            frame: 0,
            prepared: None,
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
impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
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
        println!("{}", r.capabilities);
        assert!(!r.indirect_enabled());
        upload_world(&mut r, &self.world);
        self.renderer = Some(r);
        self.window = Some(window);
    }
    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        let phase = self.frame / 6;
        let r = self.renderer.as_mut().unwrap();
        if self.prepared != Some(phase) {
            match phase {
                2 => self.lighting.sun.direction_to_sun = [0.65, 1., 0.2],
                3 => {
                    roof(&mut self.world, true);
                    upload_world(r, &self.world);
                    assert!(!r.indirect_enabled());
                    assert!(
                        r.upload_indirect(&self.cache, &self.world, 0, None)
                            .is_err(),
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
            if phase == 0 || phase == 5 {
                r.disable_indirect();
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
                r.upload_indirect(&self.cache, &self.world, 0, None)
                    .unwrap();
                println!(
                    "phase={phase} cpu_producer_ms={:.3} rays={rays} bytes={} sample={sample:?}",
                    start.elapsed().as_secs_f64() * 1000.,
                    self.cache.resident_bytes()
                );
            }
            self.prepared = Some(phase);
        }
        let eye = Vec3::new(1.8, 1.6, 1.8);
        let view = Mat4::perspective_rh(70f32.to_radians(), 640. / 480., 0.05, 100.)
            * Mat4::look_at_rh(eye, Vec3::new(-2., 0.9, -1.), Vec3::Y);
        match r.render_with_lighting(
            view.to_cols_array_2d(),
            eye.to_array(),
            &Hud::new(640., 480.),
            &self.lighting,
        ) {
            FrameResult::Presented => {
                assert_eq!(r.indirect_enabled(), phase != 0 && phase != 5);
                // Capture an already presented steady-state phase before next edits.
                if self.frame % 6 == 5 {
                    capture(self.window.as_ref().unwrap(), phase);
                }
                self.frame += 1;
            }
            FrameResult::Retry => return,
            other => panic!("{other:?}"),
        }
        if self.frame == 36 {
            // Real light invalidation without publishing another CPU cache.
            r.upload_indirect(&self.cache, &self.world, 0, None)
                .unwrap();
            assert!(r.indirect_enabled());
            self.lighting.sun.intensity = 0.;
            let result = r.render_with_lighting(
                view.to_cols_array_2d(),
                eye.to_array(),
                &Hud::new(640., 480.),
                &self.lighting,
            );
            assert_eq!(result, FrameResult::Presented);
            assert!(!r.indirect_enabled());
            self.renderer = None;
            self.window = None;
            println!("indirect_smoke: PASS off/on, moving sun, closed/open enclosure, stale edit rejection; Android costs unmeasured");
            el.exit();
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}
fn main() {
    EventLoop::new()
        .unwrap()
        .run_app(&mut App::default())
        .unwrap();
}
