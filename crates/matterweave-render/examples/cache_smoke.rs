//! Isolated Vulkan validation exercise for chunk replacement and dynamic buffers.
//! Run under Xvfb/lavapipe; this is not Android device or performance evidence.
use matterweave_core::{Mesh, Vertex};
use matterweave_render::{FrameResult, Hud, Renderer};
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
                    renderer.retain_chunks(&[[0, 0, 0]]).unwrap();
                    assert_eq!(renderer.resident_chunks, 1);
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
                _ => {}
            }
            self.prepared_frame = Some(self.frame);
        }
        let size = self.window.as_ref().unwrap().inner_size();
        match renderer.render(
            IDENTITY,
            [0.0; 3],
            &Hud::new(size.width as f32, size.height as f32),
        ) {
            FrameResult::Presented => {
                if self.frame == 0 {
                    assert_eq!(renderer.visible_chunks, 1);
                }
                self.frame += 1;
            }
            FrameResult::Retry => return,
            result => panic!("Unexpected render result: {result:?}"),
        }
        if self.frame == 6 {
            // Explicit teardown while the final submitted frame may still be running.
            self.renderer = None;
            self.window = None;
            println!("cache_smoke: six frames passed; stale uploads, invalid upload retention, culling, eviction, dynamic reuse/growth/empty and teardown");
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
