//! Native platform/sample orchestration. Authoritative world and GPU backend are separate crates.
mod controls;
use controls::{Action, Camera, Controls};
use glam::Vec2;
use matterweave_core::World;
use matterweave_render::{FrameResult, Hud, Renderer};
use std::{path::PathBuf, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const SEED: u64 = 20260907;
const EDIT_RANGE: f32 = 12.;
struct Explorer {
    // Renderer must be dropped before the Android suspend callback returns.
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    world: World,
    camera: Camera,
    controls: Controls,
    save_path: PathBuf,
    dirty: bool,
    recovery: bool,
    smoke_exercise: bool,
    smoke_stage: u8,
    smoke_resize_seen: bool,
    status: String,
    last_frame: Instant,
    frame_ms: f64,
    cpu_ms: f64,
    mesh_ms: f64,
    frames: u64,
    smoke_frames: Option<u64>,
    failed: bool,
    focused: bool,
}
impl Explorer {
    fn new(mut save_path: PathBuf, smoke_frames: Option<u64>) -> Self {
        let mut recovery = false;
        let (world, status) = if save_path.exists() {
            match World::load(&save_path) {
                Ok(world) => (world, "Loaded saved world".into()),
                Err(e) => {
                    let original = save_path.clone();
                    let name = original.file_name().unwrap_or_default().to_string_lossy();
                    let mut sequence = 1_u64;
                    let mut recovered = None;
                    loop {
                        let candidate =
                            original.with_file_name(format!("{name}.recovery-{sequence}.json"));
                        if !candidate.exists() {
                            save_path = candidate;
                            break;
                        }
                        if let Ok(world) = World::load(&candidate) {
                            recovered = Some((candidate, world));
                        }
                        sequence += 1;
                    }
                    let recovered_world = recovered.map(|(path, world)| {
                        save_path = path;
                        world
                    });
                    recovery = true;
                    log::error!(
                        "Load failed for {}: {e}. Original retained; recovery saves use {}",
                        original.display(),
                        save_path.display()
                    );
                    eprintln!(
                        "Load failed for {}: {e}. Original retained; recovery saves use {}",
                        original.display(),
                        save_path.display()
                    );
                    (
                        recovered_world.unwrap_or_else(|| World::generate(SEED)),
                        "LOAD FAILED: original kept; recovery active".into(),
                    )
                }
            }
        } else {
            (World::generate(SEED), "Explore. Edits autosave.".into())
        };
        Self {
            renderer: None,
            window: None,
            world,
            camera: Camera::default(),
            controls: Controls::default(),
            save_path,
            dirty: false,
            recovery,
            smoke_exercise: false,
            smoke_stage: 0,
            smoke_resize_seen: false,
            status,
            last_frame: Instant::now(),
            frame_ms: 0.,
            cpu_ms: 0.,
            mesh_ms: 0.,
            frames: 0,
            smoke_frames,
            failed: false,
            focused: true,
        }
    }
    fn save(&mut self) {
        match self.world.save(&self.save_path) {
            Ok(()) => {
                self.dirty = false;
                self.status = if self.recovery {
                    "Recovery saved; original retained"
                } else {
                    "World saved"
                }
                .into();
            }
            Err(e) => {
                self.status = format!("SAVE FAILED: {e}");
                log::error!("{}", self.status);
                eprintln!("{}", self.status);
            }
        }
    }
    fn action(&mut self, action: Action) {
        match action {
            Action::Save => self.save(),
            Action::Swap => {
                self.controls.swapped = !self.controls.swapped;
                self.controls.clear();
            }
            Action::Size => {
                self.controls.large = !self.controls.large;
                self.controls.clear();
            }
            Action::Remove | Action::Place => {
                if let Some(hit) = self.world.raycast(
                    self.camera.position.to_array(),
                    self.camera.forward().to_array(),
                    EDIT_RANGE,
                ) {
                    let target = if action == Action::Remove {
                        hit.cell
                    } else {
                        [
                            hit.cell[0] + hit.normal[0],
                            hit.cell[1] + hit.normal[1],
                            hit.cell[2] + hit.normal[2],
                        ]
                    };
                    if target.iter().any(|v| !(-64..64).contains(v)) {
                        self.status = "Edit outside sample bounds".into();
                        return;
                    }
                    let material = if action == Action::Remove { 0 } else { 4 };
                    if action == Action::Place && self.world.get(target) != 0 {
                        self.status = "Placement needs an empty adjacent cell".into();
                        return;
                    }
                    if self.world.set(target, material) {
                        self.dirty = true;
                        self.save();
                    }
                } else {
                    self.status = "Aim at a voxel within 12 units".into();
                }
            }
        }
    }
    // Host supporting evidence: invokes the actual application edit and lifecycle paths.
    // This does not emulate Android surface callbacks or establish device compatibility.
    fn exercise(&mut self, event_loop: &ActiveEventLoop) {
        let result: Result<(), String> = match self.smoke_stage {
            0 if self.frames >= 2 => self.exercise_edits(),
            1 if self.frames >= 4 => {
                if let Some(window) = &self.window {
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(1100, 700));
                }
                eprintln!("SMOKE EXERCISE: resize requested");
                Ok(())
            }
            2 if self.frames >= 6 => {
                if !self.smoke_resize_seen {
                    if self.frames < 120 {
                        return;
                    }
                    self.failed = true;
                    eprintln!("SMOKE EXERCISE FAILED: no resize event observed");
                    event_loop.exit();
                    return;
                }
                self.suspended(event_loop);
                self.resumed(event_loop);
                if self.renderer.is_some() {
                    eprintln!("SMOKE EXERCISE: host renderer/window recreated");
                    Ok(())
                } else {
                    Err("host renderer recreation failed".into())
                }
            }
            _ => return,
        };
        match result {
            Ok(()) => self.smoke_stage += 1,
            Err(error) => {
                eprintln!("SMOKE EXERCISE FAILED: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }
    fn exercise_edits(&mut self) -> Result<(), String> {
        let top = (-8..32)
            .rev()
            .find(|y| self.world.get([0, *y, 0]) != 0)
            .ok_or("fixture has no terrain at origin")?;
        let target = [0, top + 1, 0];
        let previous_camera = std::mem::take(&mut self.camera);
        self.camera.position = glam::Vec3::new(0.5, top as f32 + 5.5, 0.5);
        self.camera.yaw = 0.;
        self.camera.pitch = -std::f32::consts::FRAC_PI_2;
        self.action(Action::Place);
        let placed = self.world.get(target) == 4 && !self.dirty;
        let saved = World::load(&self.save_path).map_err(|e| e.to_string());
        self.action(Action::Remove);
        self.camera = previous_camera;
        if !placed || saved?.get(target) != 4 {
            return Err("place/autosave/reload assertion failed".into());
        }
        if self.world.get(target) != 0
            || self.dirty
            || World::load(&self.save_path)
                .map_err(|e| e.to_string())?
                .get(target)
                != 0
        {
            return Err("remove/autosave/reload assertion failed".into());
        }
        eprintln!(
            "SMOKE EXERCISE: aimed place/remove and save/reload passed; revision {}",
            self.world.revision()
        );
        Ok(())
    }
    fn point(&self, x: f64, y: f64) -> Vec2 {
        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1000, 600));
        Vec2::new(
            x as f32 / size.width.max(1) as f32 * 1000.,
            y as f32 / size.height.max(1) as f32 * 600.,
        )
    }
    fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.89, 0.95, 0.97, 1.];
        let muted = [0.57, 0.72, 0.77, 1.];
        let accent = [0.52, 0.94, 0.72, 1.];
        let panel = [0.025, 0.055, 0.075, 0.87];
        hud.rect([16., 16., 687., 131.], panel);
        hud.text(30., 30., "MATTERWEAVE / VOXEL EXPLORER", 2., white);
        hud.text(
            30.,
            55.,
            "NATIVE BASELINE - FLY CAMERA - NO PHYSICS",
            1.25,
            accent,
        );
        let stats = self.world.stats();
        hud.text(
            30.,
            75.,
            &format!(
                "FRAME {:.1} MS | MAIN {:.1} MS | MESH {:.1} MS",
                self.frame_ms, self.cpu_ms, self.mesh_ms
            ),
            1.25,
            white,
        );
        hud.text(
            30.,
            93.,
            &format!(
                "{} VOXELS | {} CHUNKS | DATA {} KIB | REV {}",
                stats.solid_voxels,
                stats.chunks,
                stats.allocated_bytes / 1024,
                self.world.revision()
            ),
            1.25,
            muted,
        );
        if let Some(r) = &self.renderer {
            let size = self.window.as_ref().unwrap().inner_size();
            hud.text(
                30.,
                111.,
                &format!(
                    "{}X{} | MESH {} KIB | {}",
                    size.width,
                    size.height,
                    r.mesh_bytes / 1024,
                    r.capabilities
                )
                .chars()
                .take(65)
                .collect::<String>(),
                1.,
                muted,
            );
        }
        hud.text(
            30.,
            128.,
            &format!(
                "POS {:.0} {:.0} {:.0} | SEED {}",
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                self.world.seed()
            ),
            1.,
            muted,
        );
        let hit = self.world.raycast(
            self.camera.position.to_array(),
            self.camera.forward().to_array(),
            EDIT_RANGE,
        );
        let cross = if hit.is_some() { accent } else { white };
        hud.rect([491., 299., 18., 2.], cross);
        hud.rect([499., 291., 2., 18.], cross);
        if let Some(hit) = hit {
            hud.text(
                420.,
                324.,
                &format!(
                    "{}, {}, {} / {:.1}M",
                    hit.cell[0], hit.cell[1], hit.cell[2], hit.distance
                ),
                1.,
                white,
            );
        }
        let zone = self.controls.move_zone();
        hud.rect(zone, [0.04, 0.10, 0.13, 0.52]);
        hud.text(zone[0] + 18., zone[1] + 18., "MOVE", 1.5, white);
        hud.text(zone[0] + 18., zone[1] + 42., "DRAG", 1., muted);
        let center = [zone[0] + zone[2] / 2., zone[1] + zone[3] / 2.];
        hud.rect([center[0] - 18., center[1] - 1., 36., 2.], accent);
        hud.rect([center[0] - 1., center[1] - 18., 2., 36.], accent);
        let look_x = if self.controls.swapped { 35. } else { 780. };
        hud.text(look_x, 350., "DRAG TO LOOK", 1.25, white);
        for (rect, label, _) in self.controls.buttons() {
            hud.rect(rect, panel);
            hud.text(
                rect[0] + 9.,
                rect[1] + rect[3] / 2. - 5.,
                label,
                1.25,
                white,
            );
        }
        for (rect, label) in self
            .controls
            .elevation_zones()
            .into_iter()
            .zip(["UP", "DOWN"])
        {
            hud.rect(rect, panel);
            hud.text(rect[0] + 10., rect[1] + 18., label, 1.25, white);
        }
        hud.rect([280., 460., 440., 45.], panel);
        hud.text(
            293.,
            470.,
            &self.status.chars().take(52).collect::<String>(),
            1.,
            if self.status.contains("FAILED") {
                [1., 0.45, 0.35, 1.]
            } else {
                accent
            },
        );
        hud.text(
            293.,
            486.,
            "EDITS AUTOSAVE / SAVE FOR EXPLICIT RETRY",
            1.,
            muted,
        );
        #[cfg(not(target_os = "android"))]
        hud.text(
            265.,
            588.,
            "WASD MOVE | SPACE/SHIFT HEIGHT | RMB LOOK | LMB REMOVE | E PLACE",
            1.,
            white,
        );
        hud
    }
    // Xvfb without a window manager need not grant focus. Explicit host smoke
    // runs still request real presented frames; renderer absence/zero size remain gates.
    fn wants_frames(&self) -> bool {
        self.focused || self.smoke_frames.is_some()
    }
    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if !self.wants_frames() {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 || self.renderer.is_none() {
            return;
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms = if self.frames == 0 {
            0.
        } else {
            self.frame_ms * 0.9 + f64::from(dt) * 100.
        };
        let (motion, look) = self.controls.consume();
        self.camera.update(motion, look, dt);
        let renderer = self.renderer.as_mut().unwrap();
        if renderer.mesh_revision != Some(self.world.revision()) {
            let begin = Instant::now();
            let mesh = self.world.mesh();
            if let Err(error) = renderer.upload(&mesh) {
                log::error!("Mesh upload failed: {error}");
                eprintln!("Mesh upload failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
            self.mesh_ms = begin.elapsed().as_secs_f64() * 1000.;
        }
        let hud = self.hud();
        let matrix = self
            .camera
            .view_projection(size.width as f32 / size.height as f32);
        let position = self.camera.position.to_array();
        match self
            .renderer
            .as_mut()
            .unwrap()
            .render(matrix, position, &hud)
        {
            FrameResult::Fatal(error) => {
                log::error!("Render failed: {error}");
                eprintln!("Render failed: {error}");
                self.failed = true;
                self.renderer = None;
                event_loop.exit();
                return;
            }
            FrameResult::Presented => {
                self.frames += 1;
            }
            FrameResult::Retry => {}
            FrameResult::OutOfMemory => {
                self.failed = true;
                log::error!("GPU out of memory");
                eprintln!("GPU out of memory");
                event_loop.exit();
            }
        }
        self.cpu_ms = now.elapsed().as_secs_f64() * 1000.;
        if !self.failed
            && self.smoke_frames.is_some_and(|limit| self.frames >= limit)
            && (!self.smoke_exercise || self.smoke_stage >= 3)
        {
            eprintln!(
                "SMOKE PASS: {} presented frames; {}; world revision {}",
                self.frames,
                self.renderer.as_ref().unwrap().capabilities,
                self.world.revision()
            );
            event_loop.exit();
        }
    }
}
impl ApplicationHandler for Explorer {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        self.controls.clear();
        self.last_frame = Instant::now();
        self.focused = true;
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Matterweave | Native Voxel Explorer")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 768)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Window creation failed: {e}");
                eprintln!("Window creation failed: {e}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(renderer) => {
                log::info!("Graphics: {}", renderer.capabilities);
                eprintln!("Graphics: {}", renderer.capabilities);
                self.renderer = Some(renderer);
                self.window = Some(window);
            }
            Err(e) => {
                log::error!("Renderer initialization failed: {e}");
                eprintln!("Renderer initialization failed: {e}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.controls.clear();
        self.focused = false;
        if self.dirty {
            self.save();
        }
        self.renderer = None;
        self.window = None;
    }
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().is_none_or(|w| w.id() != window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.dirty {
                    self.save();
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if self.smoke_exercise && self.smoke_stage == 2 {
                    self.smoke_resize_seen = true;
                    eprintln!(
                        "SMOKE EXERCISE: resize event {}x{}",
                        size.width, size.height
                    );
                }
                self.controls.clear();
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(focused) => {
                if self.smoke_frames.is_some() {
                    eprintln!("SMOKE: window focus {focused}; explicit smoke keeps rendering");
                }
                self.focused = focused;
                self.last_frame = Instant::now();
                if !focused {
                    self.controls.clear();
                    if self.dirty {
                        self.save();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw(event_loop);
                if self.smoke_exercise && !self.failed {
                    self.exercise(event_loop);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        self.controls.keys.insert(key);
                        if !event.repeat {
                            match key {
                                KeyCode::Escape => event_loop.exit(),
                                KeyCode::KeyE => self.action(Action::Place),
                                KeyCode::KeyQ => self.action(Action::Remove),
                                KeyCode::F5 => self.action(Action::Save),
                                KeyCode::KeyH => self.action(Action::Swap),
                                KeyCode::KeyJ => self.action(Action::Size),
                                _ => {}
                            }
                        }
                    } else {
                        self.controls.keys.remove(&key);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = self.point(position.x, position.y);
                self.controls.mouse(p);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Right {
                    self.controls.mouse_look = state == ElementState::Pressed;
                } else if button == MouseButton::Left && state == ElementState::Pressed {
                    let action = self
                        .controls
                        .cursor
                        .and_then(|p| {
                            self.controls
                                .buttons()
                                .into_iter()
                                .find(|(r, _, _)| controls::contains(*r, p))
                                .map(|(_, _, a)| a)
                        })
                        .unwrap_or(Action::Remove);
                    self.action(action);
                }
            }
            WindowEvent::Touch(touch) => {
                let p = self.point(touch.location.x, touch.location.y);
                match touch.phase {
                    TouchPhase::Started => {
                        if let Some(action) = self.controls.start(touch.id, p) {
                            self.action(action);
                        }
                    }
                    TouchPhase::Moved => self.controls.moved(touch.id, p),
                    TouchPhase::Ended => self.controls.end(touch.id),
                    TouchPhase::Cancelled => self.controls.clear(),
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        if self.wants_frames() && self.renderer.is_some() {
            if let Some(window) = &self.window {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    window.request_redraw();
                }
            }
        }
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if self.dirty {
            self.save();
        }
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(not(target_os = "android"))]
pub fn run_desktop() {
    let mut save_path = PathBuf::from("matterweave-world.json");
    let mut smoke_frames = None;
    let mut smoke_exercise = false;
    let mut explicit_save = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--save" => {
                save_path = PathBuf::from(args.next().expect("--save requires a path"));
                explicit_save = true;
            }
            "--smoke-exercise" => smoke_exercise = true,
            "--smoke-frames" => {
                smoke_frames = Some(
                    args.next()
                        .expect("--smoke-frames requires a count")
                        .parse()
                        .expect("frame count must be an integer"),
                )
            }
            "--help" => {
                println!("Matterweave native explorer\n--save PATH (default matterweave-world.json)\n--smoke-frames N exits after N presented frames\n--smoke-exercise tests edits, save/reload, resize and host surface recreation; requires new --save PATH\nWASD move; Space/Shift height; right-drag look; left remove; E place; F5 save; H swap; J size");
                return;
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                std::process::exit(2);
            }
        }
    }
    if smoke_exercise && (!explicit_save || save_path.exists()) {
        eprintln!("--smoke-exercise requires --save with a new, disposable file path");
        std::process::exit(2);
    }
    if smoke_exercise && smoke_frames.is_none() {
        smoke_frames = Some(30);
    }
    let event_loop = EventLoop::new().expect("event loop");
    let mut explorer = Explorer::new(save_path, smoke_frames);
    explorer.smoke_exercise = smoke_exercise;
    event_loop.run_app(&mut explorer).expect("event loop run");
    if explorer.failed {
        std::process::exit(1);
    }
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default()
            .with_tag("Matterweave")
            .with_max_level(log::LevelFilter::Info),
    );
    let Some(directory) = app.internal_data_path() else {
        log::error!("Android internal storage unavailable");
        return;
    };
    let mut explorer = Explorer::new(directory.join("world.json"), None);
    let event_loop = match EventLoop::builder().with_android_app(app).build() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Event loop failed: {e}");
            return;
        }
    };
    if let Err(e) = event_loop.run_app(&mut explorer) {
        log::error!("Event loop failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    fn fixture() -> Explorer {
        let path = std::env::temp_dir().join(format!(
            "matterweave-explorer-test-{}-{}.json",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut app = Explorer::new(path, None);
        app.world = World::new(SEED);
        app.world.set([0, 0, 0], 3);
        app.camera.position = glam::Vec3::new(0.5, 0.5, 3.5);
        app.camera.yaw = std::f32::consts::PI;
        app.camera.pitch = 0.;
        app
    }
    #[test]
    fn aimed_edits_autosave_and_reload_authoritative_cells() {
        let mut app = fixture();
        app.action(Action::Place);
        assert_eq!(app.world.get([0, 0, 1]), 4);
        assert!(!app.dirty);
        let loaded = World::load(&app.save_path).unwrap();
        assert_eq!(loaded.get([0, 0, 1]), 4);
        app.action(Action::Remove);
        assert_eq!(app.world.get([0, 0, 1]), 0);
        assert_eq!(app.world.get([0, 0, 0]), 3);
        let loaded = World::load(&app.save_path).unwrap();
        assert_eq!(loaded.get([0, 0, 1]), 0);
        std::fs::remove_file(&app.save_path).unwrap();
    }
    #[test]
    fn failed_autosave_retains_dirty_state_for_retry() {
        let mut app = fixture();
        app.save_path = app.save_path.join("missing-parent.json");
        app.action(Action::Remove);
        assert_eq!(app.world.get([0, 0, 0]), 0);
        assert!(app.dirty);
        assert!(app.status.starts_with("SAVE FAILED:"));
        app.save_path = app.save_path.parent().unwrap().to_path_buf();
        app.action(Action::Save);
        assert!(!app.dirty);
        assert_eq!(World::load(&app.save_path).unwrap().get([0, 0, 0]), 0);
        std::fs::remove_file(&app.save_path).unwrap();
    }
    #[test]
    fn edits_outside_range_do_not_mutate_or_create_save() {
        let mut app = fixture();
        app.camera.position.z = 30.;
        let revision = app.world.revision();
        app.action(Action::Remove);
        assert_eq!(app.world.revision(), revision);
        assert!(!app.save_path.exists());
        assert!(!app.dirty);
    }
    #[test]
    fn corrupt_original_and_existing_recovery_are_preserved() {
        let original = fixture().save_path;
        let reserved = original.with_file_name(format!(
            "{}.recovery-1.json",
            original.file_name().unwrap().to_string_lossy()
        ));
        std::fs::write(&original, b"invalid world data").unwrap();
        std::fs::write(&reserved, b"prior recovery evidence").unwrap();
        let mut app = Explorer::new(original.clone(), None);
        assert_ne!(app.save_path, original);
        assert_ne!(app.save_path, reserved);
        assert!(app.status.starts_with("LOAD FAILED:"));
        assert!(app.recovery);
        app.action(Action::Save);
        assert_eq!(std::fs::read(&original).unwrap(), b"invalid world data");
        assert_eq!(
            std::fs::read(&reserved).unwrap(),
            b"prior recovery evidence"
        );
        assert!(World::load(&app.save_path).is_ok());
        app.world.set([0, 25, 0], 7);
        app.action(Action::Save);
        let restarted = Explorer::new(original.clone(), None);
        assert_eq!(restarted.world.get([0, 25, 0]), 7);
        assert_eq!(restarted.save_path, app.save_path);
        for path in [original, reserved, app.save_path] {
            std::fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn smoke_edits_use_real_sample_actions_and_restore_camera() {
        let mut app = fixture();
        let position = app.camera.position;
        app.exercise_edits().unwrap();
        assert_eq!(app.camera.position, position);
        assert_eq!(app.world.get([0, 1, 0]), 0);
        assert_eq!(World::load(&app.save_path).unwrap().get([0, 1, 0]), 0);
        std::fs::remove_file(&app.save_path).unwrap();
    }
    #[test]
    fn unfocused_smoke_requests_frames_but_ordinary_app_pauses() {
        let mut app = fixture();
        app.focused = false;
        assert!(!app.wants_frames());
        app.smoke_frames = Some(30);
        assert!(app.wants_frames());
        app.smoke_frames = None;
        app.focused = true;
        assert!(app.wants_frames());
    }
}
