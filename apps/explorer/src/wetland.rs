//! Player-facing wetland experience; source, collision, derived graphics and saves
//! remain separate. Legacy sandbox files are never used by this mode.
use crate::{
    controls::{contains, Action, Camera, Controls},
    metrics,
    wetland_metrics::Capture,
    wetland_state::{self, Edit, SavedWetland},
};
use glam::{Vec2, Vec3};
use matterweave_core::{Mesh, World};
use matterweave_detail::{DetailScene, Lod, Yaw};
use matterweave_physics::{BodySnapshot, DynamicMeshCache, Physics, PhysicsSnapshot};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, StaticInstance};
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};
const SEED: u64 = matterweave_detail::SHOWCASE_SEED;
const GENERATOR: u32 = matterweave_detail::SHOWCASE_GENERATOR_VERSION;

struct Runtime {
    scene: DetailScene,
    meshes: Vec<Mesh>,
    instances: Vec<StaticInstance>,
    physics: Physics,
    empty_world: World,
    dynamic: DynamicMeshCache,
    edits: Vec<Edit>,
    save_path: PathBuf,
    spawn: [f32; 3],
    terrain: matterweave_detail::Terrain,
    clearing: [f32; 3],
    route: Vec<[f32; 3]>,
    camera: Camera,
    lighting: LightingSettings,
    dirty: bool,
    graphics_dirty: bool,
    dynamic_dirty: bool,
    counts: String,
}

fn graphics(scene: &mut DetailScene) -> Result<(Vec<Mesh>, Vec<StaticInstance>), String> {
    let ids = scene.prototype_ids();
    let mut meshes = Vec::with_capacity(ids.len());
    for id in &ids {
        let mesh = scene
            .prototype_mesh(id, Lod::Source)
            .map_err(|e| e.to_string())?;
        meshes.push(Mesh {
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
            revision: mesh.revision,
        });
    }
    let instances = scene
        .draws()
        .into_iter()
        .map(|draw| StaticInstance {
            prototype: ids
                .binary_search(&draw.prototype)
                .expect("scene prototype order"),
            translation: draw.transform.translation_m,
            yaw_quarters: match draw.transform.yaw {
                Yaw::Deg0 => 0,
                Yaw::Deg90 => 1,
                Yaw::Deg180 => 2,
                Yaw::Deg270 => 3,
            },
        })
        .collect();
    Ok((meshes, instances))
}

impl Runtime {
    fn load(directory: PathBuf) -> Result<Self, String> {
        let started = Instant::now();
        let built = matterweave_detail::build_showcase(SEED).map_err(|e| e.to_string())?;
        let clearing = built
            .landmarks
            .iter()
            .find(|p| p.name == "destruction_clearing")
            .map(|p| p.position_m)
            .ok_or("Showcase clearing missing")?;
        let terrain = built.terrain;
        let mut scene = built.scene;
        let mut spawn = built.spawn_eye;
        let route = built.route;
        let (save_path, saved) = SavedWetland::load_recovering(
            &directory.join(wetland_state::SAVE_FILE), GENERATOR, SEED
        )?;
        let fresh = saved.is_none();
        if let Some(save) = &saved {
            for edit in &save.edits {
                scene
                    .edit_instance(&edit.instance, edit.cell, edit.material)
                    .map_err(|e| e.to_string())?;
            }
        }
        let empty_world = World::new(SEED);
        let mut physics = Physics::new(&empty_world);
        let collision = physics.replace_detail_scene(&scene)?;
        let mut camera = Camera {
            position: Vec3::from_array(spawn),
            yaw: 0.,
            pitch: -0.08,
        };
        let mut lighting = LightingSettings::default();
        let edits = if let Some(save) = saved {
            physics.restore(&save.physics)?;
            camera.position = Vec3::from_array(save.physics.eye);
            camera.yaw = save.yaw;
            camera.pitch = save.pitch;
            lighting.shadows = save.shadows;
            save.edits
        } else {
            // The terrain sample describes one column, while the capsule spans
            // neighbouring quarter-metre steps. Find a clear standing pose above
            // that same entrance using actual source colliders, with a bounded
            // one-metre lift. Never disable collision or move to a different route.
            let desired = spawn;
            let mut clear = false;
            for step in 0..=8 {
                spawn[1] = desired[1] + step as f32 * 0.125;
                if physics.teleport(spawn) {
                    clear = true;
                    break;
                }
            }
            if !clear {
                return Err("Wetland entrance overlaps solid geometry".into());
            }
            camera.position = Vec3::from_array(spawn);
            Vec::new()
        };
        let counts = scene.counts();
        if counts.expanded_occupied_cells < 20_000_000 || counts.instances < 4_000 {
            return Err("Showcase generator failed its full-world density gate".into());
        }
        let counts = format!(
            "{} cells / {} placed objects",
            counts.expanded_occupied_cells, counts.instances
        );
        let (meshes, instances) = graphics(&mut scene)?;
        log::info!(
            "WETLAND LOADED: {counts}; collision {collision:?}; {:.3}s",
            started.elapsed().as_secs_f64()
        );
        eprintln!(
            "WETLAND LOADED: {counts}; collision {collision:?}; {:.3}s",
            started.elapsed().as_secs_f64()
        );
        let mut runtime = Self {
            scene,
            meshes,
            instances,
            physics,
            empty_world,
            dynamic: DynamicMeshCache::default(),
            edits,
            save_path,
            spawn,
            route,
            terrain,
            clearing,
            camera,
            lighting,
            dirty: false,
            graphics_dirty: true,
            dynamic_dirty: true,
            counts,
        };
        if fresh {
            runtime.playground()?;
        }
        Ok(runtime)
    }
    fn save(&mut self, _directory: &std::path::Path) -> Result<(), String> {
        let saved = SavedWetland {
            version: 1,
            generator: GENERATOR,
            seed: SEED,
            edits: self.edits.clone(),
            physics: self.physics.snapshot(),
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            shadows: self.lighting.shadows,
        };
        saved.save(&self.save_path)?;
        self.dirty = false;
        Ok(())
    }
    fn edit(&mut self, place: bool) -> Result<String, String> {
        let hit = wetland_state::raycast(
            &self.scene,
            self.camera.position.to_array(),
            self.camera.forward().to_array(),
            6.,
        )
        .ok_or("Move closer and aim at terrain or a mushroom")?;
        let cell = if place {
            hit.previous
                .ok_or("Aim at an exposed edge to add a voxel")?
        } else {
            hit.cell
        };
        if self.edits.len() >= wetland_state::MAX_EDITS
            && !self
                .edits
                .iter()
                .any(|e| e.instance == hit.instance && e.cell == cell)
        {
            return Err("This wetland save has reached its edit limit".into());
        }
        let draw = self
            .scene
            .draws()
            .into_iter()
            .find(|d| d.instance == hit.instance)
            .ok_or("Object is no longer present")?;
        let old = self
            .scene
            .prototype(&draw.prototype)
            .ok_or("Source missing")?
            .get(cell);
        let value = if place { hit.material } else { 0 };
        // Do not build a wall inside the player capsule.
        let volume = self.scene.prototype(&draw.prototype).unwrap();
        let centre = draw
            .transform
            .point_to_world(cell.map(|c| (c as f32 + 0.5) * volume.scale().metres()));
        let delta = Vec3::from_array(centre) - self.camera.position;
        if place && delta.x.abs() < 0.5 && delta.z.abs() < 0.5 && (-1.8..0.3).contains(&delta.y) {
            return Err("Step back before placing here".into());
        }
        self.scene
            .edit_instance(&hit.instance, cell, value)
            .map_err(|e| e.to_string())?;
        let (meshes, instances) = match graphics(&mut self.scene) {
            Ok(graphics) => graphics,
            Err(error) => {
                self.scene
                    .edit_instance(&hit.instance, cell, old)
                    .map_err(|e| e.to_string())?;
                return Err(error);
            }
        };
        if let Err(error) = self.physics.replace_detail_scene(&self.scene) {
            self.scene
                .edit_instance(&hit.instance, cell, old)
                .map_err(|e| e.to_string())?;
            return Err(error);
        }
        self.meshes = meshes;
        self.instances = instances;
        self.graphics_dirty = true;
        if let Some(edit) = self
            .edits
            .iter_mut()
            .find(|e| e.instance == hit.instance && e.cell == cell)
        {
            edit.material = value;
        } else {
            self.edits.push(Edit {
                instance: hit.instance,
                cell,
                material: value,
            });
        }
        self.dirty = true;
        Ok(if place {
            "Voxel added"
        } else {
            "Voxel removed"
        }
        .into())
    }
    fn playground(&mut self) -> Result<(), String> {
        // A bounded six-body arch containing exactly64 half-metre voxels. The
        // nearest designated route clearing supplies the ground, never a visual LOD.
        let centre = self.clearing;
        let hit = wetland_state::raycast(
            &self.scene,
            [centre[0], centre[1] + 12., centre[2]],
            [0., -1., 0.],
            32.,
        )
        .ok_or("Clearing ground unavailable")?;
        let ground = centre[1] + 12. - hit.distance;
        let mut bodies = Vec::new();
        for x in [-1.5, 1.5] {
            for level in 0..2 {
                bodies.push(BodySnapshot {
                    position: [centre[0] + x, ground + 0.52 + level as f32, centre[2]],
                    rotation: [0., 0., 0., 1.],
                    velocity: [0.; 3],
                    angular_velocity: [0.; 3],
                    dimensions: [2; 3],
                    material: 8,
                });
            }
        }
        bodies.push(BodySnapshot {
            position: [centre[0], ground + 2.52, centre[2]],
            rotation: [0., 0., 0., 1.],
            velocity: [0.; 3],
            angular_velocity: [0.; 3],
            dimensions: [6, 2, 2],
            material: 8,
        });
        bodies.push(BodySnapshot {
            position: [centre[0], ground + 0.52, centre[2] + 2.],
            rotation: [0., 0., 0., 1.],
            velocity: [0.; 3],
            angular_velocity: [0.; 3],
            dimensions: [2; 3],
            material: 7,
        });
        self.physics.restore(&PhysicsSnapshot {
            version: 1,
            eye: self.camera.position.to_array(),
            bodies,
        })?;
        self.dirty = true;
        Ok(())
    }
}

pub struct WetlandApp {
    started: Instant,
    capture: Capture,
    directory: PathBuf,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    runtime: Option<Runtime>,
    loading: Option<mpsc::Receiver<Result<Runtime, String>>>,
    controls: Controls,
    menu: bool,
    options: bool,
    diagnostics: bool,
    pub sandbox_requested: bool,
    pub failed: bool,
    status: String,
    last: Instant,
    next_frame: Instant,
    focused: bool,
    frames: u64,
    frame_limit: Option<u64>,
    auto_start: bool,
    fps: f32,
    saved_at: Instant,
}
impl WetlandApp {
    pub fn new(directory: PathBuf, auto_start: bool, frame_limit: Option<u64>) -> Self {
        Self {
            started: Instant::now(),
            capture: Capture::new(&directory),
            directory,
            renderer: None,
            window: None,
            runtime: None,
            loading: None,
            controls: Controls::default(),
            menu: true,
            options: false,
            diagnostics: false,
            sandbox_requested: false,
            failed: false,
            status: String::new(),
            last: Instant::now(),
            next_frame: Instant::now(),
            focused: true,
            frames: 0,
            frame_limit,
            auto_start,
            fps: 0.,
            saved_at: Instant::now(),
        }
    }
    fn enter(&mut self) {
        if self.runtime.is_some() {
            self.menu = false;
            self.controls.clear();
            return;
        }
        if self.loading.is_some() {
            return;
        }
        self.failed = false;
        let directory = self.directory.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.loading = Some(rx);
        self.status = "Shaping the wetland and its living canopy...".into();
        std::thread::spawn(move || {
            let result = Runtime::load(directory);
            let _ = tx.send(result);
        });
    }
    fn click(&mut self, p: Vec2) -> bool {
        if self.menu {
            if self.loading.is_none() && contains([70., 290., 420., 72.], p) {
                self.enter();
            }
            if self.loading.is_none() && contains([70., 382., 420., 54.], p) {
                self.sandbox_requested = true;
            }
            return true;
        }
        if contains([880., 20., 100., 42.], p) {
            self.options = !self.options;
            self.controls.clear();
            return true;
        }
        if self.options {
            for (i, label) in [
                "SAVE",
                "SHADOWS",
                "DIAGNOSTICS",
                "RESET ARCH",
                "RETURN TO MENU",
            ]
            .iter()
            .enumerate()
            {
                if contains([690., 80. + i as f32 * 55., 290., 45.], p) {
                    match i {
                        0 => self.save(),
                        1 => {
                            if let Some(r) = &mut self.runtime {
                                r.lighting.shadows = !r.lighting.shadows;
                                r.dirty = true;
                            }
                        }
                        2 => self.diagnostics = !self.diagnostics,
                        3 => {
                            if let Some(r) = &mut self.runtime {
                                self.status = r
                                    .playground()
                                    .map_or_else(|e| e, |_| "Arch restored in the clearing".into());
                            }
                        }
                        _ => {
                            self.save();
                            self.menu = true;
                            self.options = false;
                            self.controls.clear();
                        }
                    }
                    let _ = label;
                    return true;
                }
            }
            return true;
        }
        for (rect, action) in [
            ([630., 514., 102., 58.], Action::Remove),
            ([742., 514., 102., 58.], Action::Place),
            ([854., 514., 120., 58.], Action::Grab),
            ([854., 444., 120., 54.], Action::Break),
            ([742., 444., 102., 54.], Action::Throw),
        ] {
            if contains(rect, p) {
                self.action(action);
                return true;
            }
        }
        false
    }
    fn action(&mut self, action: Action) {
        let Some(r) = &mut self.runtime else {
            return;
        };
        let eye = r.camera.position.to_array();
        let forward = r.camera.forward().to_array();
        let range = wetland_state::raycast(&r.scene, eye, forward, 6.).map_or(6., |h| h.distance);
        match action {
            Action::Remove | Action::Place => {
                let t = Instant::now();
                self.status = r.edit(action == Action::Place).unwrap_or_else(|e| e);
                log::info!(
                    "WETLAND EDIT {:.3}ms {}",
                    t.elapsed().as_secs_f64() * 1000.,
                    self.status
                );
            }
            Action::Grab => {
                r.physics.grab(&r.empty_world, eye, forward, range);
                r.dirty = true;
            }
            Action::Throw => {
                r.physics.throw(forward);
                r.dirty = true;
            }
            Action::Break => {
                r.physics.break_body(&r.empty_world, eye, forward, range);
                r.dirty = true;
            }
            Action::Home => {
                if r.physics.teleport(r.spawn) {
                    r.camera.position = Vec3::from_array(r.spawn);
                }
            }
            _ => {}
        }
    }
    fn save(&mut self) {
        if let Some(r) = &mut self.runtime {
            self.status = r
                .save(&self.directory)
                .map_or_else(|e| format!("Save failed: {e}"), |_| "Wetland saved".into());
            self.saved_at = Instant::now();
        }
    }
    fn hud(&self) -> Hud {
        let mut h = Hud::new(1000., 600.);
        if !self.menu {
            if let Some(r) = &self.runtime {
                if r.terrain
                    .water_surface_at_metres(r.camera.position.x, r.camera.position.z)
                    .is_some_and(|y| y > r.camera.position.y)
                {
                    h.rect([0., 0., 1000., 600.], [0.02, 0.23, 0.24, 0.45]);
                }
            }
        }
        let ink = [0.87, 0.91, 0.82, 1.];
        let gold = [0.82, 0.65, 0.37, 1.];
        let panel = [0.035, 0.065, 0.065, 0.92];
        if self.menu {
            h.rect([0., 0., 1000., 600.], [0.045, 0.08, 0.085, 1.]);
            for i in 0..18 {
                let x = 520. + i as f32 * 29.;
                let y = 230. + ((i * 17) % 70) as f32;
                h.rect([x, y, 8., 300. - y], [0.16, 0.24, 0.21, 1.]);
                h.rect([x - 16., y - 8., 42., 14.], [0.34, 0.30, 0.23, 1.]);
            }
            h.text(70., 86., "MATTERWEAVE", 4., ink);
            h.text(73., 142., "THE AMBER MIRE", 2.4, gold);
            h.text(74., 200., "Follow the creek. Find the ridge.", 1.5, ink);
            h.text(74., 224., "Shape a world made of voxels.", 1.5, ink);
            h.rect([70., 290., 420., 72.], [0.24, 0.34, 0.28, 1.]);
            h.text(
                96.,
                315.,
                if self.loading.is_some() {
                    "GROWING THE WETLAND..."
                } else if self.runtime.is_some() {
                    "CONTINUE EXPLORING"
                } else {
                    "ENTER THE WETLAND"
                },
                1.7,
                ink,
            );
            h.rect([70., 382., 420., 54.], panel);
            h.text(96., 401., "OPEN YOUR SANDBOX", 1.4, ink);
            h.text(74., 475., "An original alien wetland", 1.25, gold);
            h.text(
                74.,
                506.,
                &self.status.chars().take(88).collect::<String>(),
                1.,
                ink,
            );
            return h;
        }
        h.rect([18., 18., 305., 67.], panel);
        h.text(32., 30., "THE AMBER MIRE", 1.65, gold);
        if let Some(r) = &self.runtime {
            let d = (Vec3::from_array(r.clearing) - r.camera.position).length();
            let route_d = r
                .route
                .iter()
                .map(|p| {
                    let v = Vec3::from_array(*p) - r.camera.position;
                    v.x * v.x + v.z * v.z
                })
                .fold(f32::INFINITY, f32::min)
                .sqrt();
            h.text(
                32.,
                58.,
                &format!("Clearing {:.0}m / path {:.0}m", d, route_d),
                1.05,
                ink,
            );
        }
        h.rect([880., 20., 100., 42.], panel);
        h.text(894., 34., "MENU", 1.4, ink);
        h.rect([60., 410., 142., 142.], [0.08, 0.14, 0.14, 0.5]);
        h.text(100., 475., "MOVE", 1.3, ink);
        h.rect([910., 367., 64., 55.], panel);
        h.text(918., 387., "JUMP", 1.1, ink);
        for (rect, text) in [
            ([630., 514., 102., 58.], "REMOVE"),
            ([742., 514., 102., 58.], "PLACE"),
            ([854., 514., 120., 58.], "GRAB"),
            ([854., 444., 120., 54.], "BREAK"),
            ([742., 444., 102., 54.], "THROW"),
        ] {
            h.rect(rect, panel);
            h.text(rect[0] + 10., rect[1] + 22., text, 1.2, ink);
        }
        h.rect([498., 297., 4., 6.], ink);
        h.rect([497., 298., 6., 4.], ink);
        h.text(
            250.,
            578.,
            &self.status.chars().take(65).collect::<String>(),
            1.,
            gold,
        );
        if self.options {
            for (i, label) in [
                "SAVE",
                "SHADOWS",
                "DIAGNOSTICS",
                "RESET ARCH",
                "RETURN TO MENU",
            ]
            .iter()
            .enumerate()
            {
                let y = 80. + i as f32 * 55.;
                h.rect([690., y, 290., 45.], panel);
                h.text(710., y + 15., label, 1.25, ink);
            }
        }
        if self.diagnostics {
            if let Some(r) = &self.runtime {
                h.rect([18., 94., 620., 66.], panel);
                h.text(30., 104., &r.counts, 1., ink);
                h.text(
                    30.,
                    126.,
                    &format!(
                        "{:.1} FPS / {} bodies / {:.1} {:.1} {:.1}",
                        self.fps,
                        r.physics.body_count(),
                        r.camera.position.x,
                        r.camera.position.y,
                        r.camera.position.z
                    ),
                    1.,
                    ink,
                );
            }
        }
        h
    }
    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if !self.focused {
            return;
        }
        if let Some(rx) = &self.loading {
            match rx.try_recv() {
                Ok(Ok(runtime)) => {
                    self.runtime = Some(runtime);
                    self.loading = None;
                    self.menu = false;
                    self.frames = 0;
                    self.status = "Left thumb moves. Drag right to look. Follow the creek.".into();
                }
                Ok(Err(e)) => {
                    self.loading = None;
                    self.status = format!("Unable to enter: {e}");
                    log::error!("{}", self.status);
                    self.failed = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = None;
                    self.status = "Wetland loading stopped".into();
                    self.failed = true;
                }
                Err(_) => {}
            }
        }
        if self.failed && self.frame_limit.is_some() {
            event_loop.exit();
            return;
        }
        let capturing = self.capture.enabled() && !self.menu && self.runtime.is_some();
        let capture_start = Instant::now();
        let capture_cpu = metrics::CpuBusySpan::begin(capturing);
        let mut row = metrics::FrameRow::default();
        if let Some(renderer) = &mut self.renderer {
            renderer.begin_frame_diagnostics();
        }
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        row.draw_interval_wall_ms = Some((now - self.last).as_secs_f64() * 1000.);
        self.last = now;
        let physics_start = Instant::now();
        if !self.menu {
            if let Some(r) = &mut self.runtime {
                let (motion, look) = self.controls.consume();
                r.camera.yaw -= look.x * 0.004;
                r.camera.pitch = (r.camera.pitch - look.y * 0.004).clamp(-1.5, 1.5);
                let forward = Vec3::new(r.camera.yaw.sin(), 0., r.camera.yaw.cos());
                let right = Vec3::new(-r.camera.yaw.cos(), 0., r.camera.yaw.sin());
                // Water is nonblocking: the character walks the physical bed. Wading
                // slows locomotion while the eye can go underwater in deeper basin areas.
                let depth = r
                    .terrain
                    .water_surface_at_metres(r.camera.position.x, r.camera.position.z)
                    .map_or(0., |surface| surface - (r.camera.position.y - 1.5));
                let speed = if depth > 0.2 {
                    2.0
                } else {
                    matterweave_detail::WALK_SPEED_M_S
                };
                let velocity = (forward * motion.z + right * motion.x) * speed;
                r.physics
                    .update_grab(r.camera.position.to_array(), r.camera.forward().to_array());
                row.physics_fixed_steps =
                    Some(r.physics.step(dt, velocity.to_array(), motion.y > 0.) as u32);
                r.camera.position = Vec3::from_array(r.physics.character_eye());
                if motion.length_squared() > 0.
                    || look.length_squared() > 0.
                    || r.physics.body_activity().active > 0
                {
                    r.dirty = true;
                }
                if r.camera.position.y < -40. {
                    r.physics.teleport(r.spawn);
                }
                row.physics_wall_ms = Some(physics_start.elapsed().as_secs_f64() * 1000.);
                let activity = r.physics.body_activity();
                row.voxel_bodies_total = Some(activity.total as u32);
                row.voxel_bodies_active = Some(activity.active as u32);
                row.voxel_bodies_sleeping = Some(activity.sleeping as u32);
                row.voxel_bodies_not_simulated = Some(activity.not_simulated as u32);
                let mesh_start = Instant::now();
                let changed = r.dynamic.update(&r.physics);
                r.dynamic_dirty |= changed;
                row.dynamic_mesh_build_wall_ms = Some(mesh_start.elapsed().as_secs_f64() * 1000.);
                row.dynamic_mesh_builds = Some(u32::from(changed));
            }
        }
        let save_start = Instant::now();
        let save_due = self.saved_at.elapsed() > Duration::from_secs(30)
            && self.runtime.as_ref().is_some_and(|r| r.dirty);
        if save_due {
            self.save();
        }
        row.save_attempts = Some(u32::from(save_due));
        row.save_failures = Some(u32::from(
            save_due && self.runtime.as_ref().is_some_and(|r| r.dirty),
        ));
        row.save_wall_ms = Some(save_start.elapsed().as_secs_f64() * 1000.);
        let h = self.hud();
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let Some(window) = &self.window else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let (camera, lighting) = if let Some(r) = &mut self.runtime {
            let sync_start = Instant::now();
            if r.graphics_dirty {
                if let Err(e) = renderer.replace_static_scene(&r.meshes, &r.instances) {
                    self.status = e;
                    self.failed = true;
                    event_loop.exit();
                    return;
                }
                r.graphics_dirty = false;
            }
            row.mesh_sync_wall_ms = Some(sync_start.elapsed().as_secs_f64() * 1000.);
            row.chunk_mesh_uploads = Some(0);
            let dynamic_start = Instant::now();
            row.dynamic_mesh_uploads = Some(u32::from(r.dynamic_dirty));
            if r.dynamic_dirty {
                if let Err(e) = renderer.upload_dynamic(r.dynamic.mesh()) {
                    self.status = e;
                    self.failed = true;
                    event_loop.exit();
                    return;
                }
                r.dynamic_dirty = false;
            }
            row.dynamic_upload_wall_ms = Some(dynamic_start.elapsed().as_secs_f64() * 1000.);
            (&r.camera, r.lighting)
        } else {
            static CAMERA: std::sync::LazyLock<Camera> = std::sync::LazyLock::new(Camera::default);
            (
                &*CAMERA,
                LightingSettings {
                    shadows: false,
                    ..Default::default()
                },
            )
        };
        let _ = renderer.set_wetland_material_time(Some(self.started.elapsed().as_secs_f32()));
        let render_start = Instant::now();
        let result = renderer.render_with_lighting(
            camera.view_projection(size.width as f32 / size.height as f32),
            camera.position.to_array(),
            &h,
            &lighting,
        );
        row.render_wall_ms = Some(render_start.elapsed().as_secs_f64() * 1000.);
        if capturing {
            self.capture
                .record(renderer, &result, row, capture_start, capture_cpu);
        }
        match result {
            FrameResult::Presented => {
                if !self.menu {
                    self.frames += 1;
                }
                self.fps = if dt > 0. {
                    self.fps * 0.9 + 0.1 / dt
                } else {
                    self.fps
                };
            }
            FrameResult::Retry => {}
            FrameResult::Fatal(e) => {
                self.status = e;
                self.failed = true;
                event_loop.exit();
            }
            FrameResult::OutOfMemory => {
                self.failed = true;
                event_loop.exit();
            }
        }
        if self.frame_limit.is_some_and(|n| self.frames >= n) && self.runtime.is_some() {
            eprintln!("WETLAND SMOKE PASS: {} frames", self.frames);
            event_loop.exit();
        }
        self.next_frame = Instant::now() + Duration::from_millis(if self.menu { 66 } else { 1 });
    }
    fn point(&self, x: f64, y: f64) -> Vec2 {
        let s = self.window.as_ref().unwrap().inner_size();
        Vec2::new(
            x as f32 / s.width.max(1) as f32 * 1000.,
            y as f32 / s.height.max(1) as f32 * 600.,
        )
    }
}
impl ApplicationHandler for WetlandApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        self.focused = true;
        self.last = Instant::now();
        self.controls.clear();
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Matterweave | The Amber Mire")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 768)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.status = e.to_string();
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(mut renderer) => {
                self.capture.renderer_created(&mut renderer);
                log::info!("Wetland graphics: {}", renderer.capabilities);
                self.renderer = Some(renderer);
                self.window = Some(window);
                if let Some(r) = &mut self.runtime {
                    r.graphics_dirty = true;
                    r.dynamic_dirty = true;
                }
            }
            Err(e) => {
                self.status = e;
                self.failed = true;
                event_loop.exit();
            }
        }
        if self.auto_start {
            self.auto_start = false;
            self.enter();
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.capture.flush();
        self.save();
        self.focused = false;
        self.controls.clear();
        self.renderer = None;
        self.window = None;
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.save();
                event_loop.exit();
            }
            WindowEvent::Resized(s) => {
                self.controls.clear();
                if let Some(r) = &mut self.renderer {
                    r.resize(s.width, s.height);
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                self.last = Instant::now();
                if !f {
                    self.controls.clear();
                    self.save();
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed
                    && (event.logical_key == Key::Named(NamedKey::BrowserBack)
                        || event.physical_key == PhysicalKey::Code(KeyCode::Escape))
                {
                    if self.menu {
                        event_loop.exit();
                    } else {
                        self.save();
                        self.menu = true;
                        self.controls.clear();
                    }
                    return;
                }
                if let PhysicalKey::Code(key) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        match key {
                            KeyCode::Enter if self.menu => self.enter(),
                            KeyCode::KeyE => self.action(Action::Place),
                            KeyCode::KeyR => self.action(Action::Remove),
                            KeyCode::KeyG => self.action(Action::Grab),
                            KeyCode::KeyT => self.action(Action::Throw),
                            KeyCode::KeyB => self.action(Action::Break),
                            KeyCode::Home => self.action(Action::Home),
                            KeyCode::F5 => self.save(),
                            KeyCode::F3 => self.diagnostics = !self.diagnostics,
                            _ => {
                                self.controls.keys.insert(key);
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
                }
                if state == ElementState::Pressed && button == MouseButton::Left {
                    if let Some(p) = self.controls.cursor {
                        if !self.click(p) && !self.menu {
                            self.action(Action::Remove);
                        }
                    }
                }
            }
            WindowEvent::Touch(t) => {
                let p = self.point(t.location.x, t.location.y);
                match t.phase {
                    TouchPhase::Started => {
                        if !self.click(p) && !self.menu {
                            self.controls.start_wetland(t.id, p);
                        }
                    }
                    TouchPhase::Moved => self.controls.moved(t.id, p),
                    TouchPhase::Ended => self.controls.end(t.id),
                    TouchPhase::Cancelled => self.controls.clear(),
                }
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.focused {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
        if Instant::now() >= self.next_frame {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.capture.flush();
        self.save();
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Full authoritative scene, actual Rapier adapters and app save/edit path.
    /// Explicit opt-in because it generates and meshes the map twice.
    #[test]
    #[ignore = "full-map integration: run explicitly in the campaign gate"]
    fn full_wetland_load_edit_collision_and_reload() {
        let directory = std::env::temp_dir().join(format!(
            "matterweave-wetland-runtime-{}", std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let legacy = directory.join("world.json");
        std::fs::write(&legacy, b"legacy world sentinel").unwrap();
        let mut runtime = Runtime::load(directory.clone()).expect("full wetland load");
        assert_eq!(runtime.physics.body_count(), 6);
        let start = runtime.physics.character_eye();
        for _ in 0..120 {
            runtime.physics.step(1. / 60., [0.; 3], false);
        }
        let settled = runtime.physics.character_eye();
        assert!((settled[1] - start[1]).abs() < 1., "entrance floor lost: {start:?} -> {settled:?}");
        runtime.physics.step(1. / 60., [0.; 3], true);
        for _ in 0..12 { runtime.physics.step(1. / 60., [0.; 3], false); }
        assert!(runtime.physics.character_eye()[1] > settled[1] + 0.1, "entrance cannot jump");
        for _ in 0..120 { runtime.physics.step(1. / 60., [0.; 3], false); }
        runtime.camera.position = Vec3::from_array(runtime.physics.character_eye());
        runtime.camera.pitch = -1.2;
        let before = runtime.scene.counts().expanded_occupied_cells;
        runtime.edit(false).expect("remove actual aimed source cell");
        assert_eq!(runtime.scene.counts().expanded_occupied_cells, before - 1);
        let edit = runtime.edits.last().unwrap().clone();
        runtime.save(&directory).unwrap();
        let saved_eye = runtime.physics.character_eye();
        drop(runtime);
        let restored = Runtime::load(directory.clone()).expect("reload edited full wetland");
        assert_eq!(restored.physics.character_eye(), saved_eye);
        assert_eq!(restored.scene.counts().expanded_occupied_cells, before - 1);
        assert_eq!(restored.edits, vec![edit.clone()]);
        let draw = restored.scene.draws().into_iter().find(|d| d.instance == edit.instance).unwrap();
        assert_eq!(restored.scene.prototype(&draw.prototype).unwrap().get(edit.cell), 0);
        assert_eq!(std::fs::read(legacy).unwrap(), b"legacy world sentinel");
        drop(restored);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
