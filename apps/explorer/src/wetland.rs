//! Player-facing wetland experience; source, collision, derived graphics and saves
//! remain separate. Legacy sandbox files are never used by this mode.
use crate::{
    controls::{contains, Action, Camera, Controls},
    metrics,
    wetland_metrics::Capture,
    wetland_replay::{Replay, Route},
    wetland_state::{self, Edit, SavedWetland},
};
use glam::{Vec2, Vec3};
use matterweave_core::{Mesh, World};
use matterweave_detail::{DetailScene, Lod, Yaw};
use matterweave_physics::{
    BodySnapshot, DetailCollisionCadence, DetailCollisionStats, DynamicMeshCache, Physics,
    PhysicsSnapshot,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, StaticInstance};
use std::{
    collections::BTreeSet,
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
    elevated_route: Vec<[f32; 3]>,
    camera: Camera,
    lighting: LightingSettings,
    dirty: bool,
    graphics_dirty: bool,
    dynamic_dirty: bool,
    counts: String,
    /// Edit-to-collision cadence: preparation is queued per edit and
    /// published at most once per frame; see `sync_detail_collision`.
    collision: DetailCollisionCadence,
    /// Edits queued for collision preparation but not yet confirmed by a
    /// publication. Reverted as a group if preparation of the current
    /// source fails, so the authoritative scene returns to the state its
    /// live collision was published from.
    pending_edits: Vec<PendingEdit>,
}

/// One authoritative edit awaiting collision publication.
struct PendingEdit {
    instance: String,
    cell: [i32; 3],
    /// Scene material the cell held before this unconfirmed burst.
    old: u8,
    /// Save-journal entry the cell held before this burst: `Some(material)`
    /// when a confirmed entry already existed, `None` when this burst created
    /// it. Rollback restores exactly this — a confirmed removal (journal
    /// material 0) reverted by a failed re-add must come back as a journal
    /// removal, not vanish — because restoring only the scene material would
    /// silently drop confirmed history and corrupt save replay.
    journal: Option<u8>,
}

impl PendingEdit {
    /// Restores the scene cell and the exact prior save-journal state.
    fn rollback(self, scene: &mut DetailScene, edits: &mut Vec<Edit>) {
        let _ = scene.edit_instance(&self.instance, self.cell, self.old);
        match self.journal {
            None => {
                edits.retain(|edit| edit.instance != self.instance || edit.cell != self.cell);
            }
            Some(material) => {
                if let Some(edit) = edits
                    .iter_mut()
                    .find(|edit| edit.instance == self.instance && edit.cell == self.cell)
                {
                    edit.material = material;
                } else {
                    edits.push(Edit {
                        instance: self.instance,
                        cell: self.cell,
                        material,
                    });
                }
            }
        }
    }
}

/// World-space `(min, max)` box of one prototype cell of a placed instance.
/// All eight corners go through the exact instance transform, so quarter-turn
/// yaw is honoured without assuming axis order. `None` when the prototype is
/// gone; the caller then treats the change as structural.
fn cell_world_aabb(
    scene: &DetailScene,
    draw: &matterweave_detail::InstanceDraw,
    cell: [i32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let scale = scene.prototype(&draw.prototype)?.scale().metres();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for dx in 0..=1 {
        for dy in 0..=1 {
            for dz in 0..=1 {
                let local = [
                    (cell[0] + dx) as f32 * scale,
                    (cell[1] + dy) as f32 * scale,
                    (cell[2] + dz) as f32 * scale,
                ];
                let world = draw.transform.point_to_world(local);
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(world[axis]);
                    hi[axis] = hi[axis].max(world[axis]);
                }
            }
        }
    }
    Some((lo, hi))
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

struct PreparedJournal {
    spawn: [f32; 3],
    physics: Physics,
    collision: DetailCollisionStats,
    meshes: Vec<Mesh>,
    instances: Vec<StaticInstance>,
}

/// Validates candidate source, collision, bodies, player pose and derived meshes
/// before accepting any of it into the runtime.
///
/// Isolation: the candidate is applied to a transient [`DetailScene::fork_source`]
/// copy of the authoritative data — never to the live scene — and the body
/// payload is checked with [`Physics::restore`] on a separate empty probe world,
/// which validates the whole snapshot *before* touching any body, so the physics
/// rules are not restated here. Only a fully accepted candidate replaces the live
/// scene; a rejected fork (including any private prototype minted by the first
/// shared-prototype edit) is dropped, so cell-by-cell undo of the live scene —
/// which could not retract that private prototype, its revision or its source
/// accounting — is never attempted, and later candidates start pristine.
fn apply_journal(
    scene: &mut DetailScene,
    probe: &mut Physics,
    instances: &BTreeSet<String>,
    save: &SavedWetland,
    world: &World,
    spawn: [f32; 3],
) -> Result<PreparedJournal, String> {
    for edit in &save.edits {
        if !instances.contains(&edit.instance) {
            return Err(format!(
                "saved edit names absent placement {}",
                edit.instance
            ));
        }
    }
    probe.restore(&save.physics)?;
    let mut fork = scene.fork_source();
    for edit in &save.edits {
        fork.edit_instance(&edit.instance, edit.cell, edit.material)
            .map_err(|error| error.to_string())?;
    }
    let mut physics = Physics::new(world);
    let collision = physics.replace_detail_scene(&fork)?;
    // Validate source collision before restoring bodies: contact with a saved
    // dynamic body must not itself relocate the player.
    let entrance = clear_pose(&mut physics, spawn);
    let desired = save.physics.eye;
    let eye = clear_pose(&mut physics, desired)
        .or(entrance)
        .ok_or("Wetland save and entrance both overlap solid geometry")?;
    physics.restore(&PhysicsSnapshot {
        eye,
        ..save.physics.clone()
    })?;
    let (meshes, instances) = graphics(&mut fork)?;
    if eye != desired {
        log::warn!("Wetland saved viewpoint {desired:?} was inside source; moved to {eye:?}");
        eprintln!("WETLAND POSE CORRECTED: {desired:?} -> {eye:?}");
    }
    *scene = fork;
    Ok(PreparedJournal {
        // If edits completely block the entrance, the validated saved viewpoint
        // is the recovery point. Never retain a known overlapping respawn pose.
        spawn: entrance.unwrap_or(eye),
        physics,
        collision,
        meshes,
        instances,
    })
}

/// First pose at or bounded-above `desired` that the actual source colliders
/// accept. The lift is one metre in eighth-metre steps; collision is
/// never disabled and the horizontal position is never moved.
fn clear_pose(physics: &mut Physics, desired: [f32; 3]) -> Option<[f32; 3]> {
    (0..=8).find_map(|step| {
        let pose = [desired[0], desired[1] + step as f32 * 0.125, desired[2]];
        physics.teleport(pose).then_some(pose)
    })
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
        let spawn = built.spawn_eye;
        let route = built.route;
        let elevated_route = built.elevated_route;
        let empty_world = World::new(SEED);
        let mut restored = None;
        // Separate empty world: validating a candidate body payload must not add
        // bodies to the physics that a later candidate or the session itself uses.
        let mut probe = Physics::new(&empty_world);
        let instances: BTreeSet<String> = scene.instance_ids().into_iter().collect();
        let (save_path, saved) = SavedWetland::load_recovering_with(
            &directory.join(wetland_state::SAVE_FILE),
            GENERATOR,
            SEED,
            |save| {
                restored = Some(apply_journal(
                    &mut scene,
                    &mut probe,
                    &instances,
                    save,
                    &empty_world,
                    spawn,
                )?);
                Ok(())
            },
        )?;
        drop(probe);
        let fresh = saved.is_none();
        let PreparedJournal {
            mut spawn,
            mut physics,
            collision,
            meshes,
            instances,
        } = match restored {
            Some(value) => value,
            None => {
                let mut physics = Physics::new(&empty_world);
                let collision = physics.replace_detail_scene(&scene)?;
                let (meshes, instances) = graphics(&mut scene)?;
                PreparedJournal {
                    spawn,
                    physics,
                    collision,
                    meshes,
                    instances,
                }
            }
        };
        let mut camera = Camera {
            position: Vec3::from_array(spawn),
            yaw: 0.,
            pitch: -0.08,
        };
        let mut lighting = LightingSettings::default();
        let edits = if let Some(save) = saved {
            camera.position = Vec3::from_array(physics.character_eye());
            camera.yaw = save.yaw;
            camera.pitch = save.pitch;
            lighting.shadows = save.shadows;
            save.edits
        } else {
            // The terrain sample describes one column, while the capsule spans
            // neighbouring quarter-metre steps. Find a clear standing pose above
            // that same entrance using actual source colliders, with a bounded
            // one-metre lift. Never disable collision or move to a different route.
            spawn = clear_pose(&mut physics, spawn)
                .ok_or("Wetland entrance overlaps solid geometry")?;
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
            elevated_route,
            terrain,
            clearing,
            camera,
            lighting,
            dirty: false,
            graphics_dirty: true,
            dynamic_dirty: true,
            counts,
            collision: DetailCollisionCadence::new(),
            pending_edits: Vec::new(),
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
        // The world-space box of the edited cell: the publication gate defers
        // the new collision while a body overlaps it. Only newly added solid
        // material gates; removals publish as soon as preparation completes.
        let adds_solid = matterweave_detail::material_policy(value)
            == matterweave_detail::MaterialPolicy::Collision
            && matterweave_detail::material_policy(old)
                != matterweave_detail::MaterialPolicy::Collision;
        // A missing prototype here means the scene changed under the edit;
        // report a structural change so the gate stays conservative.
        let added_region: Option<Vec<([f32; 3], [f32; 3])>> = if !adds_solid {
            Some(Vec::new())
        } else {
            cell_world_aabb(&self.scene, &draw, cell).map(|bounds| vec![bounds])
        };
        // The edit is authoritative now; collision preparation is queued and
        // published on the frame cadence (`sync_detail_collision`). Live
        // collision keeps serving movement until the newer shapes publish, so
        // movement and queries stay correct while the work is pending. The
        // visible scene may lead the world (a new wall is seen before it is
        // solid); the cadence's publication gate guarantees added solid
        // material can never materialise through the character.
        match self
            .collision
            .on_edit(&self.scene, &mut self.physics, added_region.as_deref())
        {
            Ok(true) => {
                // Coalesce per (instance, cell): one entry per edited cell,
                // carrying the scene material and the save-journal entry from
                // before this unconfirmed burst, so `pending_edits` stays
                // bounded by distinct edited cells and a failed preparation
                // reverts the whole burst exactly, including confirmed
                // journal history.
                let exists = self
                    .pending_edits
                    .iter()
                    .any(|pending| pending.instance == hit.instance && pending.cell == cell);
                if !exists {
                    let journal = self
                        .edits
                        .iter()
                        .find(|edit| edit.instance == hit.instance && edit.cell == cell)
                        .map(|edit| edit.material);
                    self.pending_edits.push(PendingEdit {
                        instance: hit.instance.clone(),
                        cell,
                        old,
                        journal,
                    });
                }
            }
            Ok(false) => {
                // A synchronous acceptance confirms the whole pending burst too.
                self.pending_edits.clear();
            }
            Err(error) => {
                // Even the synchronous fallback rejected the source: restore
                // the scene and leave live collision untouched.
                self.scene
                    .edit_instance(&hit.instance, cell, old)
                    .map_err(|e| e.to_string())?;
                return Err(error);
            }
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

    /// Per-frame detail-collision publication, called on the simulation
    /// thread before physics stepping.
    ///
    /// Publishes at most one completed preparation per frame. Results for a
    /// scene that has since been edited, replaced or reset are rejected by
    /// the cadence and simply do not publish; movement and queries keep using
    /// the last accepted collision meanwhile. When preparation of the current
    /// source fails, every unconfirmed edit is reverted as a group (scene,
    /// journal and meshes) and the reverted source is re-queued, so the
    /// authoritative scene returns to the state its live collision was
    /// published from. Returns a status message only on failure.
    fn sync_detail_collision(&mut self) -> Option<String> {
        match self.collision.step(&self.scene, &mut self.physics) {
            Ok(None) => None,
            Ok(Some(_)) => {
                // A publication always covers the current scene version, so
                // it confirms the whole unconfirmed burst.
                self.pending_edits.clear();
                None
            }
            Err(error) => {
                // Revert the whole unconfirmed burst, restoring each cell's
                // exact prior save-journal entry (see `PendingEdit`): a
                // confirmed removal re-added by this burst comes back as a
                // journal removal instead of vanishing from the save.
                for pending in self.pending_edits.drain(..).rev() {
                    pending.rollback(&mut self.scene, &mut self.edits);
                }
                // The reverted content is exactly what graphics accepted
                // before, so this only fails if the scene is otherwise
                // corrupt; the last meshes stay up rather than blanking.
                if let Ok((meshes, instances)) = graphics(&mut self.scene) {
                    self.meshes = meshes;
                    self.instances = instances;
                    self.graphics_dirty = true;
                }
                self.dirty = true;
                // Re-queue the reverted source; its publication restores the
                // collision/scene match (it is usually already live). The
                // reverted scene matches the last accepted publication, so no
                // added-solid region is outstanding.
                let _ = self
                    .collision
                    .on_edit(&self.scene, &mut self.physics, Some(&[]));
                Some(format!("Edit rejected: {error}"))
            }
        }
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
    replay_checked: bool,
    replay: Option<Replay>,
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
            replay_checked: false,
            replay: None,
        }
    }
    fn finish_replay(&mut self) {
        if self.replay.as_mut().is_some_and(Replay::take_finished) {
            self.save();
            self.capture.flush();
        }
    }
    fn cancel_replay(&mut self, reason: &'static str) {
        if let Some(replay) = &mut self.replay {
            replay.cancel(reason);
        }
        self.finish_replay();
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
        if self.menu {
            return;
        }
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
            Action::Home if r.physics.teleport(r.spawn) => {
                r.camera.position = Vec3::from_array(r.spawn);
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
        if !self.focused && self.frame_limit.is_none() {
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
        let frame_dt = (now - self.last).as_secs_f32();
        let dt = frame_dt.min(0.1);
        row.draw_interval_wall_ms = Some((now - self.last).as_secs_f64() * 1000.);
        self.last = now;
        let physics_start = Instant::now();
        // Explicit bounded headless smoke runs already render without X11 focus.
        // Apply the same allowance to replay startup; normal phone runs still
        // require focus and every subsequent focus-loss event cancels the run.
        if !self.menu
            && !self.options
            && (self.focused || self.frame_limit.is_some())
            && !self.replay_checked
        {
            if let Some(r) = &self.runtime {
                self.replay_checked = true;
                match Replay::requested(
                    &self.directory,
                    &r.route,
                    &r.elevated_route,
                    r.camera.position.to_array(),
                ) {
                    Ok(replay) => self.replay = replay,
                    Err(e) => log::error!("Wetland replay request rejected: {e}"),
                }
            }
        }
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
                let replay_active = self.replay.as_ref().is_some_and(Replay::active);
                let mut velocity = (forward * motion.z + right * motion.x) * speed;
                if let Some(replay) = self.replay.as_mut().filter(|replay| replay.active()) {
                    let route = match replay.route() {
                        Route::Ground => &r.route,
                        Route::Elevated => &r.elevated_route,
                    };
                    replay.observe(
                        route,
                        r.camera.position.to_array(),
                        r.physics.grounded(),
                        0.,
                        0,
                        false,
                    );
                    velocity = replay.velocity(route, r.camera.position.to_array(), speed, dt);
                    if velocity.length_squared() > 0. {
                        r.camera.yaw = velocity.x.atan2(velocity.z);
                    }
                    r.camera.pitch = -0.08;
                    r.dirty = true;
                }
                r.physics
                    .update_grab(r.camera.position.to_array(), r.camera.forward().to_array());
                // Publish at most one completed detail-collision preparation
                // before stepping, so movement always runs against a
                // consistent, accepted collision state.
                if let Some(message) = r.sync_detail_collision() {
                    self.status = message;
                }
                row.physics_fixed_steps = Some(r.physics.step(
                    dt,
                    velocity.to_array(),
                    !replay_active && motion.y > 0.,
                ) as u32);
                r.camera.position = Vec3::from_array(r.physics.character_eye());
                if motion.length_squared() > 0.
                    || look.length_squared() > 0.
                    || r.physics.body_activity().active > 0
                {
                    r.dirty = true;
                }
                let respawn = r.camera.position.y < -40.;
                if let Some(replay) = &mut self.replay {
                    let route = match replay.route() {
                        Route::Ground => &r.route,
                        Route::Elevated => &r.elevated_route,
                    };
                    replay.observe(
                        route,
                        r.camera.position.to_array(),
                        r.physics.grounded(),
                        frame_dt,
                        row.physics_fixed_steps.unwrap_or(0),
                        respawn,
                    );
                }
                if respawn {
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
        renderer.set_world_visible(!self.menu);
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
        self.finish_replay();
        self.next_frame =
            capture_start + Duration::from_micros(if self.menu { 66_667 } else { 16_667 });
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
        self.cancel_replay("lifecycle suspended");
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
        match &event {
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::MouseInput { .. } => self.cancel_replay("user input"),
            WindowEvent::Focused(false) | WindowEvent::CloseRequested => {
                self.cancel_replay("focus lost or close")
            }
            WindowEvent::CursorMoved { .. } if self.controls.mouse_look => {
                self.cancel_replay("mouse look")
            }
            _ => {}
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
        if !self.focused && self.frame_limit.is_none() {
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
        self.cancel_replay("app exiting");
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
            "matterweave-wetland-runtime-{}",
            std::process::id()
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
        assert!(
            (settled[1] - start[1]).abs() < 1.,
            "entrance floor lost: {start:?} -> {settled:?}"
        );
        runtime.physics.step(1. / 60., [0.; 3], true);
        for _ in 0..12 {
            runtime.physics.step(1. / 60., [0.; 3], false);
        }
        assert!(
            runtime.physics.character_eye()[1] > settled[1] + 0.1,
            "entrance cannot jump"
        );
        for _ in 0..120 {
            runtime.physics.step(1. / 60., [0.; 3], false);
        }
        runtime.camera.position = Vec3::from_array(runtime.physics.character_eye());
        runtime.camera.pitch = -1.2;
        let before = runtime.scene.counts().expanded_occupied_cells;
        let mut app = WetlandApp::new(directory.clone(), false, None);
        app.runtime = Some(runtime);
        app.action(Action::Remove);
        runtime = app.runtime.take().unwrap();
        assert!(runtime.edits.is_empty(), "main menu accepted a hidden edit");
        assert_eq!(runtime.scene.counts().expanded_occupied_cells, before);
        runtime
            .edit(false)
            .expect("remove actual aimed source cell");
        assert_eq!(runtime.scene.counts().expanded_occupied_cells, before - 1);
        let edit = runtime.edits.last().unwrap().clone();
        runtime.save(&directory).unwrap();
        let saved_eye = runtime.physics.character_eye();
        let saved_bodies = runtime.physics.snapshot().bodies;
        drop(runtime);
        let primary = directory.join(wetland_state::SAVE_FILE);
        let recovery = directory.join(format!("{}.recovery-1.json", wetland_state::SAVE_FILE));
        std::fs::rename(&primary, &recovery).unwrap();
        let valid_bytes = std::fs::read(&recovery).unwrap();
        let mut invalid = SavedWetland::load(&recovery, GENERATOR, SEED)
            .unwrap()
            .unwrap();
        invalid.edits.push(Edit {
            instance: "absent-placement".into(),
            cell: [0; 3],
            material: 0,
        });
        invalid.save(&primary).unwrap();
        let primary_bytes = std::fs::read(&primary).unwrap();
        invalid.edits.pop();
        invalid.physics.bodies[0].material = 0;
        let bad_body = directory.join(format!("{}.recovery-2.json", wetland_state::SAVE_FILE));
        invalid.save(&bad_body).unwrap();
        let body_bytes = std::fs::read(&bad_body).unwrap();
        let restored = Runtime::load(directory.clone()).expect("recover edited full wetland");
        assert_eq!(restored.save_path, recovery);
        assert_eq!(std::fs::read(&primary).unwrap(), primary_bytes);
        assert_eq!(std::fs::read(&bad_body).unwrap(), body_bytes);
        assert_eq!(std::fs::read(&recovery).unwrap(), valid_bytes);
        assert_eq!(restored.physics.character_eye(), saved_eye);
        assert_eq!(restored.physics.snapshot().bodies, saved_bodies);
        assert_eq!(restored.scene.counts().expanded_occupied_cells, before - 1);
        assert_eq!(restored.edits, vec![edit.clone()]);
        let draw = restored
            .scene
            .draws()
            .into_iter()
            .find(|d| d.instance == edit.instance)
            .unwrap();
        assert_eq!(
            restored
                .scene
                .prototype(&draw.prototype)
                .unwrap()
                .get(edit.cell),
            0
        );
        assert_eq!(std::fs::read(legacy).unwrap(), b"legacy world sentinel");
        drop(restored);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// Save-candidate validation against the real source/collision and physics
/// contracts, on small fixtures (never the full showcase map).
#[cfg(test)]
mod journal_tests {
    use super::*;
    use matterweave_detail::{material, DetailVolume, Scale, Transform};

    fn fixture() -> DetailScene {
        // One shared prototype placed three times: the first accepted edit of any
        // instance mints a private prototype, which is exactly the state
        // cell-by-cell undo could not retract.
        let mut scene = DetailScene::new();
        let mut rock = DetailVolume::new("rock", Scale::new(1.0).unwrap());
        rock.set([0, 0, 0], material::BANK_STONE).unwrap();
        scene.add_prototype(rock).unwrap();
        scene.place("a", "rock", Transform::identity()).unwrap();
        scene
            .place(
                "c",
                "rock",
                Transform::new([20., 0., 0.], Yaw::Deg0).unwrap(),
            )
            .unwrap();
        scene
            .place(
                "b",
                "rock",
                Transform::new([10., 0., 0.], Yaw::Deg0).unwrap(),
            )
            .unwrap();
        scene
    }

    fn journal(edits: Vec<Edit>, bodies: Vec<BodySnapshot>, eye: [f32; 3]) -> SavedWetland {
        SavedWetland {
            version: 1,
            generator: 99,
            seed: 7,
            edits,
            physics: PhysicsSnapshot {
                version: 1,
                eye,
                bodies,
            },
            yaw: 0.,
            pitch: 0.,
            shadows: false,
        }
    }

    fn good_body() -> BodySnapshot {
        BodySnapshot {
            position: [0., 50., 0.],
            rotation: [0., 0., 0., 1.],
            velocity: [0.; 3],
            angular_velocity: [0.; 3],
            dimensions: [2; 3],
            material: 8,
        }
    }

    fn proto_of(scene: &DetailScene, instance: &str) -> String {
        scene
            .draws()
            .into_iter()
            .find(|d| d.instance == instance)
            .unwrap()
            .prototype
    }

    #[test]
    fn rejected_cell_undo_approach_leaks_a_private_prototype() {
        // RED record for the rejected approach: applying a valid edit and then
        // writing old cells back cannot retract the private prototype minted
        // by the first shared-prototype mutation, so the live scene keeps
        // extra identities, revisions and source accounting.
        let mut scene = fixture();
        let before = scene.prototype_ids();
        let before_counts = scene.counts();
        let old = scene.prototype("rock").unwrap().get([1, 0, 0]);
        scene
            .edit_instance("a", [1, 0, 0], material::BANK_STONE)
            .unwrap();
        scene.edit_instance("a", [1, 0, 0], old).unwrap();
        assert_ne!(
            scene.prototype_ids(),
            before,
            "legacy undo left the private prototype behind"
        );
        assert_ne!(scene.counts(), before_counts);
        assert_eq!(proto_of(&scene, "b"), "rock");
    }

    #[test]
    fn rejected_candidate_leaves_live_source_pristine() {
        // A shallow-valid journal edits a shared prototype, then fails when
        // the next shared edit collides with an existing private prototype ID.
        // It must leave identities, counts and cells untouched,
        // so a later valid recovery still sees the original accounting.
        let world = World::new(7);
        let mut scene = fixture();
        let mut probe = Physics::new(&world);
        let instances: BTreeSet<String> = scene.instance_ids().into_iter().collect();
        scene
            .add_prototype(DetailVolume::new(
                "__instance_edit:b",
                Scale::new(1.0).unwrap(),
            ))
            .unwrap();
        let before = scene.prototype_ids();
        let before_counts = scene.counts();
        let bad = journal(
            vec![
                Edit {
                    instance: "a".into(),
                    cell: [1, 0, 0],
                    material: material::BANK_STONE,
                },
                Edit {
                    instance: "b".into(),
                    cell: [2, 0, 0],
                    material: material::BANK_STONE,
                },
            ],
            vec![],
            [50., 50., 50.],
        );
        assert!(bad.validate(99, 7).is_ok(), "journal must be shallow-valid");
        let error = apply_journal(&mut scene, &mut probe, &instances, &bad, &world, [50.; 3])
            .err()
            .unwrap();
        assert!(
            error.contains("__instance_edit:b"),
            "unexpected error: {error}"
        );
        assert_eq!(scene.prototype_ids(), before);
        assert_eq!(scene.counts(), before_counts);
        assert_eq!(scene.prototype("rock").unwrap().get([1, 0, 0]), 0);
        // The same probe still validates a later valid journal: nothing
        // from the rejection accumulated in the physics used afterwards.
        let good = journal(
            vec![Edit {
                instance: "a".into(),
                cell: [2, 0, 0],
                material: material::BANK_STONE,
            }],
            vec![good_body()],
            [50., 50., 50.],
        );
        apply_journal(&mut scene, &mut probe, &instances, &good, &world, [50.; 3]).unwrap();
        assert_eq!(
            proto_of(&scene, "b"),
            "rock",
            "untouched instance kept identity"
        );
        assert_ne!(proto_of(&scene, "a"), "rock");
        assert_eq!(
            scene
                .prototype(&proto_of(&scene, "a"))
                .unwrap()
                .get([2, 0, 0]),
            material::BANK_STONE
        );
        assert_eq!(scene.prototype("rock").unwrap().get([2, 0, 0]), 0);
    }

    #[test]
    fn invalid_body_payload_is_rejected_by_the_physics_contract() {
        // `material: 0` passes shallow `validate` (bodies are not inspected
        // there) but the real `Physics::restore` rejects it; the scene and
        // the probe stay usable for the next candidate.
        let world = World::new(7);
        let mut scene = fixture();
        let mut probe = Physics::new(&world);
        let instances: BTreeSet<String> = scene.instance_ids().into_iter().collect();
        let before = scene.prototype_ids();
        let mut bad_body = good_body();
        bad_body.material = 0;
        let bad = journal(vec![], vec![bad_body], [50., 50., 50.]);
        assert!(bad.validate(99, 7).is_ok(), "journal must be shallow-valid");
        assert!(apply_journal(&mut scene, &mut probe, &instances, &bad, &world, [50.; 3]).is_err());
        assert_eq!(scene.prototype_ids(), before);
        let good = journal(vec![], vec![good_body()], [50., 50., 50.]);
        apply_journal(&mut scene, &mut probe, &instances, &good, &world, [50.; 3]).unwrap();
        assert_eq!(probe.snapshot().bodies.len(), 1);
        assert_eq!(probe.snapshot().eye, [50., 50., 50.]);
    }

    #[test]
    fn unrestorable_primary_falls_back_to_valid_recovery_intact() {
        // Actual file selection must discard a fork after a successful COW edit
        // followed by failure, then apply only the valid recovery's edit.
        let dir =
            std::env::temp_dir().join(format!("wetland-journal-select-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        let world = World::new(7);
        let mut scene = fixture();
        let mut probe = Physics::new(&world);
        let instances: BTreeSet<String> = scene.instance_ids().into_iter().collect();
        scene
            .add_prototype(DetailVolume::new(
                "__instance_edit:b",
                Scale::new(1.0).unwrap(),
            ))
            .unwrap();
        let before_prototypes = scene.prototype_ids().len();
        let path = dir.join(wetland_state::SAVE_FILE);
        let bad = journal(
            vec![
                Edit {
                    instance: "a".into(),
                    cell: [1, 0, 0],
                    material: material::BANK_STONE,
                },
                Edit {
                    instance: "b".into(),
                    cell: [2, 0, 0],
                    material: material::BANK_STONE,
                },
            ],
            vec![],
            [50., 50., 50.],
        );
        bad.save(&path).unwrap();
        let recovery = dir.join(format!("{}.recovery-5.json", wetland_state::SAVE_FILE));
        let good = journal(
            vec![Edit {
                instance: "a".into(),
                cell: [3, 0, 0],
                material: material::BANK_STONE,
            }],
            vec![],
            [50., 50., 50.],
        );
        good.save(&recovery).unwrap();
        let primary_bytes = std::fs::read(&path).unwrap();
        let recovery_bytes = std::fs::read(&recovery).unwrap();
        let (selected, saved) = SavedWetland::load_recovering_with(&path, 99, 7, |save| {
            apply_journal(&mut scene, &mut probe, &instances, save, &world, [50.; 3]).map(|_| ())
        })
        .unwrap();
        assert_eq!(selected, recovery);
        assert_eq!(scene.prototype_ids().len(), before_prototypes + 1);
        assert_eq!(proto_of(&scene, "b"), "rock");
        assert_eq!(
            scene
                .prototype(&proto_of(&scene, "a"))
                .unwrap()
                .get([1, 0, 0]),
            0
        );
        assert_eq!(saved.unwrap().edits.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), primary_bytes);
        assert_eq!(std::fs::read(&recovery).unwrap(), recovery_bytes);
        assert!(SavedWetland::load(&recovery, 99, 7).unwrap().is_some());
        assert_eq!(
            scene
                .prototype(&proto_of(&scene, "a"))
                .unwrap()
                .get([3, 0, 0]),
            material::BANK_STONE
        );
        std::fs::remove_file(&recovery).unwrap();
        let mut pristine = fixture();
        pristine
            .add_prototype(DetailVolume::new(
                "__instance_edit:b",
                Scale::new(1.0).unwrap(),
            ))
            .unwrap();
        let original_counts = pristine.counts();
        let original_ids = pristine.prototype_ids();
        let (fresh_path, saved) = SavedWetland::load_recovering_with(&path, 99, 7, |save| {
            apply_journal(
                &mut pristine,
                &mut probe,
                &instances,
                save,
                &world,
                [50.; 3],
            )
            .map(|_| ())
        })
        .unwrap();
        assert!(saved.is_none());
        assert_eq!(
            fresh_path,
            dir.join(format!("{}.recovery-1.json", wetland_state::SAVE_FILE))
        );
        assert_eq!(pristine.counts(), original_counts);
        assert_eq!(pristine.prototype_ids(), original_ids);
        assert_eq!(proto_of(&pristine, "a"), "rock");
        assert_eq!(std::fs::read(&path).unwrap(), primary_bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn journal_that_blocks_saved_and_entrance_pose_is_rejected() {
        let world = World::new(7);
        let mut scene = fixture();
        let mut probe = Physics::new(&world);
        let instances = scene.instance_ids().into_iter().collect();
        let before = scene.counts();
        let eye = [0.5, 2.7, 0.5];
        let bad = journal(
            (1..6)
                .map(|y| Edit {
                    instance: "a".into(),
                    cell: [0, y, 0],
                    material: material::BANK_STONE,
                })
                .collect(),
            vec![],
            eye,
        );
        assert!(apply_journal(&mut scene, &mut probe, &instances, &bad, &world, eye).is_err());
        assert_eq!(scene.counts(), before);
    }

    #[test]
    fn restored_respawn_is_clear_without_moving_the_saved_eye() {
        let world = World::new(7);
        let mut scene = fixture();
        let mut probe = Physics::new(&world);
        let instances = scene.instance_ids().into_iter().collect();
        let save = journal(vec![], vec![], [50.; 3]);
        let spawn = [0.5, 1.6, 0.5];
        let mut prepared =
            apply_journal(&mut scene, &mut probe, &instances, &save, &world, spawn).unwrap();
        assert_eq!(prepared.physics.character_eye(), save.physics.eye);
        assert_eq!(prepared.spawn[0], spawn[0]);
        assert_eq!(prepared.spawn[2], spawn[2]);
        assert!(prepared.spawn[1] > spawn[1] && prepared.spawn[1] <= spawn[1] + 1.);
        assert!(prepared.physics.teleport(prepared.spawn));
        for y in 1..6 {
            scene
                .edit_prototype("rock", [0, y, 0], material::BANK_STONE)
                .unwrap();
        }
        let prepared =
            apply_journal(&mut scene, &mut probe, &instances, &save, &world, spawn).unwrap();
        assert_eq!(prepared.spawn, save.physics.eye);
        assert_eq!(prepared.physics.character_eye(), save.physics.eye);
    }

    #[test]
    fn collision_budget_rejection_keeps_source_and_accepts_later_candidate() {
        let world = World::new(7);
        let mut scene = DetailScene::new();
        let mut rock = DetailVolume::new("rock", Scale::new(1.).unwrap());
        rock.set([0; 3], material::BANK_STONE).unwrap();
        scene.add_prototype(rock).unwrap();
        scene
            .add_prototype(DetailVolume::new("empty", Scale::new(1.).unwrap()))
            .unwrap();
        for index in 0..matterweave_physics::MAX_DETAIL_COLLIDERS {
            scene
                .place(
                    format!("rock-{index}"),
                    "rock",
                    Transform::new([100., 0., 0.], Yaw::Deg0).unwrap(),
                )
                .unwrap();
        }
        scene
            .place("trigger", "empty", Transform::identity())
            .unwrap();
        let mut probe = Physics::new(&world);
        let instances = scene.instance_ids().into_iter().collect();
        let before = scene.counts();
        let bad = journal(
            vec![Edit {
                instance: "trigger".into(),
                cell: [0; 3],
                material: material::BANK_STONE,
            }],
            vec![],
            [50.; 3],
        );
        let error = apply_journal(&mut scene, &mut probe, &instances, &bad, &world, [50.; 3])
            .err()
            .expect("one extra solid instance must exceed collision budget");
        assert!(error.contains("static colliders"), "{error}");
        assert_eq!(scene.counts(), before);
        let good = journal(vec![], vec![], [50.; 3]);
        let prepared =
            apply_journal(&mut scene, &mut probe, &instances, &good, &world, [50.; 3]).unwrap();
        assert_eq!(prepared.physics.character_eye(), [50.; 3]);
        assert_eq!(scene.prototype("empty").unwrap().get([0; 3]), 0);
    }

    #[test]
    fn buried_pose_is_lifted_but_clear_pose_is_kept() {
        // Real source colliders: a buried saved eye is lifted straight up
        // within the bounded step, while a valid grounded pose is preserved.
        let world = World::new(7);
        let mut scene = DetailScene::new();
        let mut floor = DetailVolume::new("floor", Scale::new(0.25).unwrap());
        for x in -8..8 {
            for z in -8..8 {
                floor.set([x, -1, z], material::BANK_STONE).unwrap();
            }
        }
        scene.add_prototype(floor).unwrap();
        scene
            .place("floor", "floor", Transform::identity())
            .unwrap();
        let mut physics = Physics::new(&world);
        physics.replace_detail_scene(&scene).unwrap();
        let standing = [0.125, 1.6, 0.125];
        assert!(
            physics.teleport(standing),
            "fixture standing pose must clear"
        );
        assert_eq!(clear_pose(&mut physics, standing), Some(standing));
        // A saved edit raises the floor half a metre under the player.
        for x in -2..2 {
            for z in -2..2 {
                scene
                    .edit_prototype("floor", [x, 0, z], material::BANK_STONE)
                    .unwrap();
                scene
                    .edit_prototype("floor", [x, 1, z], material::BANK_STONE)
                    .unwrap();
            }
        }
        physics.replace_detail_scene(&scene).unwrap();
        assert!(
            !physics.teleport(standing),
            "raised floor must bury the pose"
        );
        let corrected = clear_pose(&mut physics, standing).expect("bounded lift must clear");
        assert_eq!(corrected[0], standing[0]);
        assert_eq!(corrected[2], standing[2]);
        assert!(corrected[1] > standing[1] && corrected[1] - standing[1] <= 1.0);
        assert!(physics.teleport(corrected));
        // A tall solid column blocks every one of the nine tested positions.
        // A point below a thin floor would be clear, not a valid burial fixture.
        for x in -2..2 {
            for z in -2..2 {
                for y in 0..16 {
                    scene
                        .edit_prototype("floor", [x, y, z], material::BANK_STONE)
                        .unwrap();
                }
            }
        }
        physics.replace_detail_scene(&scene).unwrap();
        assert_eq!(clear_pose(&mut physics, standing), None);
    }

    fn rollback_fixture() -> DetailScene {
        let mut scene = DetailScene::new();
        let mut rock = DetailVolume::new("rock", Scale::new(1.0).unwrap());
        for x in 0..3 {
            rock.set([x, 0, 0], material::BANK_STONE).unwrap();
        }
        scene.add_prototype(rock).unwrap();
        scene.place("a", "rock", Transform::identity()).unwrap();
        scene
    }

    fn cell_of(scene: &DetailScene, instance: &str, cell: [i32; 3]) -> u8 {
        let proto = proto_of(scene, instance);
        scene.prototype(&proto).unwrap().get(cell)
    }

    #[test]
    fn pending_rollback_restores_confirmed_journal_removal() {
        // Lead-confirmed gap: a confirmed removal (journal material 0)
        // re-added by a failed burst must come back as a journal removal.
        // Restoring only the scene material deletes confirmed history and
        // corrupts save replay.
        let mut scene = rollback_fixture();
        scene.edit_instance("a", [0, 0, 0], 0).unwrap();
        let mut edits = vec![Edit {
            instance: "a".into(),
            cell: [0, 0, 0],
            material: 0,
        }];
        // Failed burst re-adds the cell; Runtime.edit updates the entry.
        scene
            .edit_instance("a", [0, 0, 0], material::BANK_STONE)
            .unwrap();
        edits[0].material = material::BANK_STONE;
        PendingEdit {
            instance: "a".into(),
            cell: [0, 0, 0],
            old: 0,
            journal: Some(0),
        }
        .rollback(&mut scene, &mut edits);
        assert_eq!(cell_of(&scene, "a", [0, 0, 0]), 0);
        assert_eq!(
            edits,
            vec![Edit {
                instance: "a".into(),
                cell: [0, 0, 0],
                material: 0,
            }],
            "confirmed removal history must survive the rollback"
        );
    }

    #[test]
    fn pending_rollback_drops_burst_created_journal_entry() {
        // No confirmed entry existed: rollback removes the burst's entry and
        // restores the scene cell.
        let mut scene = rollback_fixture();
        scene.edit_instance("a", [1, 0, 0], 0).unwrap();
        let mut edits = vec![Edit {
            instance: "a".into(),
            cell: [1, 0, 0],
            material: 0,
        }];
        PendingEdit {
            instance: "a".into(),
            cell: [1, 0, 0],
            old: material::BANK_STONE,
            journal: None,
        }
        .rollback(&mut scene, &mut edits);
        assert_eq!(cell_of(&scene, "a", [1, 0, 0]), material::BANK_STONE);
        assert!(edits.is_empty(), "burst entry removed: {edits:?}");
    }

    #[test]
    fn pending_rollback_reinserts_missing_journal_entry() {
        // Defensive arm: a confirmed entry absent at rollback time is
        // reinserted with its prior material, never left dropped.
        let mut scene = rollback_fixture();
        let mut edits = Vec::new();
        PendingEdit {
            instance: "a".into(),
            cell: [2, 0, 0],
            old: material::BANK_STONE,
            journal: Some(material::BANK_STONE),
        }
        .rollback(&mut scene, &mut edits);
        assert_eq!(
            edits,
            vec![Edit {
                instance: "a".into(),
                cell: [2, 0, 0],
                material: material::BANK_STONE,
            }]
        );
    }
}
