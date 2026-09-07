//! Isolated Vulkan validation exercise for chunk replacement and dynamic buffers.
//! Run under Xvfb/lavapipe; this is not Android device or performance evidence.
use matterweave_core::{Mesh, Vertex};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer};
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
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn triangle(x: f32, revision: u64) -> Mesh {
    Mesh {
        vertices: [[x, -0.5, 0.5], [x + 0.5, -0.5, 0.5], [x, 0.5, 0.5]]
            .map(|position| Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                color: [0.4, 0.8, 0.5],
            })
            .to_vec(),
        indices: vec![0, 1, 2],
        revision,
    }
}

#[derive(Default)]
struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    frame: u32,
    prepared_frame: Option<u32>,
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes().with_title("Matterweave cache validation"),
                )
                .unwrap(),
        );
        // Renderer initialization currently has no suspension points.
        let mut initialization = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(result) = initialization
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("Renderer initialization unexpectedly suspended")
        };
        let mut renderer = result.unwrap();
        println!("{}", renderer.capabilities);
        renderer
            .upload_chunk([0, 0, 0], &triangle(-0.5, 2))
            .unwrap();
        renderer.upload_chunk([1, 0, 0], &triangle(5.0, 1)).unwrap();
        renderer
            .upload_chunk(
                [2, 0, 0],
                &Mesh {
                    revision: 7,
                    ..Mesh::default()
                },
            )
            .unwrap();
        assert_eq!(renderer.chunk_revision([2, 0, 0]), Some(7));
        assert_eq!(renderer.resident_chunks, 3);
        self.renderer = Some(renderer);
        self.window = Some(window);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        let renderer = self.renderer.as_mut().unwrap();
        if self.prepared_frame != Some(self.frame) {
            match self.frame {
                1 => {
                    // Replacement/eviction happen while the previous frame may be in flight.
                    renderer.upload_chunk([0, 0, 0], &triangle(5.0, 1)).unwrap();
                    assert_eq!(renderer.chunk_revision([0, 0, 0]), Some(2));
                    let previous_bytes = renderer.mesh_bytes;
                    let mut invalid = triangle(-0.5, 3);
                    invalid.indices.push(500);
                    assert!(renderer.upload_chunk([0, 0, 0], &invalid).is_err());
                    assert_eq!(renderer.chunk_revision([0, 0, 0]), Some(2));
                    assert_eq!(renderer.mesh_bytes, previous_bytes);
                    renderer.upload_dynamic(&triangle(-0.3, 1)).unwrap();
                    renderer.retain_chunks(&[[0, 0, 0], [1, 0, 0]]).unwrap();
                    assert_eq!(renderer.resident_chunks, 2);
                    assert_eq!(renderer.chunk_revision([2, 0, 0]), None);
                }
                2 => {
                    let previous_bytes = renderer.mesh_bytes;
                    renderer.upload_dynamic(&triangle(-0.2, 1)).unwrap();
                    assert_eq!(renderer.mesh_bytes, previous_bytes);
                    let mut larger = triangle(0.0, 2);
                    larger.indices = [0, 1, 2].repeat(64);
                    renderer.upload_dynamic(&larger).unwrap();
                    assert!(renderer.mesh_bytes > previous_bytes);
                }
                3 => {
                    renderer.upload_dynamic(&Mesh::default()).unwrap();
                    renderer
                        .upload_chunk([0, 0, 0], &triangle(-0.4, 3))
                        .unwrap();
                }
                4 => {
                    renderer.retain_chunks(&[]).unwrap();
                    renderer.upload(&triangle(-0.5, 4)).unwrap();
                    assert_eq!(renderer.mesh_revision, Some(4));
                }
                5 => {
                    let _ = self
                        .window
                        .as_ref()
                        .unwrap()
                        .request_inner_size(winit::dpi::PhysicalSize::new(800, 600));
                    renderer.resize(800, 600);
                }
                6 => {
                    renderer.resize(0, 0);
                    assert_eq!(
                        renderer.render(IDENTITY, [0.; 3], &Hud::new(1., 1.)),
                        FrameResult::Retry
                    );
                    renderer.resize(800, 600);
                }
                _ => {}
            }
            self.prepared_frame = Some(self.frame);
        }
        let size = self.window.as_ref().unwrap().inner_size();
        let lighting = LightingSettings {
            shadows: matches!(self.frame, 1 | 3 | 5 | 7 | 9),
            shadow_map_size: if matches!(self.frame, 3 | 4) {
                2048
            } else {
                1024
            },
            ..Default::default()
        };
        match renderer.render_with_lighting(
            IDENTITY,
            [0.0; 3],
            &Hud::new(size.width as f32, size.height as f32),
            &lighting,
        ) {
            FrameResult::Presented => {
                if self.frame == 0 {
                    assert_eq!(renderer.visible_chunks, 1);
                }
                if self.frame == 1 {
                    // The x=5 terrain is outside the camera but contributes to shadows.
                    assert_eq!(renderer.visible_chunks, 1);
                    assert_eq!(renderer.shadow_caster_meshes(), 3);
                }
                if !lighting.shadows {
                    assert_eq!(renderer.shadow_caster_meshes(), 0);
                }
                if self.frame == 2 && renderer.gpu_timestamps_supported() {
                    let timing = renderer.gpu_timings().expect("completed queries available");
                    assert!(timing.render_ms >= 0. && timing.render_ms.is_finite());
                    assert!(timing.shadows && timing.shadow_ms.is_some());
                    println!(
                        "cache_smoke: completed GPU frame {} total {}ms shadow {:?}ms",
                        timing.frame_id, timing.render_ms, timing.shadow_ms
                    );
                }
                self.frame += 1;
            }
            FrameResult::Retry => return,
            result => panic!("Unexpected render result: {result:?}"),
        }
        if self.frame == 8 {
            // Drop a renderer with submitted shadow work, then initialize a fresh
            // device/map. Its first frame is shadows off, followed by shadows on.
            self.renderer = None;
            let mut initialization = Box::pin(Renderer::new(self.window.as_ref().unwrap().clone()));
            let Poll::Ready(result) = initialization
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            else {
                panic!("unexpected initialization suspension")
            };
            let mut replacement = result.unwrap();
            replacement.upload(&triangle(-0.5, 1)).unwrap();
            self.renderer = Some(replacement);
        }
        if self.frame == 10 {
            // Explicit teardown while the final submitted frame may still be running.
            self.renderer = None;
            self.window = None;
            println!("cache_smoke: ten frames passed; shadow off/on/off, 1024/2048, offscreen casters, resize/zero extent/recreation, stale uploads, invalid upload retention, culling, eviction, dynamic reuse/growth/empty and teardown");
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
