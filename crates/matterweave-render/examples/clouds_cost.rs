//! Host acceptance harness for the sky and cloud cost work.
//!
//! One synthetic "shore" framing: a ground plane, a headland that covers most
//! of the sky, and a camera a little above the ground looking level. The point
//! is coverage, not art: the counters below need geometry that hides a large
//! fraction of the cloud target the way the landscape's shore framing does.
//!
//! What it prints per frame:
//!
//! * target size and quality from [`Renderer::cloud_target`];
//! * `candidates`: cloud-target rays that reach the march loop, which is what
//!   the pre-change shader marched on that pixel;
//! * `marched`: rays the depth mask and the amortised sub-grid let through;
//! * `steps`: view-march steps those rays actually executed;
//! * `masked` / `reused`: the two ways a candidate was skipped;
//! * `sky`: sky dome fragments that reached the shading path, against the
//!   `width * height` a full-screen pre-change dome shaded.
//!
//! `--hold-seconds` keeps the last frame presented so an X11 grab can compare
//! it with the same frame from the previous revision. This is host validation
//! on a software Vulkan driver, never Android evidence and never a performance
//! measurement; llvmpipe milliseconds are noise.

use glam::{Mat4, Vec3};
use matterweave_core::{Mesh, Vertex};
use matterweave_render::{Atmosphere, Clouds, FrameResult, Hud, LightingSettings, Renderer};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

#[derive(Clone, Copy)]
struct Options {
    width: u32,
    height: u32,
    frames: u32,
    quality: u8,
    turn_deg_per_frame: f32,
    hold_seconds: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 800,
            height: 480,
            frames: 8,
            quality: 0,
            turn_deg_per_frame: 0.0,
            hold_seconds: 0.0,
        }
    }
}

fn parse_options() -> Options {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                let value = args.next().expect("--size WxH");
                let (width, height) = value.split_once('x').expect("--size WxH");
                options.width = width.parse().expect("width");
                options.height = height.parse().expect("height");
            }
            "--frames" => options.frames = args.next().expect("--frames N").parse().expect("N"),
            "--quality" => {
                let value = args.next().expect("--quality low|high");
                options.quality = match value.as_str() {
                    "low" | "0" => 0,
                    "high" | "1" => 1,
                    other => panic!("--quality takes low or high, not {other}"),
                };
            }
            "--turn" => {
                options.turn_deg_per_frame = args
                    .next()
                    .expect("--turn DEG_PER_FRAME")
                    .parse()
                    .expect("degrees")
            }
            "--hold-seconds" => {
                options.hold_seconds = args
                    .next()
                    .expect("--hold-seconds F")
                    .parse()
                    .expect("seconds")
            }
            other => panic!("unexpected argument {other}"),
        }
    }
    options
}

fn vertex(position: [f32; 3], normal: [f32; 3], color: [f32; 3]) -> Vertex {
    Vertex {
        position,
        normal,
        color,
    }
}

/// Ground plane plus a headland box. Every quad carries both windings so the
/// silhouette does not depend on the cull mode: this scene exists to occlude
/// the sky, and it must occlude it from whichever side the camera sees.
fn shore_scene() -> Mesh {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], normal: [f32; 3]| {
        let base = vertices.len() as u32;
        for position in [a, b, c, d] {
            vertices.push(vertex(position, normal, [0.32, 0.40, 0.28]));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    };
    // Ground at y = 0, large enough that its far edge sits just below the
    // horizon in this framing.
    let e = 4000.0;
    quad(
        [-e, 0.0, e],
        [e, 0.0, e],
        [e, 0.0, -e],
        [-e, 0.0, -e],
        [0.0, 1.0, 0.0],
    );
    // Headland: a box whose near face is 800 m away and 560 m tall, standing
    // left of the camera's forward line. It covers roughly two thirds of the
    // sky region the cloud march considers, so the depth mask has something
    // real to skip, while leaving the right of the frame open for clouds.
    let (near, far) = (-800.0, -1600.0);
    let (left, right) = (-1700.0, 400.0);
    let top = 560.0;
    // Camera-facing face.
    quad(
        [left, 0.0, near],
        [right, 0.0, near],
        [right, top, near],
        [left, top, near],
        [0.0, 0.0, 1.0],
    );
    // Top.
    quad(
        [left, top, near],
        [right, top, near],
        [right, top, far],
        [left, top, far],
        [0.0, 1.0, 0.0],
    );
    // Right flank, so the silhouette has a second edge in frame.
    quad(
        [right, 0.0, near],
        [right, 0.0, far],
        [right, top, far],
        [right, top, near],
        [1.0, 0.0, 0.0],
    );
    Mesh {
        vertices,
        indices,
        revision: 1,
    }
}

fn view_projection(aspect: f32, yaw_deg: f32, eye: Vec3) -> [[f32; 4]; 4] {
    let yaw = yaw_deg.to_radians();
    let forward = Vec3::new(yaw.sin(), 0.0, -yaw.cos());
    (Mat4::perspective_rh(70_f32.to_radians(), aspect.max(0.01), 0.2, 20_000.0)
        * Mat4::look_at_rh(eye, eye + forward, Vec3::Y))
    .to_cols_array_2d()
}

#[derive(Clone, Copy)]
struct FrameCounters {
    sky: u32,
    candidates: u32,
    marched: u32,
    steps: u32,
    masked: u32,
    reused: u32,
}

#[derive(Default)]
struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    options: Options,
    frame: u32,
    yaw: f32,
    full_update: Option<FrameCounters>,
    amortised_steps: Vec<u32>,
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
                        .with_title("Matterweave cloud cost")
                        .with_inner_size(winit::dpi::LogicalSize::new(
                            self.options.width,
                            self.options.height,
                        )),
                )
                .unwrap(),
        );
        let mut initialization = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(result) = initialization
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("Renderer initialization unexpectedly suspended")
        };
        let mut renderer = result.unwrap();
        println!("{}", renderer.capabilities);
        renderer.upload_chunk([0, 0, 0], &shore_scene()).unwrap();
        renderer.set_cost_counters_enabled(true);
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        let options = self.options;
        let renderer = self.renderer.as_mut().unwrap();
        let size = self.window.as_ref().unwrap().inner_size();
        let lighting = LightingSettings {
            atmosphere: Atmosphere {
                sky_gradient: true,
                ..Default::default()
            },
            clouds: Clouds {
                enabled: true,
                quality: options.quality,
                time_s: self.frame as f32 * 0.25,
            },
            ..Default::default()
        };
        let hud = Hud::new(size.width as f32, size.height as f32);
        let eye = Vec3::new(0.0, 60.0, 0.0);
        let outcome = renderer.render_with_lighting(
            view_projection(size.width as f32 / size.height as f32, self.yaw, eye),
            [eye.x, eye.y, eye.z],
            &hud,
            &lighting,
        );
        match outcome {
            FrameResult::Presented => {}
            FrameResult::Retry => return,
            result => panic!("Unexpected render result: {result:?}"),
        }
        let counters = renderer
            .cost_counters()
            .expect("counter read")
            .expect("counters enabled with a sky pass");
        let frame = FrameCounters {
            sky: counters.sky_shaded_pixels,
            candidates: counters.cloud_candidates,
            marched: counters.cloud_marched_pixels,
            steps: counters.cloud_march_steps,
            masked: counters.cloud_masked_pixels,
            reused: counters.cloud_reused_pixels,
        };
        println!(
            "CLOUD COST frame {} target {:?} sky {} candidates {} marched {} steps {} masked {} reused {}",
            self.frame,
            renderer.cloud_target(),
            frame.sky,
            frame.candidates,
            frame.marched,
            frame.steps,
            frame.masked,
            frame.reused,
        );
        if self.frame == 0 {
            self.full_update = Some(frame);
        } else if self.frame > 0 {
            self.amortised_steps.push(frame.steps);
        }
        self.yaw += options.turn_deg_per_frame;
        self.frame += 1;
        if self.frame >= options.frames {
            self.summary();
            if options.hold_seconds > 0.0 {
                // The last frame stays presented; scripts grab the window now.
                std::thread::sleep(Duration::from_secs_f32(options.hold_seconds));
            }
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl App {
    fn summary(&self) {
        let Some(full) = self.full_update else {
            println!("CLOUD COST ACCEPTANCE: SKIPPED (no frame)");
            return;
        };
        let (width, height) = match self.renderer.as_ref().and_then(Renderer::cloud_target) {
            Some((size, _)) => size,
            None => (0, 0),
        };
        let frame_pixels = self.options.width * self.options.height;
        let marched_fraction = full.marched as f64 / full.candidates.max(1) as f64;
        let masked_fraction = full.masked as f64 / full.candidates.max(1) as f64;
        let amortised_mean = if self.amortised_steps.is_empty() {
            f64::NAN
        } else {
            self.amortised_steps.iter().sum::<u32>() as f64 / self.amortised_steps.len() as f64
        };
        let steps_fraction = amortised_mean / full.steps.max(1) as f64;
        let sky_fraction = full.sky as f64 / frame_pixels.max(1) as f64;
        println!(
            "CLOUD COST SUMMARY target {width}x{height} full-update candidates {} marched {} ({:.1}% fall) steps {} amortised mean steps {:.0} ({:.1}% fall) sky {}/{} ({:.1}% shaded)",
            full.candidates,
            full.marched,
            100.0 * (1.0 - marched_fraction),
            full.steps,
            amortised_mean,
            100.0 * (1.0 - steps_fraction),
            full.sky,
            frame_pixels,
            100.0 * sky_fraction,
        );
        let pass = marched_fraction <= 0.55 && steps_fraction <= 0.40 && sky_fraction <= 0.60;
        println!(
            "CLOUD COST ACCEPTANCE: {} (marched <=55% {:.3}, steps <=40% {:.3}, sky <=60% {:.3})",
            if pass { "PASS" } else { "FAIL" },
            marched_fraction,
            steps_fraction,
            sky_fraction
        );
        println!(
            "CLOUD COST DETAIL masked {:.1}% of candidates, reused {:.1}%",
            100.0 * masked_fraction,
            100.0 * (full.reused as f64 / full.candidates.max(1) as f64),
        );
    }
}

fn main() {
    let options = parse_options();
    let mut app = App {
        options,
        ..App::default()
    };
    EventLoop::new().unwrap().run_app(&mut app).unwrap();
}
