//! Actual Vulkan shadow reuse/invalidation contract, independent of sample content.
use matterweave_core::{Mesh, Vertex};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, StaticInstance};
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

const IDENTITY: [[f32; 4]; 4] = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
fn triangle(x: f32, revision: u64) -> Mesh {
    Mesh {
        vertices: [[x, -0.5, 0.5], [x + 0.5, -0.5, 0.5], [x, 0.5, 0.5]]
            .map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                color: [0.4, 0.8, 0.5],
            })
            .to_vec(),
        indices: vec![0, 1, 2],
        revision,
    }
}
fn renderer(window: Arc<Window>) -> Renderer {
    let mut init = Box::pin(Renderer::new(window));
    let Poll::Ready(result) = init.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
        panic!("unexpected initialization suspension")
    };
    result.unwrap()
}
fn placement(x: f32) -> StaticInstance {
    StaticInstance {
        prototype: 0,
        translation: [x, 0., 0.],
        yaw_quarters: 1,
    }
}
#[derive(Default)]
struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    frame: u32,
    prepared: Option<u32>,
    previous: Option<(bool, bool)>,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Matterweave engine shadow cache validation"),
                )
                .unwrap(),
        );
        let mut r = renderer(window.clone());
        println!("{}", r.capabilities);
        r.upload_chunk([0, 0, 0], &triangle(-0.5, 2)).unwrap();
        self.renderer = Some(r);
        self.window = Some(window);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        if self.frame == 27 && self.prepared != Some(27) {
            self.renderer = None;
            self.renderer = Some(renderer(self.window.as_ref().unwrap().clone()));
            self.previous = None;
        }
        let r = self.renderer.as_mut().unwrap();
        if self.prepared != Some(self.frame) {
            match self.frame {
                5 => r.upload_dynamic(&triangle(-0.3, 1)).unwrap(),
                6 => r.upload_dynamic(&triangle(-0.2, 1)).unwrap(), // same revision, different transform
                7 => r.upload_dynamic(&Mesh::default()).unwrap(),
                9 => r.upload_chunk([0, 0, 0], &triangle(-0.4, 3)).unwrap(),
                10 => r.upload_chunk([0, 0, 0], &triangle(-0.1, 1)).unwrap(), // rejected stale
                11 => r.retain_chunks(&[]).unwrap(),
                12 => r.upload(&triangle(-0.5, 4)).unwrap(),
                13 => {
                    r.replace_static_scene(&[triangle(0., 1)], &[placement(1.)])
                        .unwrap();
                }
                14 => {
                    let mut invalid = triangle(0., 1);
                    invalid.indices.push(100);
                    assert!(r
                        .replace_static_scene(&[invalid], &[placement(1.)])
                        .is_err());
                }
                15 => {
                    r.replace_static_scene(&[triangle(0., 1)], &[placement(2.)])
                        .unwrap();
                }
                16 => {
                    r.replace_static_scene(&[], &[]).unwrap();
                }
                21 => r.upload_dynamic(&triangle(-0.1, 1)).unwrap(),
                23 => r.set_world_visible(false),
                24 => r.set_world_visible(true),
                25 => {
                    let _ = self
                        .window
                        .as_ref()
                        .unwrap()
                        .request_inner_size(winit::dpi::PhysicalSize::new(800, 600));
                    r.resize(800, 600);
                }
                26 => {
                    r.resize(0, 0);
                    assert_eq!(
                        r.render_with_lighting(
                            IDENTITY,
                            [0.; 3],
                            &Hud::new(1., 1.),
                            &Default::default()
                        ),
                        FrameResult::Retry
                    );
                    assert!(!r.shadow_map_updated());
                    r.resize(800, 600);
                }
                _ => {}
            }
            self.prepared = Some(self.frame);
        }
        let mut lighting = LightingSettings {
            shadows: !matches!(self.frame, 0 | 20 | 21 | 27),
            shadow_map_size: if self.frame >= 19 { 2048 } else { 1024 },
            ..Default::default()
        };
        if self.frame >= 3 {
            lighting.sun.intensity = 1.4;
        }
        if self.frame >= 17 {
            lighting.sun.direction_to_sun = [0., 1., 0.];
        }
        let eye = if self.frame >= 18 {
            [8., 0., 0.]
        } else {
            [0.; 3]
        };
        let mut view = IDENTITY;
        if self.frame >= 4 {
            view[0][0] = -1.;
        } // view orientation alone does not change depth
        let expected_update = matches!(
            self.frame,
            0 | 1 | 5 | 6 | 7 | 9 | 11 | 12 | 13 | 15 | 16 | 17 | 18 | 19 | 22 | 27 | 28
        );
        let size = self.window.as_ref().unwrap().inner_size();
        match r.render_with_lighting(
            view,
            eye,
            &Hud::new(size.width as f32, size.height as f32),
            &lighting,
        ) {
            FrameResult::Presented => {
                assert_eq!(
                    r.shadow_map_updated(),
                    expected_update,
                    "frame {}",
                    self.frame
                );
                if !expected_update || !lighting.shadows {
                    assert_eq!(r.shadow_caster_meshes(), 0);
                }
                if let Some((previous_enabled, previous_update)) = self.previous {
                    if r.gpu_timestamps_supported() {
                        let timing = r.gpu_timings().expect("completed timestamp queries");
                        assert_eq!(timing.shadows, previous_enabled);
                        assert_eq!(timing.shadow_map_updated, previous_update);
                        assert_eq!(
                            timing.shadow_ms.is_some(),
                            previous_enabled && previous_update
                        );
                    }
                }
                self.previous = Some((lighting.shadows && self.frame != 23, expected_update));
                println!(
                    "shadow_cache_smoke frame={} updated={} casters={}",
                    self.frame,
                    r.shadow_map_updated(),
                    r.shadow_caster_meshes()
                );
                self.frame += 1;
            }
            FrameResult::Retry => return,
            other => panic!("unexpected result: {other:?}"),
        }
        if self.frame == 30 {
            self.renderer = None;
            self.window = None;
            println!("shadow_cache_smoke: PASS 30 frames, reuse and all geometry/light/lifecycle invalidations; no device performance claim");
            event_loop.exit();
        }
    }
    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
fn main() {
    EventLoop::new()
        .unwrap()
        .run_app(&mut App::default())
        .unwrap();
}
