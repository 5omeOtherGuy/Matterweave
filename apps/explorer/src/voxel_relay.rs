//! Voxel Relay puzzle game sample.
//! Demonstrates engine reuse with an orthographic camera, authoritative voxel chamber,
//! real rigid body physics crate pushing, door reactions, and player voxel edits.

use glam::{Mat4, Vec3};
use matterweave_core::{InputService, VirtualKey, World};
use matterweave_physics::{BodySnapshot, DynamicMeshCache, Physics, PhysicsSnapshot};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

pub const CHAMBER_SEED: u64 = 20260909;
pub const CRATE_DIMENSIONS: [u8; 3] = [2, 2, 2];
pub const CRATE_MATERIAL: u8 = 8;
pub const DOOR_MATERIAL: u8 = 7;
pub const OBSTACLE_MATERIAL: u8 = 2;

pub const PLAYER_SPAWN: [f32; 3] = [10.0, 2.55, 1.6];
pub const CRATE_SPAWN: [f32; 3] = [10.0, 1.55, 3.2];

pub const DOOR_CELLS: [[i32; 3]; 6] = [
    [5, 1, 10],
    [6, 1, 10],
    [7, 1, 10],
    [5, 2, 10],
    [6, 2, 10],
    [7, 2, 10],
];

pub const OBSTACLE_CELLS: [[i32; 3]; 6] = [
    [5, 1, 15],
    [6, 1, 15],
    [7, 1, 15],
    [5, 2, 15],
    [6, 2, 15],
    [7, 2, 15],
];

pub const EXIT_Z_MIN: f32 = 19.5;

/// Generates the puzzle chamber with stone floor, outer walls, dividing wall,
/// pressure plate, closed door, destructible obstacle, and exit zone.
pub fn generate_chamber(seed: u64) -> World {
    let mut world = World::new(seed);

    // Floor: X in 0..=15, Z in 0..=24, Y = 0
    for x in 0..=15 {
        for z in 0..=24 {
            let material = if (9..=11).contains(&x) && (6..=8).contains(&z) {
                7 // Pressure plate (mineral/crystal floor)
            } else if (4..=8).contains(&x) && (20..=23).contains(&z) {
                1 // Exit zone (grass/green floor)
            } else {
                3 // Stone floor
            };
            world.set([x, 0, z], material);
        }
    }

    // Outer perimeter walls: Y in 1..=3
    for y in 1..=3 {
        for x in 0..=15 {
            world.set([x, y, 0], 3);
            world.set([x, y, 24], 3);
        }
        for z in 0..=24 {
            world.set([0, y, z], 3);
            world.set([15, y, z], 3);
        }
    }

    // Dividing wall at Z = 10: X in 1..=14, Y in 1..=3 (except doorway X in 5..=7)
    for y in 1..=3 {
        for x in 1..=14 {
            if !(5..=7).contains(&x) {
                world.set([x, y, 10], 3);
            }
        }
    }

    // Closed door in doorway: X in 5..=7, Z = 10, Y in 1..=2
    for &[x, y, z] in &DOOR_CELLS {
        world.set([x, y, z], DOOR_MATERIAL);
    }

    // Destructible obstacle at Z = 15: X in 5..=7, Y in 1..=2
    for &[x, y, z] in &OBSTACLE_CELLS {
        world.set([x, y, z], OBSTACLE_MATERIAL);
    }

    world
}

/// Initializes physics with player and dynamic crate rigid body.
pub fn initial_physics(world: &World) -> Physics {
    let mut physics = Physics::new(world);
    let snapshot = PhysicsSnapshot {
        version: 1,
        eye: PLAYER_SPAWN,
        bodies: vec![BodySnapshot {
            position: CRATE_SPAWN,
            rotation: [0.0, 0.0, 0.0, 1.0],
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            dimensions: CRATE_DIMENSIONS,
            material: CRATE_MATERIAL,
        }],
    };
    physics
        .restore(&snapshot)
        .expect("valid initial physics snapshot");
    physics
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VoxelRelaySave {
    pub version: u32,
    pub physics: PhysicsSnapshot,
    pub door_open: bool,
    pub obstacle_cleared: bool,
    pub solved: bool,
    pub status: String,
}

/// Voxel Relay puzzle game application.
pub struct VoxelRelayApp {
    pub world: World,
    pub physics: Physics,
    pub dynamic_mesh: DynamicMeshCache,
    pub input: InputService,
    pub renderer: Option<Renderer>,
    pub window: Option<Arc<Window>>,
    pub save_path: PathBuf,
    pub door_open: bool,
    pub obstacle_cleared: bool,
    pub solved: bool,
    pub status: &'static str,
    pub facing: Vec3,
    pub frames: u64,
    pub frame_limit: Option<u64>,
    pub failed: bool,
    pub return_to_menu: bool,
    pub focused: bool,
    pub last_frame: Instant,
    pub recreate_renderer: bool,
}

impl VoxelRelayApp {
    pub fn new(save_path: PathBuf, frame_limit: Option<u64>) -> Self {
        let mut app = Self {
            world: generate_chamber(CHAMBER_SEED),
            physics: Physics::new(&World::new(CHAMBER_SEED)),
            dynamic_mesh: DynamicMeshCache::default(),
            input: InputService::new(),
            renderer: None,
            window: None,
            save_path,
            door_open: false,
            obstacle_cleared: false,
            solved: false,
            status: "PUSH CRATE ONTO PLATE",
            facing: Vec3::Z,
            frames: 0,
            frame_limit,
            failed: false,
            return_to_menu: false,
            focused: true,
            last_frame: Instant::now(),
            recreate_renderer: true,
        };

        app.setup_input();

        if app.save_path.exists() {
            if let Err(e) = app.load() {
                log::warn!(
                    "Could not load puzzle save from {}: {e}; using fresh chamber",
                    app.save_path.display()
                );
                app.reset();
            }
        } else {
            app.reset();
        }

        app
    }

    fn setup_input(&mut self) {
        self.input.clear();
        self.input.set_move_zone([40., 380., 180., 180.], 70.0);
        self.input.add_button_zone([760., 450., 200., 70.], 1); // ACTION
        self.input.add_button_zone([860., 20., 95., 40.], 2); // RESET
        self.input.add_button_zone([755., 20., 90., 40.], 3); // SAVE
        self.input.add_button_zone([650., 20., 90., 40.], 4); // MENU
    }

    pub fn reset(&mut self) {
        self.world = generate_chamber(CHAMBER_SEED);
        self.physics = initial_physics(&self.world);
        self.door_open = false;
        self.obstacle_cleared = false;
        self.solved = false;
        self.status = "PUSH CRATE ONTO PLATE";
        self.facing = Vec3::Z;
        self.setup_input();
        self.recreate_renderer = true;
    }

    pub fn save(&mut self) -> Result<(), String> {
        let save_data = VoxelRelaySave {
            version: 1,
            physics: self.physics.snapshot(),
            door_open: self.door_open,
            obstacle_cleared: self.obstacle_cleared,
            solved: self.solved,
            status: self.status.to_string(),
        };
        let value = serde_json::to_value(&save_data).map_err(|e| e.to_string())?;
        self.world
            .save_with_attachment(&self.save_path, Some(value))
            .map_err(|e| e.to_string())
    }

    pub fn load(&mut self) -> Result<(), String> {
        let world = World::load(&self.save_path).map_err(|e| e.to_string())?;
        let attachment = world.attachment().ok_or("missing puzzle save attachment")?;
        let save_data: VoxelRelaySave =
            serde_json::from_value(attachment.clone()).map_err(|e| e.to_string())?;
        if save_data.version != 1 {
            return Err("incompatible save version".into());
        }
        self.world = world;
        self.physics.sync_world(&self.world);
        self.physics.restore(&save_data.physics)?;
        self.door_open = save_data.door_open;
        self.obstacle_cleared = save_data.obstacle_cleared;
        self.solved = save_data.solved;
        self.status = match () {
            _ if self.solved => "PUZZLE SOLVED",
            _ if self.obstacle_cleared => "OBSTACLE CLEARED",
            _ if self.door_open => "DOOR OPEN",
            _ => "PUSH CRATE ONTO PLATE",
        };
        self.recreate_renderer = true;
        Ok(())
    }

    /// Whether the crate is currently positioned on the pressure plate.
    pub fn is_crate_on_plate(&self) -> bool {
        let snapshot = self.physics.snapshot();
        if let Some(crate_body) = snapshot
            .bodies
            .iter()
            .find(|b| b.dimensions == CRATE_DIMENSIONS)
        {
            let p = crate_body.position;
            p[0] >= 8.5 && p[0] <= 11.5 && p[2] >= 5.5 && p[2] <= 8.5
        } else {
            false
        }
    }

    /// Attempts to remove the destructible obstacle voxels if facing it within range.
    pub fn try_remove_obstacle(&mut self) -> bool {
        if self.obstacle_cleared {
            return false;
        }
        let eye = self.physics.character_eye();
        let dx = eye[0] - 6.0;
        let dz = eye[2] - 15.0;
        let dist = (dx * dx + dz * dz).sqrt();
        let facing_forward = self.facing.z >= -0.2;
        let in_corridor = eye[2] >= 11.5 && eye[2] <= 15.5 && eye[0] >= 3.5 && eye[0] <= 8.5;

        if (dist <= 3.5 && facing_forward) || in_corridor {
            self.obstacle_cleared = true;
            for &[x, y, z] in &OBSTACLE_CELLS {
                self.world.set([x, y, z], 0);
            }
            self.physics.sync_world(&self.world);
            self.status = "OBSTACLE CLEARED";
            return true;
        }
        false
    }

    /// Updates puzzle logic, character simulation, and physics step.
    pub fn update(&mut self, dt: f32) {
        let motion = self.input.consume_motion();
        if motion[0].hypot(motion[2]) > 0.05 {
            self.facing = Vec3::new(motion[0], 0.0, motion[2]).normalize();
        }
        let speed = 4.5;
        let horizontal_velocity = [motion[0] * speed, 0.0, motion[2] * speed];
        self.physics.step(dt, horizontal_velocity, false);

        // Process button actions
        if self.input.take_action(1) {
            self.try_remove_obstacle();
        }
        if self.input.take_action(2) {
            self.reset();
        }
        if self.input.take_action(3) {
            let _ = self.save();
        }
        if self.input.take_action(4) {
            self.return_to_menu = true;
        }

        // Pressure plate check
        let on_plate = self.is_crate_on_plate();
        if on_plate && !self.door_open {
            self.door_open = true;
            for &[x, y, z] in &DOOR_CELLS {
                self.world.set([x, y, z], 0);
            }
            self.physics.sync_world(&self.world);
        }

        // Exit zone check
        let eye = self.physics.character_eye();
        if eye[2] >= EXIT_Z_MIN && eye[0] >= 3.5 && eye[0] <= 8.5 {
            self.solved = true;
        }

        // Status update
        self.status = match () {
            _ if self.solved => "PUZZLE SOLVED",
            _ if self.obstacle_cleared => "OBSTACLE CLEARED",
            _ if self.door_open => "DOOR OPEN",
            _ => "PUSH CRATE ONTO PLATE",
        };
    }

    /// High-angle orthographic view-projection matrix framing the entire chamber.
    pub fn camera_view_proj(&self, aspect: f32) -> [[f32; 4]; 4] {
        let view_height = 24.0;
        let view_width = view_height * aspect.max(0.01);
        let proj = Mat4::orthographic_rh(
            -view_width / 2.0,
            view_width / 2.0,
            -view_height / 2.0,
            view_height / 2.0,
            0.1,
            100.0,
        );
        let eye = Vec3::new(7.5, 22.0, -2.0);
        let target = Vec3::new(7.5, 1.0, 12.0);
        let view = Mat4::look_at_rh(eye, target, Vec3::Y);
        (proj * view).to_cols_array_2d()
    }

    /// Synchronously uploads chunk meshes and dynamic physics meshes to the renderer.
    pub fn sync_render_meshes(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let keys = self.world.chunk_keys();
        renderer.retain_chunks(&keys)?;
        for key in keys {
            if renderer.chunk_revision(key) != self.world.chunk_revision(key) {
                renderer.upload_chunk(key, &self.world.mesh_chunk(key))?;
            }
        }
        let rebuilt = self.dynamic_mesh.update(&self.physics);
        if rebuilt || self.recreate_renderer {
            renderer.upload_dynamic(self.dynamic_mesh.mesh())?;
            self.recreate_renderer = false;
        }
        Ok(())
    }

    /// Builds HUD elements: title, status text, on-screen touch joystick, and buttons.
    pub fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.89, 0.95, 0.97, 1.];
        let muted = [0.57, 0.72, 0.77, 1.];
        let accent = [0.52, 0.94, 0.72, 1.];
        let gold = [1.0, 0.85, 0.3, 1.0];
        let panel = [0.025, 0.055, 0.075, 0.87];

        // Status Panel
        hud.rect([16., 16., 610., 82.], panel);
        hud.text(30., 28., "MATTERWEAVE / VOXEL RELAY", 1.8, gold);
        let status_color = if self.solved {
            accent
        } else if self.obstacle_cleared || self.door_open {
            gold
        } else {
            white
        };
        hud.text(30., 55., self.status, 1.4, status_color);

        let hint = match () {
            _ if self.solved => "CHAMBER COMPLETED! PRESS RESET TO REPLAY.",
            _ if self.obstacle_cleared => "PATH CLEAR! PROCEED TO THE EXIT ZONE.",
            _ if self.door_open => "DOOR UNLOCKED! ADVANCE AND CLEAR OBSTACLE.",
            _ => "PUSH THE WOODEN CRATE ONTO THE CRYSTAL PLATE.",
        };
        hud.text(30., 77., hint, 1.0, muted);

        // Top action buttons
        hud.rect([650., 20., 90., 40.], panel);
        hud.text(670., 32., "MENU", 1.25, white);

        hud.rect([755., 20., 90., 40.], panel);
        hud.text(778., 32., "SAVE", 1.25, white);

        hud.rect([860., 20., 95., 40.], panel);
        hud.text(880., 32., "RESET", 1.25, white);

        // Bottom left virtual joystick
        let stick_rect = [40., 380., 180., 180.];
        hud.rect(stick_rect, [0.04, 0.10, 0.13, 0.52]);
        hud.text(56., 396., "MOVE", 1.4, white);
        hud.text(56., 418., "DRAG STICK", 1.0, muted);

        if let Some((origin, current)) = self.input.joystick_state() {
            let stick_x =
                current[0].clamp(stick_rect[0] + 20., stick_rect[0] + stick_rect[2] - 20.);
            let stick_y =
                current[1].clamp(stick_rect[1] + 20., stick_rect[1] + stick_rect[3] - 20.);
            hud.rect(
                [origin[0] - 12., origin[1] - 12., 24., 24.],
                [0.3, 0.5, 0.6, 0.5],
            );
            hud.rect([stick_x - 16., stick_y - 16., 32., 32.], accent);
        } else {
            let cx = stick_rect[0] + stick_rect[2] / 2.;
            let cy = stick_rect[1] + stick_rect[3] / 2.;
            hud.rect([cx - 18., cy - 1., 36., 2.], accent);
            hud.rect([cx - 1., cy - 18., 2., 36.], accent);
        }

        // Bottom right ACTION button
        let action_rect = [760., 450., 200., 70.];
        hud.rect(action_rect, [0.20, 0.38, 0.32, 0.88]);
        hud.text(788., 474., "ACTION / CLEAR", 1.4, white);

        hud
    }

    fn point(&self, x: f64, y: f64) -> [f32; 2] {
        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1000, 600));
        [
            x as f32 / size.width.max(1) as f32 * 1000.,
            y as f32 / size.height.max(1) as f32 * 600.,
        ]
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .clamp(0.001, 0.05);
        self.last_frame = now;

        self.update(dt);

        if let Err(e) = self.sync_render_meshes() {
            log::error!("Mesh sync failed: {e}");
            self.failed = true;
            event_loop.exit();
            return;
        }

        let Some(window) = &self.window else {
            return;
        };
        let size = window.inner_size();
        let aspect = size.width as f32 / size.height.max(1) as f32;
        let view_proj = self.camera_view_proj(aspect);
        let eye = Vec3::new(7.5, 22.0, -2.0);
        let hud = self.hud();
        let lighting = LightingSettings {
            sun: Sun {
                direction_to_sun: [0.3, 0.9, 0.3],
                intensity: 0.85,
            },
            shadows: true,
            shadow_map_size: 1024,
        };

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match renderer.render_with_lighting(view_proj, eye.to_array(), &hud, &lighting) {
            FrameResult::Presented | FrameResult::Retry => {}
            FrameResult::OutOfMemory => {
                log::warn!("Out of memory during puzzle render");
            }
            FrameResult::Fatal(err) => {
                log::error!("Vulkan render fatal error: {err}");
                self.failed = true;
                event_loop.exit();
            }
        }

        self.frames += 1;
        if let Some(limit) = self.frame_limit {
            if self.frames >= limit {
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler for VoxelRelayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        self.input.clear();
        self.last_frame = Instant::now();
        self.focused = true;
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Matterweave | Voxel Relay Puzzle")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 768)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Window creation failed: {e}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };

        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(renderer) => {
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.recreate_renderer = true;
            }
            Err(e) => {
                log::error!("Renderer initialization failed: {e}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.physics.release();
        self.input.clear();
        self.focused = false;
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
                event_loop.exit();
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                if !f {
                    self.input.clear();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::KeyW | KeyCode::ArrowUp) => {
                        if pressed {
                            self.input.key_down(VirtualKey::W);
                        } else {
                            self.input.key_up(VirtualKey::W);
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyS | KeyCode::ArrowDown) => {
                        if pressed {
                            self.input.key_down(VirtualKey::S);
                        } else {
                            self.input.key_up(VirtualKey::S);
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyA | KeyCode::ArrowLeft) => {
                        if pressed {
                            self.input.key_down(VirtualKey::A);
                        } else {
                            self.input.key_up(VirtualKey::A);
                        }
                    }
                    PhysicalKey::Code(KeyCode::KeyD | KeyCode::ArrowRight) => {
                        if pressed {
                            self.input.key_down(VirtualKey::D);
                        } else {
                            self.input.key_up(VirtualKey::D);
                        }
                    }
                    PhysicalKey::Code(KeyCode::Space | KeyCode::KeyE | KeyCode::KeyF)
                        if pressed =>
                    {
                        self.try_remove_obstacle();
                    }
                    PhysicalKey::Code(KeyCode::KeyR) if pressed => {
                        self.reset();
                    }
                    PhysicalKey::Code(KeyCode::KeyP | KeyCode::F5) if pressed => {
                        let _ = self.save();
                    }
                    PhysicalKey::Code(KeyCode::Escape) if pressed => {
                        self.return_to_menu = true;
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if state == ElementState::Released {
                    self.input.pointer_up(0);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = self.point(position.x, position.y);
                self.input.pointer_move(0, p);
            }
            WindowEvent::Touch(touch) => {
                let p = self.point(touch.location.x, touch.location.y);
                match touch.phase {
                    TouchPhase::Started => {
                        if let Some(action) = self.input.pointer_down(touch.id, p) {
                            match action {
                                1 => {
                                    self.try_remove_obstacle();
                                }
                                2 => {
                                    self.reset();
                                }
                                3 => {
                                    let _ = self.save();
                                }
                                4 => {
                                    self.return_to_menu = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    TouchPhase::Moved => self.input.pointer_move(touch.id, p),
                    TouchPhase::Ended => self.input.pointer_up(touch.id),
                    TouchPhase::Cancelled => self.input.clear(),
                }
            }
            WindowEvent::RedrawRequested => {
                self.draw(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chamber_generation_verifies_walls_plate_door_and_obstacle() {
        let world = generate_chamber(CHAMBER_SEED);

        // Floor
        assert_eq!(world.get([1, 0, 1]), 3);

        // Pressure plate region
        assert_eq!(world.get([10, 0, 7]), 7);

        // Dividing wall
        assert_eq!(world.get([2, 1, 10]), 3);

        // Closed door
        for cell in &DOOR_CELLS {
            assert_eq!(world.get(*cell), DOOR_MATERIAL);
        }

        // Destructible obstacle
        for cell in &OBSTACLE_CELLS {
            assert_eq!(world.get(*cell), OBSTACLE_MATERIAL);
        }

        // Exit zone floor
        assert_eq!(world.get([6, 0, 21]), 1);
    }

    #[test]
    fn orthographic_camera_projection_matrix_is_valid() {
        let app = VoxelRelayApp::new(PathBuf::from("test_relay.json"), None);
        let view_proj = app.camera_view_proj(16.0 / 9.0);
        let mat = Mat4::from_cols_array_2d(&view_proj);
        assert!(mat.is_finite());
        assert!(mat.determinant().abs() > 1e-6);

        // Test that chamber center transforms into valid normalized clip coordinates
        let center = Vec3::new(7.5, 1.0, 12.0);
        let clip = mat.project_point3(center);
        assert!(clip.x.abs() <= 1.0);
        assert!(clip.y.abs() <= 1.0);
    }

    #[test]
    fn physics_crate_spawns_and_pressure_plate_triggers_door_reaction() {
        let mut app = VoxelRelayApp::new(PathBuf::from("test_relay.json"), None);
        assert_eq!(app.status, "PUSH CRATE ONTO PLATE");
        assert!(!app.is_crate_on_plate());
        assert!(!app.door_open);

        // Teleport crate directly onto the plate
        let snapshot = PhysicsSnapshot {
            version: 1,
            eye: PLAYER_SPAWN,
            bodies: vec![BodySnapshot {
                position: [10.0, 1.55, 7.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [0.0; 3],
                angular_velocity: [0.0; 3],
                dimensions: CRATE_DIMENSIONS,
                material: CRATE_MATERIAL,
            }],
        };
        app.physics.restore(&snapshot).unwrap();

        assert!(app.is_crate_on_plate());

        // Step puzzle update
        app.update(0.016);

        assert!(app.door_open);
        assert_eq!(app.status, "DOOR OPEN");

        // Authoritative door voxels must be cleared in World
        for cell in &DOOR_CELLS {
            assert_eq!(app.world.get(*cell), 0);
        }
    }

    #[test]
    fn character_collision_impulses_push_crate_physically() {
        let mut app = VoxelRelayApp::new(PathBuf::from("test_relay.json"), None);
        let crate_pos_before = app.physics.snapshot().bodies[0].position;

        // Player starts right behind crate and steps forward physically into it
        for _ in 0..60 {
            app.physics.step(0.016, [0.0, 0.0, 4.0], false);
        }

        let crate_pos_after = app.physics.snapshot().bodies[0].position;
        assert!(
            crate_pos_after[2] > crate_pos_before[2] + 0.2,
            "Crate was not pushed: before {:?}, after {:?}",
            crate_pos_before,
            crate_pos_after
        );
    }

    fn walk_towards(app: &mut VoxelRelayApp, target_x: f32, target_z: f32, max_steps: usize) {
        for _ in 0..max_steps {
            let eye = app.physics.character_eye();
            let dx = target_x - eye[0];
            let dz = target_z - eye[2];
            let dist = dx.hypot(dz);
            if dist < 0.25 {
                break;
            }
            let speed = 4.0;
            let vx = (dx / dist) * speed;
            let vz = (dz / dist) * speed;
            app.physics.step(0.016, [vx, 0.0, vz], false);
            app.update(0.016);
        }
    }

    #[test]
    fn obstacle_removal_and_exit_solve_flow() {
        let mut app = VoxelRelayApp::new(PathBuf::from("test_relay.json"), None);

        // Open door first
        app.door_open = true;
        for cell in &DOOR_CELLS {
            app.world.set(*cell, 0);
        }
        app.physics.sync_world(&app.world);

        // Walk to doorway and through to obstacle
        walk_towards(&mut app, 6.0, 9.0, 200);
        walk_towards(&mut app, 6.0, 13.8, 150);
        app.facing = Vec3::Z;

        assert!(app.try_remove_obstacle());
        assert!(app.obstacle_cleared);
        for cell in &OBSTACLE_CELLS {
            assert_eq!(app.world.get(*cell), 0);
        }

        // Walk through cleared obstacle to exit zone
        walk_towards(&mut app, 6.0, 21.0, 150);

        assert!(app.solved);
        assert_eq!(app.status, "PUZZLE SOLVED");
    }

    #[test]
    fn full_puzzle_solve_via_physics_push_and_door_reaction() {
        let mut app = VoxelRelayApp::new(PathBuf::from("test_full_relay.json"), None);
        assert_eq!(app.status, "PUSH CRATE ONTO PLATE");
        assert!(!app.door_open);

        // Phase 1: Push crate forward along +Z onto the pressure plate at Z = 7.0
        for _ in 0..150 {
            app.physics.step(0.016, [0.0, 0.0, 4.0], false);
            app.update(0.016);
            if app.door_open {
                break;
            }
        }

        assert!(
            app.door_open,
            "Crate was pushed onto plate, door must be open"
        );
        assert_eq!(app.status, "DOOR OPEN");
        for cell in &DOOR_CELLS {
            assert_eq!(app.world.get(*cell), 0);
        }

        // Phase 2: Walk to open doorway and through to obstacle at Z = 15.0
        walk_towards(&mut app, 6.0, 9.0, 200);
        walk_towards(&mut app, 6.0, 13.8, 150);
        app.facing = Vec3::Z;

        // Phase 3: Trigger action to remove obstacle
        assert!(app.try_remove_obstacle());
        assert!(app.obstacle_cleared);
        assert_eq!(app.status, "OBSTACLE CLEARED");

        // Phase 4: Walk into exit zone
        walk_towards(&mut app, 6.0, 21.0, 150);
        assert!(app.solved);
        assert_eq!(app.status, "PUZZLE SOLVED");
    }

    #[test]
    fn save_and_reload_atomic_persistence() {
        let temp_save = std::env::temp_dir().join(format!(
            "voxel-relay-test-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let mut app = VoxelRelayApp::new(temp_save.clone(), None);

        // Push crate to plate and open door
        let snapshot = PhysicsSnapshot {
            version: 1,
            eye: [6.0, 2.55, 12.0],
            bodies: vec![BodySnapshot {
                position: [10.0, 1.55, 7.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                velocity: [0.0; 3],
                angular_velocity: [0.0; 3],
                dimensions: CRATE_DIMENSIONS,
                material: CRATE_MATERIAL,
            }],
        };
        app.physics.restore(&snapshot).unwrap();
        app.update(0.016);
        assert!(app.door_open);

        app.save().expect("save successful");
        assert!(temp_save.exists());

        // Reload into a fresh app instance
        let loaded_app = VoxelRelayApp::new(temp_save.clone(), None);
        assert!(loaded_app.door_open);
        assert_eq!(loaded_app.status, "DOOR OPEN");
        for cell in &DOOR_CELLS {
            assert_eq!(loaded_app.world.get(*cell), 0);
        }
        let reloaded_crate = &loaded_app.physics.snapshot().bodies[0];
        assert!((reloaded_crate.position[0] - 10.0).abs() < 0.1);
        assert!((reloaded_crate.position[2] - 7.0).abs() < 0.1);

        let _ = std::fs::remove_file(temp_save);
    }

    #[derive(Debug, Clone, PartialEq)]
    struct ReplayOutcome {
        door_open: bool,
        obstacle_cleared: bool,
        solved: bool,
        world_revision: u64,
        final_eye: [f32; 3],
        final_crate_pos: [f32; 3],
    }

    fn execute_deterministic_replay(save_path: PathBuf) -> (VoxelRelayApp, ReplayOutcome) {
        if save_path.exists() {
            let _ = std::fs::remove_file(&save_path);
        }
        let mut app = VoxelRelayApp::new(save_path.clone(), None);

        // Verify initial state
        assert_eq!(app.status, "PUSH CRATE ONTO PLATE");
        assert!(!app.door_open);
        assert!(!app.obstacle_cleared);
        assert!(!app.solved);
        for cell in &DOOR_CELLS {
            assert_eq!(app.world.get(*cell), DOOR_MATERIAL);
        }
        for cell in &OBSTACLE_CELLS {
            assert_eq!(app.world.get(*cell), OBSTACLE_MATERIAL);
        }

        // Phase 1: Closed door blocks passage
        // Walk player towards doorway at Z=10
        // Sidestep crate first to X=6.0, Z=1.6
        walk_towards(&mut app, 6.0, 1.6, 150);
        // Walk forward towards doorway at Z=10.5
        walk_towards(&mut app, 6.0, 10.5, 200);

        let blocked_eye = app.physics.character_eye();
        assert!(
            blocked_eye[2] < 9.75 && blocked_eye[2] > 9.5,
            "Closed door must block passage: character eye Z was {}, expected between 9.5 and 9.75",
            blocked_eye[2]
        );
        for cell in &DOOR_CELLS {
            assert_eq!(
                app.world.get(*cell),
                DOOR_MATERIAL,
                "Door voxels in World must remain solid"
            );
        }
        assert!(!app.door_open);

        // Phase 2: Move crate through real physics onto pressure plate at Z=7.0
        // Walk back to behind the crate at [10.0, 1.6]
        walk_towards(&mut app, 6.0, 1.6, 200);
        walk_towards(&mut app, 10.0, 1.6, 150);

        // Step forward into crate, collision impulses push crate onto plate
        for _ in 0..150 {
            app.physics.step(0.016, [0.0, 0.0, 4.0], false);
            app.update(0.016);
            if app.door_open {
                break;
            }
        }

        assert!(app.is_crate_on_plate(), "Crate must be pushed onto plate");
        let crate_pos = app.physics.snapshot().bodies[0].position;
        assert!(
            crate_pos[0] >= 8.5
                && crate_pos[0] <= 11.5
                && crate_pos[2] >= 5.5
                && crate_pos[2] <= 8.5,
            "Crate position {:?} must be on pressure plate",
            crate_pos
        );

        // Phase 3: Authoritative door opens
        assert!(app.door_open, "Plate triggers door open");
        for cell in &DOOR_CELLS {
            assert_eq!(app.world.get(*cell), 0, "Door cells in World must be 0");
        }

        // Phase 4: Walk through open doorway into corridor 2
        walk_towards(&mut app, 6.0, 5.4, 150);
        walk_towards(&mut app, 6.0, 12.0, 200);
        let corridor_eye = app.physics.character_eye();
        assert!(
            corridor_eye[2] > 10.5,
            "Player must pass through doorway into corridor 2: eye Z was {}",
            corridor_eye[2]
        );

        // Phase 5: Player-triggered voxel removal via input action button (ID 1)
        walk_towards(&mut app, 6.0, 13.8, 150);
        app.facing = Vec3::Z;

        let triggered = app.input.pointer_down(1, [800.0, 480.0]);
        assert_eq!(
            triggered,
            Some(1),
            "Pointer in button zone must trigger action ID 1"
        );
        app.update(0.016);
        app.input.pointer_up(1);
        app.update(0.016);

        assert!(app.obstacle_cleared, "Obstacle must be cleared");
        for cell in &OBSTACLE_CELLS {
            assert_eq!(app.world.get(*cell), 0, "Obstacle cells in World must be 0");
        }

        // Phase 6: Reaches exit zone (Z >= 19.5)
        walk_towards(&mut app, 6.0, 21.0, 200);
        let final_eye = app.physics.character_eye();
        assert!(app.solved, "Puzzle must be solved");
        assert!(
            final_eye[2] >= EXIT_Z_MIN,
            "Player eye Z {} must reach exit zone (>= {})",
            final_eye[2],
            EXIT_Z_MIN
        );
        assert_eq!(app.status, "PUZZLE SOLVED");

        let outcome = ReplayOutcome {
            door_open: app.door_open,
            obstacle_cleared: app.obstacle_cleared,
            solved: app.solved,
            world_revision: app.world.revision(),
            final_eye,
            final_crate_pos: app.physics.snapshot().bodies[0].position,
        };

        (app, outcome)
    }

    #[test]
    fn test_deterministic_replay_flow() {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let save_path = std::env::temp_dir().join(format!("voxel_relay_deterministic_{}.json", id));
        let (_app, outcome) = execute_deterministic_replay(save_path.clone());
        let _ = std::fs::remove_file(save_path);

        assert!(outcome.door_open);
        assert!(outcome.obstacle_cleared);
        assert!(outcome.solved);
    }

    #[test]
    fn test_save_reload_intermediate_and_unrelated_invariance() {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir();
        let task_save_path = temp_dir.join(format!("voxel_relay_intermediate_{}.json", id));
        let unrelated_path = temp_dir.join(format!("unrelated_world_{}.json", id));

        if task_save_path.exists() {
            let _ = std::fs::remove_file(&task_save_path);
        }

        // 1. Pre-create an unrelated pre-existing save file with known content
        let unrelated_content = br#"{
  "world_version": 1,
  "generator": "matterweave-test",
  "seed": 987654321,
  "chunks": [
    {"key": [0, 0, 0], "revision": 42, "voxels_hash": "a1b2c3d4e5f6"}
  ],
  "authoritative_metadata": {
    "checksum": 1337,
    "user_notes": "Preserve unmodified across all other task operations"
  }
}"#;
        std::fs::write(&unrelated_path, unrelated_content).expect("write unrelated save");
        let unrelated_before = std::fs::read(&unrelated_path).expect("read unrelated save before");
        assert_eq!(unrelated_before, unrelated_content);

        // 2. Run puzzle to an intermediate state (crate on plate, door open)
        let mut app = VoxelRelayApp::new(task_save_path.clone(), None);
        assert_eq!(app.status, "PUSH CRATE ONTO PLATE");
        assert!(!app.door_open);

        // Push crate onto plate
        for _ in 0..150 {
            app.physics.step(0.016, [0.0, 0.0, 4.0], false);
            app.update(0.016);
            if app.door_open {
                break;
            }
        }

        assert!(app.door_open, "Door must be open at intermediate state");
        assert!(app.is_crate_on_plate(), "Crate must be on plate");
        assert!(!app.obstacle_cleared, "Obstacle must not yet be cleared");
        assert!(!app.solved, "Puzzle must not yet be solved");
        assert_eq!(app.status, "DOOR OPEN");

        // Move player through doorway into corridor
        walk_towards(&mut app, 6.0, 5.4, 60);
        walk_towards(&mut app, 6.0, 11.5, 100);

        let saved_eye = app.physics.character_eye();
        let saved_crate_body = app.physics.snapshot().bodies[0].clone();
        let saved_door_open = app.door_open;
        let saved_obstacle_cleared = app.obstacle_cleared;
        let saved_solved = app.solved;
        let saved_status = app.status.to_string();

        for cell in &DOOR_CELLS {
            assert_eq!(app.world.get(*cell), 0);
        }
        for cell in &OBSTACLE_CELLS {
            assert_eq!(app.world.get(*cell), OBSTACLE_MATERIAL);
        }

        // 3. Save sample to task-owned save path
        app.save().expect("save successful");
        assert!(task_save_path.exists(), "Save file must exist after save()");

        // 4. Terminate / drop app instance
        drop(app);

        // 5. Instantiate fresh app and reload from save path
        let loaded_app = VoxelRelayApp::new(task_save_path.clone(), None);

        // 6. Assert player position, crate body position/velocity, door open state,
        // world voxel edits, and puzzle status are all preserved
        let loaded_eye = loaded_app.physics.character_eye();
        for i in 0..3 {
            assert!(
                (loaded_eye[i] - saved_eye[i]).abs() < 1e-4,
                "Player eye[{}] mismatch: saved {}, loaded {}",
                i,
                saved_eye[i],
                loaded_eye[i]
            );
        }

        let loaded_crate_body = &loaded_app.physics.snapshot().bodies[0];
        for i in 0..3 {
            assert!(
                (loaded_crate_body.position[i] - saved_crate_body.position[i]).abs() < 1e-4,
                "Crate position[{}] mismatch: saved {}, loaded {}",
                i,
                saved_crate_body.position[i],
                loaded_crate_body.position[i]
            );
            assert!(
                (loaded_crate_body.velocity[i] - saved_crate_body.velocity[i]).abs() < 1e-4,
                "Crate velocity[{}] mismatch: saved {}, loaded {}",
                i,
                saved_crate_body.velocity[i],
                loaded_crate_body.velocity[i]
            );
        }

        assert_eq!(
            loaded_app.door_open, saved_door_open,
            "door_open must match"
        );
        assert_eq!(
            loaded_app.obstacle_cleared, saved_obstacle_cleared,
            "obstacle_cleared must match"
        );
        assert_eq!(loaded_app.solved, saved_solved, "solved must match");
        assert_eq!(loaded_app.status, saved_status, "status must match");

        for cell in &DOOR_CELLS {
            assert_eq!(
                loaded_app.world.get(*cell),
                0,
                "Door cell must remain 0 after reload"
            );
        }
        for cell in &OBSTACLE_CELLS {
            assert_eq!(
                loaded_app.world.get(*cell),
                OBSTACLE_MATERIAL,
                "Obstacle cell must remain untouched after reload"
            );
        }

        // 7. Verify unrelated save file remains 100% byte-identical
        let unrelated_after = std::fs::read(&unrelated_path).expect("read unrelated save after");
        assert_eq!(
            unrelated_before, unrelated_after,
            "Unrelated save file was modified! Expected 100% byte identity."
        );

        let _ = std::fs::remove_file(task_save_path);
        let _ = std::fs::remove_file(unrelated_path);
    }

    #[test]
    fn test_ten_repeated_replays_deterministic_outcome() {
        let mut outcomes = Vec::new();

        for run_idx in 0..10 {
            let temp_path = std::env::temp_dir().join(format!(
                "voxel_relay_repeat_{}_{}.json",
                std::process::id(),
                run_idx
            ));

            let (_app, outcome) = execute_deterministic_replay(temp_path.clone());
            let _ = std::fs::remove_file(temp_path);

            assert!(outcome.door_open, "Run {} door must be open", run_idx);
            assert!(
                outcome.obstacle_cleared,
                "Run {} obstacle must be cleared",
                run_idx
            );
            assert!(outcome.solved, "Run {} must be solved", run_idx);

            outcomes.push(outcome);
        }

        let baseline = &outcomes[0];
        for (i, outcome) in outcomes.iter().enumerate().skip(1) {
            assert_eq!(
                outcome.door_open, baseline.door_open,
                "Run {} door_open does not match run 0",
                i
            );
            assert_eq!(
                outcome.obstacle_cleared, baseline.obstacle_cleared,
                "Run {} obstacle_cleared does not match run 0",
                i
            );
            assert_eq!(
                outcome.solved, baseline.solved,
                "Run {} solved does not match run 0",
                i
            );
            assert_eq!(
                outcome.world_revision, baseline.world_revision,
                "Run {} world revision ({}) does not match run 0 ({})",
                i, outcome.world_revision, baseline.world_revision
            );
            for dim in 0..3 {
                assert!(
                    (outcome.final_eye[dim] - baseline.final_eye[dim]).abs() < 1e-4,
                    "Run {} final_eye[{}] ({}) does not match run 0 ({})",
                    i,
                    dim,
                    outcome.final_eye[dim],
                    baseline.final_eye[dim]
                );
                assert!(
                    (outcome.final_crate_pos[dim] - baseline.final_crate_pos[dim]).abs() < 1e-4,
                    "Run {} final_crate_pos[{}] ({}) does not match run 0 ({})",
                    i,
                    dim,
                    outcome.final_crate_pos[dim],
                    baseline.final_crate_pos[dim]
                );
            }
        }
    }

    #[test]
    fn test_input_service_comprehensive_edge_cases() {
        let mut input = InputService::new();
        let move_zone = [40., 380., 180., 180.];
        let radius = 70.0;
        input.set_move_zone(move_zone, radius);
        input.add_button_zone([760., 450., 200., 70.], 1); // ACTION
        input.add_button_zone([860., 20., 95., 40.], 2); // RESET
        input.add_button_zone([755., 20., 90., 40.], 3); // SAVE
        input.add_button_zone([650., 20., 90., 40.], 4); // MENU

        // 1. Simultaneous movement (pointer 1 in move zone) and action (pointer 2 on button)
        assert_eq!(input.pointer_down(1, [130.0, 470.0]), None);
        input.pointer_move(1, [130.0, 400.0]); // dragged forward (-Y => +Z in motion)

        assert_eq!(input.pointer_down(2, [800.0, 480.0]), Some(1));
        assert!(input.take_action(1), "Action 1 must be registered");
        assert!(!input.take_action(1), "Action 1 must be consumed");

        assert_eq!(input.pointer_down(3, [780.0, 30.0]), Some(3));
        assert!(input.take_action(3), "Action 3 must be registered");

        let motion1 = input.consume_motion();
        assert!((motion1[0] - 0.0).abs() < 1e-4);
        assert!(
            (motion1[2] - 1.0).abs() < 1e-4,
            "Pointer 1 motion[2] must be 1.0, was {}",
            motion1[2]
        );

        input.pointer_up(2);
        input.pointer_up(3);

        let motion2 = input.consume_motion();
        assert!(
            (motion2[2] - 1.0).abs() < 1e-4,
            "Motion must continue after button release"
        );

        input.pointer_up(1);
        assert_eq!(
            input.consume_motion(),
            [0.0, 0.0, 0.0],
            "consume_motion() must be [0,0,0] after pointer release"
        );

        // 2. Touch cancellation
        input.pointer_down(10, [130.0, 470.0]);
        input.pointer_move(10, [130.0, 420.0]);
        assert!(input.consume_motion()[2] > 0.5);

        input.clear();
        assert_eq!(
            input.consume_motion(),
            [0.0, 0.0, 0.0],
            "consume_motion() must return [0,0,0] after touch cancellation"
        );
        assert_eq!(input.active_pointers(), 0, "No active pointers after clear");
        assert_eq!(
            input.joystick_state(),
            None,
            "No joystick state after clear"
        );

        // 3. Focus loss with combined keyboard and multi-touch
        input.key_down(VirtualKey::W);
        input.key_down(VirtualKey::D);
        input.pointer_down(20, [130.0, 470.0]);
        input.pointer_move(20, [80.0, 470.0]);
        input.pointer_down(21, [800.0, 480.0]);
        input.add_look_delta([50.0, -30.0]);

        input.clear();

        assert_eq!(
            input.consume_motion(),
            [0.0, 0.0, 0.0],
            "consume_motion() must return [0,0,0] after focus loss"
        );
        assert_eq!(
            input.consume_look(),
            [0.0, 0.0],
            "consume_look() must return [0,0] after focus loss"
        );
        assert_eq!(
            input.active_pointers(),
            0,
            "Active pointers must be 0 after focus loss"
        );
        assert!(!input.is_key_down(VirtualKey::W), "W key must not be down");
        assert!(!input.is_key_down(VirtualKey::D), "D key must not be down");
        assert!(!input.take_action(1), "No pending actions after focus loss");

        // 4. Resume after cancellation and focus loss: ensure no stuck movement
        input.key_down(VirtualKey::S);
        assert_eq!(input.consume_motion(), [0.0, 0.0, -1.0]);
        input.key_up(VirtualKey::S);
        assert_eq!(
            input.consume_motion(),
            [0.0, 0.0, 0.0],
            "No stuck keyboard movement after release"
        );

        input.pointer_down(30, [130.0, 470.0]);
        input.pointer_move(30, [180.0, 470.0]);
        assert!(input.consume_motion()[0] > 0.5);
        input.pointer_up(30);
        assert_eq!(
            input.consume_motion(),
            [0.0, 0.0, 0.0],
            "No stuck touch movement after release"
        );
    }
}
