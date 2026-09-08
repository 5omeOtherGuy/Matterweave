//! Native platform/sample orchestration. Authoritative world and GPU backend are separate crates.
mod controls;
mod detail_check;
mod dynamic_upload;
mod engine_check;
mod experience;
mod gallery;
mod metrics;
mod wetland;
mod wetland_metrics;
mod wetland_replay;
mod wetland_state;
use controls::{Action, Camera, Controls};
use glam::{Vec2, Vec3};
use matterweave_core::{AsyncWorld, World};
use matterweave_physics::{DynamicMeshCache, Physics, PhysicsSnapshot};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};

const SEED: u64 = 20260907;
const EDIT_RANGE: f32 = 12.;

/// Directory holding the world save, its gallery marker and capture requests.
fn data_directory(save_path: &Path) -> &Path {
    match save_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LightPreferences {
    shadows: bool,
    sun_index: usize,
    map_size: u32,
}
impl Default for LightPreferences {
    fn default() -> Self {
        Self {
            shadows: true,
            sun_index: 0,
            map_size: 1024,
        }
    }
}
impl LightPreferences {
    fn valid(&self) -> bool {
        self.sun_index < 3 && [1024, 2048].contains(&self.map_size)
    }
    fn settings(&self) -> LightingSettings {
        LightingSettings {
            sun: Sun {
                direction_to_sun: [[0.4, 0.85, 0.3], [-0.8, 0.35, 0.3], [0.2, 1., -0.5]]
                    [self.sun_index],
                intensity: 0.8,
            },
            shadows: self.shadows,
            shadow_map_size: self.map_size,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    version: u32,
    physics: PhysicsSnapshot,
    yaw: f32,
    pitch: f32,
    flying: bool,
    swapped: bool,
    large: bool,
    #[serde(default)]
    lighting: LightPreferences,
}
/// Save work since the last recorded row. Failed attempts consume time too, so
/// attempts and failures are counted separately and the wall time covers both.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SaveAccounting {
    wall_ms: f64,
    attempts: u32,
    failures: u32,
}
/// Per-draw work counters and stage wall times filled by `sync_render_meshes`.
/// Timings stay absent unless a capture is active.
#[derive(Clone, Copy, Debug, Default)]
struct StageCapture {
    chunk_mesh_uploads: u32,
    dynamic_mesh_builds: u32,
    dynamic_mesh_uploads: u32,
    dynamic_mesh_build_ms: Option<f64>,
    dynamic_upload_ms: Option<f64>,
}
struct Explorer {
    // Renderer must be dropped before the Android suspend callback returns.
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    world: World,
    preparation: AsyncWorld,
    lighting: LightingSettings,
    sun_index: usize,
    gpu_completions: metrics::GpuCompletionTracker,
    /// Increments on every renderer creation/recreation so per-renderer counters
    /// (GPU submission ids) cannot be joined across a recreation.
    renderer_epoch: u64,
    draw_attempts: u64,
    stage: StageCapture,
    camera: Camera,
    physics: Physics,
    dynamic_mesh: DynamicMeshCache,
    dynamic_upload: dynamic_upload::DynamicUploadState,
    flying: bool,
    jump_held: bool,
    autosave_elapsed: f32,
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
    saves: SaveAccounting,
    profile: Option<metrics::FrameLog>,
    frames: u64,
    smoke_frames: Option<u64>,
    failed: bool,
    focused: bool,
}
impl Explorer {
    fn flush_profile(&mut self) {
        if let Some(profile) = &mut self.profile {
            if let Err(error) = profile.flush() {
                log::warn!("Frame capture flush failed: {error}");
            }
        }
    }
    fn new(mut save_path: PathBuf, smoke_frames: Option<u64>) -> Self {
        let mut recovery = false;
        let (mut world, mut status) = if save_path.exists() {
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
        world.enable_streaming();
        let mut camera = Camera::default();
        world.stream_around(camera.position.to_array());
        let mut physics = Physics::new(&world);
        physics.teleport(camera.position.to_array());
        let mut flying = false;
        let mut controls = Controls::default();
        let mut light_preferences = LightPreferences::default();
        let restored = if let Some(value) = world.attachment() {
            let loaded = serde_json::from_value::<Session>(value.clone())
                .map_err(|e| e.to_string())
                .and_then(|session| {
                    if session.version != 1
                        || !session.lighting.valid()
                        || !session.yaw.is_finite()
                        || !session.pitch.is_finite()
                        || session.pitch.abs() > 1.51
                    {
                        return Err("Invalid session camera or version".into());
                    }
                    physics.restore(&session.physics)?;
                    camera.position = Vec3::from_array(physics.character_eye());
                    camera.yaw = session.yaw;
                    camera.pitch = session.pitch;
                    flying = session.flying;
                    controls.swapped = session.swapped;
                    controls.large = session.large;
                    light_preferences = session.lighting;
                    Ok(())
                });
            if let Err(error) = loaded {
                // Keep the invalid combined snapshot intact. New saves use a recovery path.
                let original = save_path.clone();
                let name = original.file_name().unwrap_or_default().to_string_lossy();
                let mut latest_valid = None;
                for sequence in 1_u64.. {
                    let candidate =
                        original.with_file_name(format!("{name}.session-recovery-{sequence}.json"));
                    if !candidate.exists() {
                        save_path = candidate;
                        break;
                    }
                    if let Ok(recovered) = World::load(&candidate) {
                        if let Some(attachment) = recovered.attachment() {
                            if let Ok(session) =
                                serde_json::from_value::<Session>(attachment.clone())
                            {
                                if session.version == 1
                                    && session.lighting.valid()
                                    && session.yaw.is_finite()
                                    && session.pitch.is_finite()
                                    && session.pitch.abs() <= 1.51
                                    && physics.restore(&session.physics).is_ok()
                                {
                                    latest_valid = Some(candidate);
                                }
                            }
                        }
                    }
                }
                if let Some(path) = latest_valid {
                    let mut recovered = Self::new(path, smoke_frames);
                    recovered.recovery = true;
                    recovered.status = "Recovered session; original retained".into();
                    return recovered;
                }
                recovery = true;
                status = "LOAD FAILED: session retained; recovery active".into();
                log::error!("Session load failed: {error}");
                false
            } else {
                true
            }
        } else {
            false
        };
        world.stream_around(camera.position.to_array());
        physics.sync_world(&world);
        if !restored {
            physics.spawn_playground(&world);
        }
        let profile = match metrics::FrameLog::requested(
            save_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
        ) {
            Ok(profile) => {
                if let Some(profile) = &profile {
                    log::info!("Frame capture: {}", profile.path.display());
                }
                profile
            }
            Err(error) => {
                log::warn!("Frame capture request failed: {error}");
                None
            }
        };
        Self {
            renderer: None,
            window: None,
            world,
            preparation: AsyncWorld::new(),
            lighting: light_preferences.settings(),
            sun_index: light_preferences.sun_index,
            gpu_completions: metrics::GpuCompletionTracker::default(),
            renderer_epoch: 0,
            draw_attempts: 0,
            stage: StageCapture::default(),
            camera,
            physics,
            dynamic_mesh: DynamicMeshCache::default(),
            dynamic_upload: dynamic_upload::DynamicUploadState::default(),
            flying,
            jump_held: false,
            autosave_elapsed: 0.,
            controls,
            save_path,
            dirty: true,
            recovery,
            smoke_exercise: false,
            smoke_stage: 0,
            smoke_resize_seen: false,
            status,
            last_frame: Instant::now(),
            frame_ms: 0.,
            cpu_ms: 0.,
            mesh_ms: 0.,
            saves: SaveAccounting::default(),
            profile,
            frames: 0,
            smoke_frames,
            failed: false,
            focused: true,
        }
    }
    /// Reads and clears the save accounting; one recorded row consumes it.
    fn take_save_accounting(&mut self) -> SaveAccounting {
        std::mem::take(&mut self.saves)
    }
    fn save(&mut self) {
        let begin = Instant::now();
        self.saves.attempts = self.saves.attempts.saturating_add(1);
        let session = Session {
            version: 1,
            physics: self.physics.snapshot(),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch.clamp(-1.5, 1.5),
            flying: self.flying,
            swapped: self.controls.swapped,
            large: self.controls.large,
            lighting: LightPreferences {
                shadows: self.lighting.shadows,
                sun_index: self.sun_index,
                map_size: self.lighting.shadow_map_size,
            },
        };
        let result = serde_json::to_value(session)
            .map_err(std::io::Error::other)
            .and_then(|value| {
                self.world
                    .save_with_attachment(&self.save_path, Some(value))
            });
        match result {
            Ok(()) => {
                self.dirty = false;
                log::info!(
                    "Saved revision {} bodies {} eye {:?}",
                    self.world.revision(),
                    self.physics.body_count(),
                    self.camera.position.to_array()
                );
                self.status = if self.recovery {
                    "Recovery saved; original retained"
                } else {
                    "World saved"
                }
                .into();
            }
            Err(e) => {
                self.saves.failures = self.saves.failures.saturating_add(1);
                self.status = format!("SAVE FAILED: {e}");
                log::error!("{}", self.status);
                eprintln!("{}", self.status);
            }
        }
        self.saves.wall_ms += begin.elapsed().as_secs_f64() * 1000.;
    }
    /// Renderer wait diagnostics exist only while a capture is active.
    fn apply_capture_diagnostics(&mut self) {
        let enabled = self.profile.is_some();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_diagnostics_enabled(enabled);
        }
    }
    fn action(&mut self, action: Action) {
        log::info!("Action {action:?}");
        let origin = self.camera.position.to_array();
        let direction = self.camera.forward().to_array();
        match action {
            Action::Shadows => {
                self.lighting.shadows = !self.lighting.shadows;
                self.status = if self.lighting.shadows {
                    "Shadows on"
                } else {
                    "Shadows off"
                }
                .into();
            }
            Action::Sun => {
                self.sun_index = (self.sun_index + 1) % 3;
                self.lighting.sun = Sun {
                    direction_to_sun: [[0.4, 0.85, 0.3], [-0.8, 0.35, 0.3], [0.2, 1.0, -0.5]]
                        [self.sun_index],
                    intensity: 0.8,
                };
                self.status =
                    ["Sun: afternoon", "Sun: low angle", "Sun: overhead"][self.sun_index].into();
            }
            Action::ShadowQuality => {
                self.lighting.shadow_map_size = if self.lighting.shadow_map_size == 1024 {
                    2048
                } else {
                    1024
                };
                self.status = format!("Shadow detail: {}", self.lighting.shadow_map_size);
            }
            Action::Flight => {
                if self.flying && !self.physics.teleport(origin) {
                    self.status = "Move into open space before walking".into();
                    return;
                }
                self.flying = !self.flying;
                self.jump_held = false;
                self.status = if self.flying {
                    "Flight enabled"
                } else {
                    "Walking enabled: jump to climb"
                }
                .into();
                self.dirty = true;
            }
            Action::Home => {
                self.preparation.reset();
                self.physics.release();
                let mut home = Camera::default();
                self.world.stream_around(home.position.to_array());
                self.physics.sync_world(&self.world);
                let mut placed = false;
                for height in (10..=70).step_by(2) {
                    home.position.y = height as f32;
                    if self.physics.teleport(home.position.to_array()) {
                        placed = true;
                        break;
                    }
                }
                if placed {
                    self.camera = home;
                    self.status = "Returned home".into();
                } else {
                    self.world.stream_around(origin);
                    self.physics.sync_world(&self.world);
                    self.status = "Home obstructed: move into open space".into();
                }
                self.dirty = true;
            }
            Action::ResetObjects => {
                let mut snapshot = self.physics.snapshot();
                snapshot.bodies.clear();
                if let Err(error) = self.physics.restore(&snapshot) {
                    self.status = format!("RESET FAILED: {error}");
                    return;
                }
                // Demo terrain may be evicted when resetting far from home.
                let mut demo_world = self.world.clone();
                demo_world.stream_around(Camera::default().position.to_array());
                self.physics.spawn_playground(&demo_world);
                self.dirty = true;
                self.save();
                self.status = "Playground reset at home".into();
            }
            Action::Grab => {
                self.status = if self
                    .physics
                    .grab(&self.world, origin, direction, EDIT_RANGE)
                {
                    "Grab changed: aim to carry, THROW to launch"
                } else {
                    "Aim at a nearby wooden physics object"
                }
                .into();
                self.dirty = true;
            }
            Action::Throw => {
                self.status = if self.physics.throw(direction) {
                    "Object thrown"
                } else {
                    "GRAB an object first"
                }
                .into();
                self.dirty = true;
            }
            Action::Break => {
                self.status = if self
                    .physics
                    .break_body(&self.world, origin, direction, EDIT_RANGE)
                {
                    "Object fractured into physical voxels"
                } else {
                    "Aim at an unbroken physics object"
                }
                .into();
                self.dirty = true;
            }
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
                    if target[0].abs() >= 256
                        || target[2].abs() >= 256
                        || !(-16..32).contains(&target[1])
                    {
                        self.status = "Edit outside sample bounds".into();
                        return;
                    }
                    let material = if action == Action::Remove { 0 } else { 4 };
                    if action == Action::Place && self.world.get(target) != 0 {
                        self.status = "Placement needs an empty adjacent cell".into();
                        return;
                    }
                    if action == Action::Place
                        && !self.flying
                        && self.physics.intersects_character_cell(target)
                    {
                        self.status = "Move away before placing here".into();
                        return;
                    }
                    if self.world.set(target, material) {
                        self.physics.sync_world(&self.world);
                        self.dirty = true;
                        self.save();
                    } else {
                        self.status = "Edit rejected: world/save limit reached".into();
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
            0 if self.frames >= 2 => self.exercise_edits().and_then(|()| self.exercise_objects()),
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
    fn exercise_objects(&mut self) -> Result<(), String> {
        let snapshot = self.physics.snapshot();
        let body = snapshot.bodies.first().ok_or("No demo objects")?;
        let previous_camera = std::mem::take(&mut self.camera);
        self.camera.position = Vec3::from_array(body.position) + Vec3::Y * 6.;
        self.camera.yaw = 0.;
        self.camera.pitch = -1.5;
        self.action(Action::Grab);
        if !self.physics.held() {
            return Err("Object grab failed".into());
        }
        self.action(Action::Throw);
        if self.physics.held() {
            return Err("Object throw failed".into());
        }
        self.action(Action::Break);
        self.camera = previous_camera;
        if self.physics.body_count() <= snapshot.bodies.len() {
            return Err("Object fracture failed".into());
        }
        self.save();
        let loaded = World::load(&self.save_path).map_err(|e| e.to_string())?;
        let session: Session =
            serde_json::from_value(loaded.attachment().ok_or("Missing session")?.clone())
                .map_err(|e| e.to_string())?;
        if session.physics.bodies.len() != self.physics.body_count() {
            return Err("Object snapshot did not round trip".into());
        }
        eprintln!(
            "SMOKE EXERCISE: grab/throw/fracture and atomic object save passed; {} bodies",
            self.physics.body_count()
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
        hud.text(30., 30., "MATTERWEAVE 0.3 / VOXEL PLAYGROUND", 2., white);
        hud.text(
            30.,
            55.,
            &format!(
                "{} | {} BODIES | {}",
                if self.flying { "FLIGHT" } else { "WALK + JUMP" },
                self.physics.body_count(),
                if self.physics.held() {
                    "CARRYING"
                } else {
                    "GRAB / THROW / BREAK"
                }
            ),
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
                    "{}X{} | MESH {} KIB | VISIBLE {}/{}",
                    size.width,
                    size.height,
                    r.mesh_bytes / 1024,
                    r.visible_chunks,
                    r.resident_chunks
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
        let object_target = self.physics.has_target(
            &self.world,
            self.camera.position.to_array(),
            self.camera.forward().to_array(),
            EDIT_RANGE,
        );
        let cross = if object_target {
            [1., 0.72, 0.38, 1.]
        } else if hit.is_some() {
            accent
        } else {
            white
        };
        hud.rect([491., 299., 18., 2.], cross);
        hud.rect([499., 291., 2., 18.], cross);
        if object_target {
            hud.text(402., 324., "OBJECT / GRAB OR BREAK", 1., cross);
        } else if let Some(hit) = hit {
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
        for (rect, label, action) in self.controls.buttons() {
            let detail;
            let label = match action {
                Action::Shadows => {
                    if self.lighting.shadows {
                        "SHADOWS ON"
                    } else {
                        "SHADOWS OFF"
                    }
                }
                Action::ShadowQuality => {
                    detail = format!("SHADOW DETAIL {}", self.lighting.shadow_map_size);
                    &detail
                }
                _ => label,
            };
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
            .zip(if self.flying {
                ["UP", "DOWN"]
            } else {
                ["JUMP", ""]
            })
        {
            if !label.is_empty() {
                hud.rect(rect, panel);
                hud.text(rect[0] + 10., rect[1] + 18., label, 1.25, white);
            }
        }
        hud.rect([280., 400., 440., 42.], panel);
        hud.text(
            293.,
            410.,
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
            426.,
            "WOODEN STRUCTURE AHEAD / HOME RETURNS TO START",
            1.,
            muted,
        );
        #[cfg(not(target_os = "android"))]
        hud.text(
            265.,
            588.,
            "WASD | SPACE JUMP | RMB LOOK | E PLACE | G GRAB | T THROW | B BREAK | F FLY",
            1.,
            white,
        );
        hud
    }
    fn sync_render_meshes(&mut self, capturing: bool) -> Result<(), String> {
        self.stage = StageCapture::default();
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let begin = Instant::now();
        let mut chunk_uploads = 0_u32;
        let keys = self.world.chunk_keys();
        renderer.retain_chunks(&keys)?;
        let mut changed = false;
        if self.preparation.available() {
            // Consume before enqueueing so completed work cannot be duplicated.
            // Bound upload count and stop after the current upload crosses 2ms.
            for _ in 0..4 {
                let Some((key, mesh)) = self.preparation.poll_mesh(&self.world) else {
                    break;
                };
                if renderer.chunk_revision(key) != Some(mesh.revision) {
                    renderer.upload_chunk(key, &mesh)?;
                    chunk_uploads += 1;
                    changed = true;
                }
                if begin.elapsed().as_secs_f64() >= 0.002 {
                    break;
                }
            }
            let mut keys = keys;
            keys.sort_by_key(|key| {
                let dx = key[0] * 16 + 8 - self.camera.position.x as i32;
                let dz = key[2] * 16 + 8 - self.camera.position.z as i32;
                dx * dx + dz * dz
            });
            let mut requested = 0;
            for key in keys {
                if renderer.chunk_revision(key) != self.world.chunk_revision(key)
                    && self.preparation.request_mesh(&self.world, key)
                {
                    requested += 1;
                    if requested >= 4 {
                        break;
                    }
                }
            }
        } else {
            // A failed background worker must not leave an empty or frozen world.
            for key in keys {
                if renderer.chunk_revision(key) != self.world.chunk_revision(key) {
                    renderer.upload_chunk(key, &self.world.mesh_chunk(key))?;
                    chunk_uploads += 1;
                    changed = true;
                }
            }
        }
        if changed {
            self.mesh_ms = begin.elapsed().as_secs_f64() * 1000.;
        }
        let build_begin = capturing.then(Instant::now);
        let rebuilt = self.dynamic_mesh.update(&self.physics);
        let build_ms = build_begin.map(|begin| {
            if rebuilt {
                begin.elapsed().as_secs_f64() * 1000.
            } else {
                0.
            }
        });
        let upload = self
            .dynamic_upload
            .needs_upload(rebuilt, self.renderer_epoch);
        let mut upload_ms = capturing.then_some(0.);
        if upload {
            let upload_begin = capturing.then(Instant::now);
            renderer.upload_dynamic(self.dynamic_mesh.mesh())?;
            upload_ms = upload_begin.map(|begin| begin.elapsed().as_secs_f64() * 1000.);
            // A failed upload must leave the new geometry pending, including
            // an empty mesh that clears formerly visible bodies.
            self.dynamic_upload.uploaded(self.renderer_epoch);
        }
        self.stage = StageCapture {
            chunk_mesh_uploads: chunk_uploads,
            dynamic_mesh_builds: u32::from(rebuilt),
            dynamic_mesh_uploads: u32::from(upload),
            dynamic_mesh_build_ms: build_ms,
            dynamic_upload_ms: upload_ms,
        };
        Ok(())
    }
    // Xvfb without a window manager need not grant focus. Explicit host smoke
    // runs still request real presented frames; renderer absence/zero size remain gates.
    fn column_ready(&self, position: Vec3) -> bool {
        // The complete window includes all simulation Y chunks. Flight above
        // that domain needs the same resident XZ footprint, including empty air.
        self.world
            .stream_contains_position([position.x, 0., position.z], 2.)
    }
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
        // Every attempt that reaches the renderer gets one identity, including
        // retries. Attempts skipped above (no window, zero size, unfocused) are
        // not draw attempts and are not recorded.
        self.draw_attempts += 1;
        let capturing = self.profile.is_some();
        // Wall clock starts first, then the CPU clock: the busy interval stays
        // inside the wall interval. The two readings are adjacent, not
        // simultaneous. `dt` still measures frame start to frame start.
        let now = Instant::now();
        let cpu_busy = metrics::CpuBusySpan::begin(capturing);
        // Upload fence waits belong to this frame only; reset before mesh sync.
        if let Some(renderer) = &mut self.renderer {
            renderer.begin_frame_diagnostics();
        }
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms = if self.frames == 0 {
            0.
        } else {
            self.frame_ms * 0.9 + f64::from(dt) * 100.
        };
        let (motion, look) = self.controls.consume();
        let previous_position = self.camera.position;
        self.camera
            .update(if self.flying { motion } else { Vec3::ZERO }, look, dt);
        let stream_begin = Instant::now();
        if self.preparation.available() {
            // Set the latest destination before polling; reversing across a boundary
            // must invalidate a result for the abandoned direction.
            self.preparation
                .request_stream(&self.world, self.camera.position.to_array());
            if self.preparation.poll_stream(&mut self.world) {
                self.physics.sync_world(&self.world);
            }
        } else if self.world.stream_around(self.camera.position.to_array()) {
            self.physics.sync_world(&self.world);
        }
        if !self.column_ready(self.camera.position) {
            self.camera.position = previous_position;
            self.status = "Preparing terrain...".into();
        }
        let stream_ms = stream_begin.elapsed().as_secs_f64() * 1000.;
        let horizontal = (Vec3::new(-self.camera.yaw.cos(), 0., self.camera.yaw.sin()) * motion.x
            + Vec3::new(self.camera.yaw.sin(), 0., self.camera.yaw.cos()) * motion.z)
            .clamp_length_max(1.)
            * 6.;
        let jumping = motion.y > 0.;
        let physics_begin = capturing.then(Instant::now);
        let fixed_steps;
        self.physics.update_grab(
            self.camera.position.to_array(),
            self.camera.forward().to_array(),
        );
        if self.flying {
            self.physics.set_flying_eye(self.camera.position.to_array());
            fixed_steps = self.physics.step_objects(dt);
        } else {
            // Collision publication above completes before stepping. The inflated
            // current/proposed footprint covers the bounded fixed-step movement.
            if self.column_ready(self.camera.position)
                && self.column_ready(self.camera.position + horizontal * 0.1)
            {
                fixed_steps =
                    self.physics
                        .step(dt, horizontal.to_array(), jumping && !self.jump_held);
            } else {
                fixed_steps = self.physics.step_objects(dt);
                self.status = "Preparing terrain...".into();
            }
            self.camera.position = Vec3::from_array(self.physics.character_eye());
        }
        let physics_ms = physics_begin.map(|begin| begin.elapsed().as_secs_f64() * 1000.);
        // Counting body states is capture-only work.
        let body_activity = capturing.then(|| self.physics.body_activity());
        if !self.flying && self.camera.position.y < -14. {
            self.action(Action::Home);
        }
        self.jump_held = jumping;
        self.dirty = true;
        self.autosave_elapsed += dt.min(0.1);
        if self.autosave_elapsed >= 15. {
            self.save();
            self.autosave_elapsed = 0.;
        }
        let mesh_begin = Instant::now();
        if let Err(error) = self.sync_render_meshes(capturing) {
            log::error!("Mesh upload failed: {error}");
            eprintln!("Mesh upload failed: {error}");
            self.failed = true;
            event_loop.exit();
            return;
        }
        let mesh_work_ms = mesh_begin.elapsed().as_secs_f64() * 1000.;
        let hud = self.hud();
        let matrix = self
            .camera
            .view_projection(size.width as f32 / size.height as f32);
        let position = self.camera.position.to_array();
        let render_begin = capturing.then(Instant::now);
        let outcome = self.renderer.as_mut().unwrap().render_with_lighting(
            matrix,
            position,
            &hud,
            &self.lighting,
        );
        let render_ms = render_begin.map(|begin| begin.elapsed().as_secs_f64() * 1000.);
        let result = match &outcome {
            FrameResult::Presented => metrics::DrawOutcome::Presented,
            FrameResult::OutOfMemory => metrics::DrawOutcome::OutOfMemory,
            // A fatal draw returns before recording; retries stay visible rows.
            FrameResult::Retry | FrameResult::Fatal(_) => metrics::DrawOutcome::Retry,
        };
        match outcome {
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
                if self.frames.is_multiple_of(300) {
                    log::info!("FRAME {} interval_ms {:.2} main_ms {:.2} mesh_ms {:.2} eye {:?} grounded {} bodies {} chunks {}",
                        self.frames, self.frame_ms, self.cpu_ms, self.mesh_ms,
                        self.camera.position.to_array(), self.physics.grounded(), self.physics.body_count(), self.world.stats().chunks);
                }
            }
            FrameResult::Retry => {}
            FrameResult::OutOfMemory => {
                self.failed = true;
                log::error!("GPU out of memory");
                eprintln!("GPU out of memory");
                event_loop.exit();
            }
        }
        // CPU busy ends immediately before the wall end, with no diagnostics
        // queries in between, so busy time cannot exceed the reported wall time.
        let cpu_busy_ms = cpu_busy.finish();
        self.cpu_ms = now.elapsed().as_secs_f64() * 1000.;
        let epoch = self.renderer_epoch;
        // Completions repeat until the next submission finishes; the epoch keeps
        // a recreated renderer's restarted ids from joining an old submission.
        let gpu = self
            .renderer
            .as_ref()
            .and_then(|r| r.gpu_timings())
            .filter(|t| self.gpu_completions.accept(epoch, t.frame_id));
        let diagnostics = self.renderer.as_ref().and_then(|r| r.draw_diagnostics());
        let shadow_casters = self
            .renderer
            .as_ref()
            .map(|r| r.shadow_caster_meshes())
            .and_then(|n| u32::try_from(n).ok());
        let stage = self.stage;
        let saves = self.take_save_accounting();
        let mut finished = false;
        if let Some(profile) = &mut self.profile {
            let row = metrics::FrameRow {
                draw_attempt_id: self.draw_attempts,
                renderer_epoch: epoch,
                presented_count: self.frames,
                result,
                submitted_gpu_frame_id: diagnostics.and_then(|d| d.submitted_frame_id),
                completed_gpu_frame_id: gpu.map(|t| t.frame_id),
                completed_gpu_renderer_epoch: gpu.map(|_| epoch),
                draw_interval_wall_ms: Some(f64::from(dt) * 1000.),
                main_wall_ms: Some(self.cpu_ms),
                main_cpu_busy_ms: cpu_busy_ms,
                stream_request_elapsed_ms: Some(stream_ms),
                physics_wall_ms: physics_ms,
                mesh_sync_wall_ms: Some(mesh_work_ms),
                mesh_sync_fence_wait_wall_ms: diagnostics.and_then(|d| d.upload_fence_wait_ms),
                dynamic_mesh_build_wall_ms: stage.dynamic_mesh_build_ms,
                dynamic_upload_wall_ms: stage.dynamic_upload_ms,
                render_wall_ms: render_ms,
                save_wall_ms: Some(saves.wall_ms),
                render_fence_wait_wall_ms: diagnostics.and_then(|d| d.render_fence_wait_ms),
                acquire_wall_ms: diagnostics.and_then(|d| d.acquire_ms),
                present_wall_ms: diagnostics.and_then(|d| d.present_ms),
                gpu_prev_render_ms: gpu.map(|t| t.render_ms),
                gpu_prev_shadow_ms: gpu.and_then(|t| t.shadow_ms),
                gpu_prev_shadows: gpu.map(|t| t.shadows),
                gpu_prev_shadow_map_size: gpu.map(|t| t.shadow_map_size),
                mesh_sync_fence_waits: diagnostics.and_then(|d| d.upload_fence_waits),
                physics_fixed_steps: u32::try_from(fixed_steps).ok(),
                voxel_bodies_total: body_activity.and_then(|a| u32::try_from(a.total).ok()),
                voxel_bodies_active: body_activity.and_then(|a| u32::try_from(a.active).ok()),
                voxel_bodies_sleeping: body_activity.and_then(|a| u32::try_from(a.sleeping).ok()),
                voxel_bodies_not_simulated: body_activity
                    .and_then(|a| u32::try_from(a.not_simulated).ok()),
                chunk_mesh_uploads: Some(stage.chunk_mesh_uploads),
                dynamic_mesh_builds: Some(stage.dynamic_mesh_builds),
                dynamic_mesh_uploads: Some(stage.dynamic_mesh_uploads),
                save_attempts: Some(saves.attempts),
                save_failures: Some(saves.failures),
                // Shadow work counts only for an attempt that submitted; a retry
                // must not inherit the previous pass's caster count.
                shadow_caster_meshes: metrics::shadow_casters_for_attempt(
                    diagnostics.and_then(|d| d.submitted_frame_id),
                    shadow_casters,
                ),
            };
            match profile.record(&row) {
                Ok(true) => {}
                Ok(false) => {
                    log::info!("Frame capture complete: {}", profile.path.display());
                    self.profile = None;
                    finished = true;
                }
                Err(error) => {
                    log::warn!("Frame capture failed: {error}");
                    self.profile = None;
                    finished = true;
                }
            }
        }
        if finished {
            self.apply_capture_diagnostics();
        }
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
        log::info!("Lifecycle resumed");
        self.controls.clear();
        self.jump_held = false;
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
                // A new renderer restarts its GPU submission counter.
                self.renderer_epoch += 1;
                self.gpu_completions = metrics::GpuCompletionTracker::default();
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.apply_capture_diagnostics();
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
        log::info!("Lifecycle suspended");
        self.physics.release();
        self.jump_held = false;
        self.controls.clear();
        self.focused = false;
        if self.dirty {
            self.save();
        }
        self.flush_profile();
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
                    self.physics.release();
                    self.jump_held = false;
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
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::BrowserBack)
                {
                    event_loop.exit();
                    return;
                }
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
                                KeyCode::KeyG => self.action(Action::Grab),
                                KeyCode::KeyT => self.action(Action::Throw),
                                KeyCode::KeyB => self.action(Action::Break),
                                KeyCode::KeyF => self.action(Action::Flight),
                                KeyCode::Home => self.action(Action::Home),
                                KeyCode::F6 => self.action(Action::ResetObjects),
                                KeyCode::F7 => self.action(Action::Shadows),
                                KeyCode::F8 => self.action(Action::Sun),
                                KeyCode::F9 => self.action(Action::ShadowQuality),
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
        self.flush_profile();
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(not(target_os = "android"))]
pub fn run_desktop() {
    let mut save_path = PathBuf::from("matterweave-world.json");
    let mut smoke_frames = None;
    let mut smoke_exercise = false;
    let mut gallery_exercise = false;
    let mut showcase = false;
    let mut sandbox = false;
    let mut engine_check = false;
    let mut detail_check = false;
    let mut explicit_save = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--save" => {
                save_path = PathBuf::from(args.next().expect("--save requires a path"));
                explicit_save = true;
            }
            "--showcase" => showcase = true,
            "--sandbox" => sandbox = true,
            "--engine-check" => engine_check = true,
            "--detail-check" => detail_check = true,
            "--smoke-exercise" => smoke_exercise = true,
            "--gallery-exercise" => gallery_exercise = true,
            "--smoke-frames" => {
                smoke_frames = Some(
                    args.next()
                        .expect("--smoke-frames requires a count")
                        .parse()
                        .expect("frame count must be an integer"),
                )
            }
            "--help" => {
                println!("Matterweave native explorer\n--save PATH (default matterweave-world.json)\n--smoke-frames N exits after N presented frames\n--smoke-exercise tests edits, save/reload, resize and host surface recreation; requires new --save PATH\n--gallery-exercise checks the opt-in detail gallery viewer lifecycle; requires a gallery request and never writes user data\nDetail gallery opt-in: `detail-gallery.txt` beside the save, or MATTERWEAVE_DETAIL_GALLERY; e.g. `tile source`, `parasol-underside half`\nWASD walk; Space jump; F flight; right-drag look; left remove; E place; G grab; T throw; B break; Home respawn; F5 save; H swap; J size");
                return;
            }
            _ => {
                eprintln!("Unknown argument: {arg}");
                std::process::exit(2);
            }
        }
    }
    if engine_check {
        let mut check =
            engine_check::IndirectCheck::new(save_path.with_file_name("engine-check-report.txt"));
        EventLoop::new()
            .expect("event loop")
            .run_app(&mut check)
            .expect("engine check loop");
        return;
    }
    if detail_check {
        let mut check =
            detail_check::DetailCheck::new(save_path.with_file_name("detail-check-report.txt"));
        EventLoop::new()
            .expect("event loop")
            .run_app(&mut check)
            .expect("detail check loop");
        return;
    }
    // Explicit developer opt-in is resolved before any world is loaded, so an
    // invalid request fails without touching user data.
    let directory = data_directory(&save_path).to_path_buf();
    let requested = match gallery::Request::resolve(
        &directory.join(gallery::MARKER_FILE),
        std::env::var(gallery::ENV_VAR).ok().as_deref(),
    ) {
        Ok(requested) => requested,
        Err(error) => {
            eprintln!("Detail gallery request rejected: {error}");
            eprintln!("No world was loaded, changed or saved. Fix or remove the request.");
            std::process::exit(2);
        }
    };
    if let Some(request) = requested {
        if smoke_exercise {
            eprintln!("--smoke-exercise drives gameplay edits and saves; it is not available in the detail gallery");
            std::process::exit(2);
        }
        let view = match gallery::GalleryView::build(request) {
            Ok(view) => view,
            Err(error) => {
                eprintln!("Detail gallery scene failed: {error}");
                std::process::exit(1);
            }
        };
        eprintln!(
            "DETAIL GALLERY: preset {} | {} prototypes {} instances | {} triangles | viewer only, no world/save/physics",
            request.label(),
            view.stats.prototypes,
            view.stats.instances,
            view.stats.combined_triangles
        );
        if gallery_exercise && smoke_frames.is_none() {
            smoke_frames = Some(30);
        }
        let event_loop = EventLoop::new().expect("event loop");
        let mut app = gallery::GalleryApp::new(view, &directory, smoke_frames);
        app.exercise = gallery_exercise;
        event_loop.run_app(&mut app).expect("event loop run");
        if app.failed() {
            std::process::exit(1);
        }
        return;
    }
    if gallery_exercise {
        eprintln!(
            "--gallery-exercise requires an active detail gallery request ({} or {})",
            gallery::MARKER_FILE,
            gallery::ENV_VAR
        );
        std::process::exit(2);
    }
    if smoke_exercise && (!explicit_save || save_path.exists()) {
        eprintln!("--smoke-exercise requires --save with a new, disposable file path");
        std::process::exit(2);
    }
    if smoke_exercise && smoke_frames.is_none() {
        smoke_frames = Some(30);
    }
    let event_loop = EventLoop::new().expect("event loop");
    if showcase || (!sandbox && !smoke_exercise && smoke_frames.is_none()) {
        let mut experience = experience::Experience::new(save_path, showcase, smoke_frames);
        event_loop.run_app(&mut experience).expect("event loop run");
        if experience.failed() {
            std::process::exit(1);
        }
        return;
    }
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
    // The normal Android entry always presents the world chooser. Historical
    // development-gallery marker files cannot trap upgrades in the old viewer.
    let event_loop = match EventLoop::builder().with_android_app(app).build() {
        Ok(e) => e,
        Err(e) => {
            log::error!("Event loop failed: {e}");
            return;
        }
    };
    // Explicit one-shot developer validation, consumed before loading any sample.
    let request = directory.join("engine-check.txt");
    let requested_check = std::fs::read_to_string(&request)
        .ok()
        .map(|s| s.trim().to_string());
    if matches!(requested_check.as_deref(), Some("indirect") | Some("detail")) {
        // Remove only the consumed request marker; never any world or save.
        if let Err(e) = std::fs::remove_file(&request) {
            log::error!("Engine check request: {e}");
            return;
        }
        let result = if requested_check.as_deref() == Some("detail") {
            let mut check =
                detail_check::DetailCheck::new(directory.join("detail-check-report.txt"));
            event_loop.run_app(&mut check)
        } else {
            let mut check =
                engine_check::IndirectCheck::new(directory.join("engine-check-report.txt"));
            event_loop.run_app(&mut check)
        };
        if let Err(e) = result {
            log::error!("Engine check: {e}");
        }
        return;
    }
    let mut experience = experience::Experience::new(directory.join("world.json"), false, None);
    if let Err(e) = event_loop.run_app(&mut experience) {
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
        app.physics = Physics::new(&app.world);
        app.flying = true;
        app.dirty = false;
        app.camera.position = glam::Vec3::new(0.5, 0.5, 3.5);
        app.camera.yaw = std::f32::consts::PI;
        app.camera.pitch = 0.;
        app
    }
    #[test]
    fn lighting_preferences_roundtrip_and_old_sessions_get_defaults() {
        let mut app = fixture();
        app.action(Action::Shadows);
        app.action(Action::Sun);
        app.action(Action::ShadowQuality);
        app.save();
        let loaded = Explorer::new(app.save_path.clone(), None);
        assert!(!loaded.lighting.shadows);
        assert_eq!(loaded.sun_index, 1);
        assert_eq!(loaded.lighting.shadow_map_size, 2048);
        let world = World::load(&app.save_path).unwrap();
        let mut old = world.attachment().unwrap().clone();
        old.as_object_mut().unwrap().remove("lighting");
        let session: Session = serde_json::from_value(old).unwrap();
        assert!(session.lighting.valid() && session.lighting.shadows);
        assert_eq!(session.lighting.map_size, 1024);
        std::fs::remove_file(&app.save_path).unwrap();
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
    fn save_accounting_separates_attempts_from_failures_and_resets_per_row() {
        let mut app = fixture();
        assert_eq!(app.take_save_accounting(), SaveAccounting::default());
        app.action(Action::Save);
        let good = app.take_save_accounting();
        assert_eq!(good.attempts, 1);
        assert_eq!(good.failures, 0);
        assert!(good.wall_ms > 0., "a completed save costs measured work");
        assert_eq!(
            app.take_save_accounting(),
            SaveAccounting::default(),
            "a recorded row consumes the accounting"
        );
        // A failed save still consumed work and must stay distinguishable.
        let good_path = app.save_path.clone();
        app.save_path = app.save_path.join("missing-parent.json");
        app.action(Action::Save);
        app.save_path = good_path.clone();
        app.action(Action::Save);
        let mixed = app.take_save_accounting();
        assert_eq!(mixed.attempts, 2, "attempts count failures too");
        assert_eq!(mixed.failures, 1);
        assert!(mixed.wall_ms > 0.);
        std::fs::remove_file(&good_path).unwrap();
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
    #[test]
    fn physics_actions_and_combined_session_survive_restart() {
        let path = fixture().save_path;
        let mut app = Explorer::new(path.clone(), None);
        app.exercise_objects().unwrap();
        app.controls.swapped = true;
        app.flying = true;
        app.physics.set_flying_eye([70., 20., -40.]);
        app.camera.position = Vec3::from_array(app.physics.character_eye());
        app.camera.yaw = 0.75;
        app.world.stream_around(app.camera.position.to_array());
        app.save();
        let restarted = Explorer::new(path.clone(), None);
        assert!(restarted.flying && restarted.controls.swapped);
        assert_eq!(restarted.camera.position, app.camera.position);
        assert_eq!(restarted.camera.yaw, 0.75);
        assert_eq!(restarted.physics.snapshot(), app.physics.snapshot());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_session_is_retained_and_latest_recovery_reopens() {
        let path = fixture().save_path;
        World::generate(SEED)
            .save_with_attachment(&path, Some(serde_json::json!({"invalid": true})))
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let mut app = Explorer::new(path.clone(), None);
        assert!(app.recovery);
        assert_ne!(app.save_path, path);
        app.world.set([0, 25, 0], 7);
        app.save();
        let restarted = Explorer::new(path.clone(), None);
        assert_eq!(restarted.save_path, app.save_path);
        assert_eq!(restarted.world.get([0, 25, 0]), 7);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(app.save_path).unwrap();
    }
    #[test]
    fn without_a_request_the_normal_application_path_is_unchanged() {
        let mut app = fixture();
        let directory = data_directory(&app.save_path).to_path_buf();
        assert!(directory.is_dir());
        // No marker beside the save and no host environment request.
        let marker = directory.join(gallery::MARKER_FILE);
        assert!(!marker.exists(), "test directory must have no marker");
        assert_eq!(gallery::Request::resolve(&marker, None).unwrap(), None);
        assert_eq!(gallery::Request::resolve(&marker, Some("")).unwrap(), None);
        // Normal gameplay still edits, autosaves and reloads authoritative cells.
        app.action(Action::Place);
        assert_eq!(app.world.get([0, 0, 1]), 4);
        assert_eq!(World::load(&app.save_path).unwrap().get([0, 0, 1]), 4);
        std::fs::remove_file(&app.save_path).unwrap();
    }
    #[test]
    fn an_invalid_gallery_request_is_rejected_before_any_world_is_touched() {
        let directory = std::env::temp_dir().join(format!(
            "matterweave-explorer-gallery-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let save = directory.join("world.json");
        std::fs::write(&save, b"sentinel user data").unwrap();
        let marker = directory.join(gallery::MARKER_FILE);
        std::fs::write(&marker, "not-a-preset").unwrap();
        let error = gallery::Request::resolve(&marker, None).unwrap_err();
        assert!(error.to_string().contains("invalid detail gallery request"));
        assert_eq!(std::fs::read(&save).unwrap(), b"sentinel user data");
        let mut entries: Vec<String> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        entries.sort();
        assert_eq!(entries, vec![gallery::MARKER_FILE, "world.json"]);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn data_directory_falls_back_to_the_working_directory() {
        assert_eq!(
            data_directory(Path::new("matterweave-world.json")),
            Path::new(".")
        );
        assert_eq!(
            data_directory(Path::new("/tmp/saves/world.json")),
            Path::new("/tmp/saves")
        );
    }
    #[test]
    fn home_finds_clear_spawn_and_blocked_flight_switch_stays_in_flight() {
        let mut app = fixture();
        let home = Camera::default();
        app.world
            .set(home.position.floor().to_array().map(|v| v as i32), 3);
        app.physics.sync_world(&app.world);
        app.camera.position = home.position;
        app.action(Action::Flight);
        assert!(app.flying);
        app.action(Action::Home);
        assert_eq!(app.camera.position.x, home.position.x);
        assert!(app.camera.position.y > home.position.y);
        assert_eq!(app.physics.character_eye(), app.camera.position.to_array());
    }
}
