//! Vulkan validation exercise for the Renderer's reflection capability.
//!
//! This drives the **real** `Renderer` (window, swapchain, frame fence, shadow
//! descriptor set) through enabling, disabling, editing, resizing and full
//! renderer recreation. Run under Xvfb/lavapipe with `MATTERWEAVE_VALIDATION=1`
//! and require an empty validation stream; it is not Android or performance
//! evidence and makes no pixel-quality claim (see `reflection_validation`).
use glam::Mat4;
use matterweave_core::{Mesh, World};
use matterweave_render::reflection::{MaterialTable, ReflectionVolume, SURFACE_OFFSET};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
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

const MIRROR: u8 = 7;
const STONE: u8 = 2;
const TARGET: u8 = 1;
const ORIGIN: [i32; 3] = [-8, -4, -8];
const DIMENSIONS: [u32; 3] = [16, 12, 16];

fn fill(world: &mut World, lo: [i32; 3], hi: [i32; 3], material: u8) {
    for x in lo[0]..hi[0] {
        for y in lo[1]..hi[1] {
            for z in lo[2]..hi[2] {
                world.set([x, y, z], material);
            }
        }
    }
}

fn scene() -> World {
    let mut world = World::new(71);
    fill(&mut world, [-6, -1, -6], [6, 0, 6], MIRROR);
    fill(&mut world, [3, 0, -1], [5, 3, 1], TARGET);
    fill(&mut world, [5, 0, -6], [6, 5, 6], STONE);
    world
}

fn table() -> MaterialTable {
    let mut color = [[0.5; 3]; 256];
    color[0] = [0.; 3];
    color[MIRROR as usize] = [0.30, 0.34, 0.38];
    color[STONE as usize] = [0.62, 0.63, 0.60];
    color[TARGET as usize] = [0.90, 0.10, 0.05];
    let mut table = MaterialTable::new(color).unwrap();
    table.set_mirror(MIRROR, 1.0).unwrap();
    table
}

/// Reuse the authoritative mesher, then recolour from the material table.
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

fn camera() -> [[f32; 4]; 4] {
    let view = Mat4::look_at_rh(
        glam::Vec3::new(9.0, 6.0, 9.0),
        glam::Vec3::new(0.0, 0.0, 0.0),
        glam::Vec3::Y,
    );
    let projection = Mat4::perspective_rh(1.0, 1.4, 0.1, 100.0);
    (projection * view).to_cols_array_2d()
}

struct App {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    frame: u32,
    prepared: Option<u32>,
    world: World,
    epoch: u64,
    volume: Option<ReflectionVolume>,
    stale_publish_rejected: bool,
    publication_frames: Vec<u64>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            renderer: None,
            window: None,
            frame: 0,
            prepared: None,
            world: scene(),
            epoch: 0,
            volume: None,
            stale_publish_rejected: false,
            publication_frames: Vec::new(),
        }
    }
}

fn spawn(window: Arc<Window>) -> Renderer {
    let mut initialization = Box::pin(Renderer::new(window));
    let Poll::Ready(result) = initialization
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    else {
        panic!("Renderer initialization unexpectedly suspended")
    };
    result.unwrap()
}

impl App {
    /// Full publication path: rebuild the mesh, upload it, then publish a fresh
    /// volume for the *current* world and epoch.
    fn republish(&mut self) {
        let renderer = self.renderer.as_mut().unwrap();
        let table = table();
        let mut mesh = self.world.mesh();
        recolor(&mut mesh, &self.world, &table);
        renderer.upload(&mesh).unwrap();
        let volume = ReflectionVolume::pack(
            &self.world,
            self.epoch,
            ORIGIN,
            DIMENSIONS,
            &table,
            matterweave_render::reflection::DEFAULT_TRACE_STEPS,
        )
        .unwrap();
        let stats = renderer
            .upload_reflection(&volume, &self.world, self.epoch, None)
            .unwrap();
        assert!(stats.bytes > 0);
        // Publication precedes the frame that presents it.
        let published = renderer
            .reflection_state()
            .published_submission
            .expect("publication records its submission");
        self.publication_frames.push(published);
        self.volume = Some(volume);
        println!(
            "frame {}: published {} bytes (fence wait {:.3}ms, upload {:.3}ms) at submission {published}",
            self.frame, stats.bytes, stats.fence_wait_ms, stats.upload_ms
        );
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes().with_title("Matterweave reflection validation"),
                )
                .unwrap(),
        );
        let renderer = spawn(window.clone());
        println!("{}", renderer.capabilities);
        self.renderer = Some(renderer);
        self.window = Some(window);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if !matches!(event, WindowEvent::RedrawRequested) {
            return;
        }
        if self.prepared != Some(self.frame) {
            match self.frame {
                0 => {
                    assert!(
                        !self.renderer.as_ref().unwrap().reflection_enabled(),
                        "off by default"
                    );
                    self.republish();
                    assert!(self.renderer.as_ref().unwrap().reflection_enabled());
                }
                1 => {
                    // An edit invalidates: the next frame must not use stale data.
                    let renderer = self.renderer.as_mut().unwrap();
                    self.world.set([0, 1, 0], TARGET);
                    let table = table();
                    let mut mesh = self.world.mesh();
                    recolor(&mut mesh, &self.world, &table);
                    renderer.upload(&mesh).unwrap();
                    assert!(
                        !renderer.reflection_enabled(),
                        "geometry upload must disable stale reflection"
                    );
                }
                2 => {
                    // A delayed/obsolete result cannot publish.
                    let stale = self.volume.take().unwrap();
                    let rejected = {
                        let renderer = self.renderer.as_mut().unwrap();
                        let result =
                            renderer.upload_reflection(&stale, &self.world, self.epoch, None);
                        let disabled = !renderer.reflection_enabled();
                        (result.is_err(), disabled)
                    };
                    assert!(rejected.0, "an edited world must reject the previous pack");
                    assert!(rejected.1, "a rejected publish must disable reflection");
                    self.stale_publish_rejected = true;
                    self.republish();
                }
                3 => {
                    // Scene replacement with a new epoch; the old pack is refused.
                    let old = self.volume.take().unwrap();
                    let mut replacement = scene();
                    replacement.set([0, 4, 0], STONE);
                    self.epoch += 1;
                    self.world = replacement;
                    let refused = {
                        let renderer = self.renderer.as_mut().unwrap();
                        renderer
                            .upload_reflection(&old, &self.world, self.epoch, None)
                            .is_err()
                            || !old.valid_for(&self.world, self.epoch)
                    };
                    assert!(refused, "replaced scene must not accept the old pack");
                    self.republish();
                }
                4 => {
                    let renderer = self.renderer.as_mut().unwrap();
                    renderer.disable_reflection();
                    assert!(!renderer.reflection_enabled());
                }
                5 => {
                    self.republish();
                    let size = self.window.as_ref().unwrap().inner_size();
                    self.renderer
                        .as_mut()
                        .unwrap()
                        .resize(size.width.max(640), size.height.max(480));
                }
                6 => {
                    assert!(self.renderer.as_ref().unwrap().reflection_enabled());
                    // Recreate the whole renderer while reflection is published.
                    self.renderer = None;
                    let replacement = spawn(self.window.as_ref().unwrap().clone());
                    self.renderer = Some(replacement);
                    assert!(
                        !self.renderer.as_ref().unwrap().reflection_enabled(),
                        "a fresh renderer owns no reflection data"
                    );
                    self.republish();
                }
                _ => {}
            }
            self.prepared = Some(self.frame);
        }
        let lighting = LightingSettings {
            shadows: true,
            sun: Sun {
                direction_to_sun: [0.4, 0.85, 0.3],
                intensity: 0.8,
            },
            ..Default::default()
        };
        let size = self.window.as_ref().unwrap().inner_size();
        match self.renderer.as_mut().unwrap().render_with_lighting(
            camera(),
            [9.0, 6.0, 9.0],
            &Hud::new(size.width as f32, size.height as f32),
            &lighting,
        ) {
            FrameResult::Presented => {
                let state = self.renderer.as_ref().unwrap().reflection_state();
                if self.renderer.as_ref().unwrap().reflection_enabled() {
                    let presented = state
                        .presented_submission
                        .expect("published reflection is presented");
                    let published = state.published_submission.unwrap();
                    assert!(
                        presented >= published,
                        "presentation {presented} must follow publication {published}"
                    );
                    self.frame += 1;
                } else if self.frame == 4 {
                    assert_eq!(state.published_submission, None);
                    self.frame += 1;
                } else {
                    self.frame += 1;
                }
            }
            FrameResult::Retry => return,
            result => panic!("Unexpected render result: {result:?}"),
        }
        if self.frame == 9 {
            let state = self.renderer.as_ref().unwrap().reflection_state();
            println!(
                "reflection_smoke: {} bytes resident (materials {} + palette {}), publications at {:?}, presented at {:?}",
                state.material_bytes + state.palette_bytes,
                state.material_bytes,
                state.palette_bytes,
                self.publication_frames,
                state.presented_submission
            );
            assert!(self.stale_publish_rejected);
            self.renderer = None;
            self.window = None;
            println!("reflection_smoke: enabling, edit invalidation, stale rejection, epoch replacement, disable, resize and renderer recreation passed");
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
