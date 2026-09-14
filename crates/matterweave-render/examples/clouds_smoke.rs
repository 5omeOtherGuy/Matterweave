//! Isolated Vulkan validation exercise for the sky dome and the volumetric
//! cloud pass: off, both quality levels, a resize under load and back to off,
//! checking that the offscreen target is allocated only while clouds are on and
//! that it follows the frame. Run under Xvfb/lavapipe; this is neither Android
//! device evidence nor a performance measurement.
use glam::{Mat4, Vec3};
use matterweave_core::{Mesh, Vertex};
use matterweave_render::{Atmosphere, Clouds, FrameResult, Hud, LightingSettings, Renderer};
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

/// An eye below the cloud base looking up into it, so the march actually
/// integrates the slab instead of leaving on the first ray test.
fn view_projection(aspect: f32) -> [[f32; 4]; 4] {
    let eye = Vec3::new(0., 120., 0.);
    (Mat4::perspective_rh(70_f32.to_radians(), aspect.max(0.01), 0.2, 8000.)
        * Mat4::look_at_rh(eye, eye + Vec3::new(0.6, 0.45, -0.6), Vec3::Y))
    .to_cols_array_2d()
}

fn ground() -> Mesh {
    Mesh {
        vertices: [[-50., 0., -50.], [50., 0., -50.], [0., 0., 50.]]
            .map(|position| Vertex {
                position,
                normal: [0., 1., 0.],
                color: [0.35, 0.45, 0.3],
            })
            .to_vec(),
        indices: vec![0, 1, 2],
        revision: 1,
    }
}

fn settings(sky_gradient: bool, clouds: Option<u8>, time_s: f32) -> LightingSettings {
    LightingSettings {
        atmosphere: Atmosphere {
            sky_gradient,
            ..Default::default()
        },
        clouds: Clouds {
            enabled: clouds.is_some(),
            quality: clouds.unwrap_or(0),
            time_s,
        },
        ..Default::default()
    }
}

#[derive(Default)]
struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    frame: u32,
    low: Option<(u32, u32)>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Matterweave cloud validation")
                        .with_inner_size(winit::dpi::LogicalSize::new(640, 400)),
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
        renderer.upload_chunk([0, 0, 0], &ground()).unwrap();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        let frame = self.frame;
        if frame == 4 {
            let window = self.window.as_ref().unwrap();
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(800, 480));
            self.renderer.as_mut().unwrap().resize(800, 480);
        }
        let renderer = self.renderer.as_mut().unwrap();
        let size = self.window.as_ref().unwrap().inner_size();
        // Frame 0 is the untouched default: no dome, no clouds, no target.
        // 1 adds the dome, 2 and 3 the cheap layer, 4 resizes it, 5 the
        // expensive one, 6 switches everything off again.
        let lighting = match frame {
            0 => settings(false, None, 0.),
            1 => settings(true, None, 0.),
            2..=4 => settings(true, Some(0), frame as f32 * 30.),
            5 => settings(true, Some(1), 150.),
            _ => settings(false, None, 180.),
        };
        let hud = Hud::new(size.width as f32, size.height as f32);
        let outcome = renderer.render_with_lighting(
            view_projection(size.width as f32 / size.height as f32),
            [0., 120., 0.],
            &hud,
            &lighting,
        );
        match outcome {
            FrameResult::Presented => {}
            FrameResult::Retry => return,
            result => panic!("Unexpected render result: {result:?}"),
        }
        let target = renderer.cloud_target();
        match frame {
            0 | 1 => assert_eq!(target, None, "a disabled cloud pass allocates no target"),
            2..=4 => {
                let ((width, height), quality) = target.expect("cheap cloud target allocated");
                assert_eq!(quality, 0);
                assert!(width > 0 && height > 0);
                assert!(
                    width * 2 < size.width.max(1) && height * 2 < size.height.max(1),
                    "the cheap target is well below half resolution: {width}x{height} \
                     of {}x{}",
                    size.width,
                    size.height
                );
                if frame == 2 {
                    self.low = Some((width, height));
                }
                if frame == 4 {
                    assert_ne!(
                        self.low,
                        Some((width, height)),
                        "the target follows a resized frame"
                    );
                }
            }
            5 => {
                let ((width, height), quality) = target.expect("quality cloud target allocated");
                assert_eq!(quality, 1);
                assert!(width * 2 >= size.width && height * 2 >= size.height);
                assert!(width <= size.width && height <= size.height);
            }
            _ => assert_eq!(target, None, "switching clouds off frees the target"),
        }
        if renderer.gpu_timestamps_supported() {
            if let Some(timings) = renderer.gpu_timings() {
                assert!(timings.render_ms.is_finite() && timings.render_ms >= 0.);
                if let Some(cloud_ms) = timings.cloud_ms {
                    assert!(cloud_ms.is_finite() && cloud_ms >= 0.);
                    assert!(
                        cloud_ms <= timings.render_ms,
                        "the cloud pass is part of the frame it is measured in"
                    );
                    println!(
                        "clouds_smoke: completed GPU frame {} total {:.3} ms cloud {cloud_ms:.3} ms",
                        timings.frame_id, timings.render_ms
                    );
                }
            }
        }
        self.frame += 1;
        if self.frame == 8 {
            // Tear down while the last submission may still be running.
            self.renderer = None;
            self.window = None;
            println!(
                "clouds_smoke: eight frames passed; dome off/on, cloud quality 0 and 1, resize \
                 while marching, target freed when disabled, teardown"
            );
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
