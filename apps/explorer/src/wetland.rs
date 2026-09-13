//! Player-facing wetland experience; source, collision, derived graphics and saves
//! remain separate. Legacy sandbox files are never used by this mode.
use crate::{
    audio_service::{EventQueue, GameplayEvent},
    controls::{
        action_at, contains, draw_settings_panel, settings_panel_click, wetland_layout, Action,
        Camera, Controls, SettingsPanelClick,
    },
    detail_runtime::DetailRuntime,
    metrics,
    settings::SharedSettings,
    wetland_metrics::Capture,
    wetland_replay::{Replay, Route},
    wetland_state::{self, Edit, SavedWetland},
};
use glam::{Vec2, Vec3};
use matterweave_core::{Mesh, World};
#[cfg(test)]
use matterweave_detail::Yaw;
use matterweave_detail::{
    Camera as LodCamera, DetailScene, Lod, LodConfig, Projection, SceneVersion,
};
use matterweave_pacing::{
    Config as PacingConfig, FrameSample, Pacer, Policy as PacingPolicy, DEFAULT_SETTLE_FRAMES,
    MAX_DIVISOR,
};
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
/// Display period assumed when the platform reports no usable refresh rate.
/// 60 Hz is the conservative floor for the declared Android profile.
const FALLBACK_REFRESH_PERIOD_NS: u64 = 16_666_667;
/// Idle cadence for the menu. The menu is static, so it is paced well below the
/// display rather than adaptively: there is no frame cost worth measuring there.
const MENU_INTERVAL_NS: u64 = 66_666_667;
/// How often the frame path rechecks the display's reported refresh period. A
/// variable-refresh (LTPO) panel or a window moved between monitors otherwise
/// leaves the pacer recommending multiples of the wrong period. Only a material
/// change re-arms, because re-arming clears the history.
const REFRESH_RECHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Duration to nanoseconds, saturating. A session would have to run for roughly
/// 584 years to reach the cap.
fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Display period reported by `window`'s current monitor, or the conservative
/// fallback when the platform reports nothing usable.
fn display_period_ns(window: &Window) -> u64 {
    window
        .current_monitor()
        .and_then(|monitor| monitor.refresh_rate_millihertz())
        .filter(|rate| *rate > 0)
        // Millihertz to nanosecond period: 1e12 / rate. A non-zero u32 rate always
        // yields a non-zero period.
        .map_or(FALLBACK_REFRESH_PERIOD_NS, |rate| {
            1_000_000_000_000 / u64::from(rate)
        })
}

/// Whether a newly reported display period differs materially from the one in force.
/// A refresh-rate report carries no useful sub-percent information for pacing, and
/// re-arming clears the history, so only a change large enough to move a cadence
/// boundary — more than one percent — counts.
fn period_changed_materially(current_ns: u64, reported_ns: u64) -> bool {
    let larger = current_ns.max(reported_ns);
    current_ns.abs_diff(reported_ns).saturating_mul(100) > larger
}

struct Runtime {
    scene: DetailScene,
    /// Resident derived geometry and the per-frame camera-driven selection;
    /// the renderer indexes the mesh pool it owns.
    detail: WetlandDetail,
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

/// Vertical field of view of the wetland render projection. It must describe
/// the same perspective as `Camera::view_projection` in `controls.rs`; the
/// `lod_camera_matches_the_render_projection` test fails if the two drift apart.
const WETLAND_FOV_RAD: f32 = 65.0 * std::f32::consts::PI / 180.0;
/// Near-plane distance of the wetland render projection (metres).
const WETLAND_NEAR_M: f32 = 0.1;
/// Per-prepare cap on newly realized coarse levels. An instance whose coarse
/// level is capped resolves to its authoritative `Source` mesh instead, so a
/// frame's coarsening work — and the full static-scene install a new resident
/// level requires — stays bounded.
const MAX_COARSE_BUILDS_PER_PREPARE: usize = 2;

/// The LOD camera for one frame, built from the camera the render pass uses.
/// The engine projects its error estimates through this view, so it must see
/// the real viewport height in the same physical pixels the window reports and
/// the same perspective `Camera::view_projection` draws with.
fn lod_camera(camera: &Camera, viewport_height_px: f32) -> LodCamera {
    LodCamera {
        eye_m: camera.position.to_array(),
        forward_m: camera.forward().to_array(),
        viewport_height_px,
        near_m: WETLAND_NEAR_M,
        projection: Projection::Perspective {
            vertical_fov_rad: WETLAND_FOV_RAD,
        },
    }
}

/// Install work the current selection requires from the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticUpdate {
    /// Resident geometry and packed instances already match the renderer.
    Current,
    /// Resident geometry was added or revised (or the surface is fresh): the
    /// renderer must replace its whole static scene before it can draw the new
    /// selection.
    Replace,
    /// Geometry is unchanged; only the packed placements moved.
    Instances,
}

/// Production automatic-detail state for the wetland frame path.
///
/// [`DetailRuntime`] owns the resident derived geometry pool. This wrapper adds
/// the bounded selection policy, remembers the exact camera and source version
/// the current selection was prepared from (so an idle frame does not walk
/// every instance again), and tracks what the renderer accepted so a frame with
/// unchanged placements does no GPU work.
struct WetlandDetail {
    runtime: DetailRuntime,
    config: LodConfig,
    /// View the current `instances` were prepared for.
    prepared_camera: Option<LodCamera>,
    /// Source version the current `instances` were prepared from.
    prepared_version: Option<SceneVersion>,
    /// Selection to draw, referencing [`Self::runtime`]'s mesh pool.
    instances: Vec<StaticInstance>,
    /// Instances the renderer accepted at the last successful install.
    installed: Vec<StaticInstance>,
    /// The renderer needs a full static-scene replace before the next instance
    /// update: set on surface (re)creation, when resident geometry changed, and
    /// after a failed install; cleared only by a successful replace.
    reinstall: bool,
    /// Instances the last prepare degraded to `Source` because its cap or a
    /// budget could not realize the requested level.
    deferred: usize,
    /// True while a stationary re-prepare is still expected to shrink
    /// `deferred`; cleared when it makes no progress.
    pending: bool,
}

impl WetlandDetail {
    fn new() -> Self {
        Self {
            runtime: DetailRuntime::new(),
            config: LodConfig {
                max_coarse_builds: Some(MAX_COARSE_BUILDS_PER_PREPARE),
                ..LodConfig::default()
            },
            prepared_camera: None,
            prepared_version: None,
            instances: Vec::new(),
            installed: Vec::new(),
            // No renderer has accepted geometry yet.
            reinstall: true,
            deferred: 0,
            pending: false,
        }
    }

    /// Realizes the authoritative `Source` geometry of the whole scene into the
    /// resident pool once, off the frame path. This preserves the previous
    /// wetland behavior (every Source mesh valid and ready before the first
    /// world frame, and an unbuildable save rejected during recovery) and keeps
    /// a capped coarse selection cheap: it falls back to pool-resident `Source`
    /// instead of acquiring geometry during play. Coarse levels stay lazy.
    fn warm_source(&mut self, scene: &mut DetailScene, camera: &Camera) -> Result<(), String> {
        let source_only = LodConfig {
            max_lod: Lod::Source,
            ..self.config
        };
        self.runtime
            .prepare(scene, &lod_camera(camera, 1.0), &source_only)?;
        Ok(())
    }

    /// The surface was created or recreated: no renderer-resident geometry
    /// survives, so the next frame must fully replace the static scene even
    /// when the selection itself has not moved.
    fn rearm(&mut self) {
        self.reinstall = true;
    }

    /// Prepare this frame's camera-driven selection when the view or the
    /// authoritative source moved, then report what the renderer needs. An
    /// unchanged pair reuses the resident selection: no mesh is realized,
    /// copied into the pool or re-uploaded while nothing moved.
    ///
    /// `max_coarse_builds` caps one prepare, so a stationary camera can still
    /// have requested coarse levels outstanding; a repeat prepare runs while
    /// the deferred set shrinks and stops once it is empty or stops making
    /// progress (out-of-range scale or the mesh budget cannot be fixed by
    /// waiting).
    fn update(
        &mut self,
        scene: &mut DetailScene,
        camera: &Camera,
        viewport_height_px: f32,
    ) -> Result<StaticUpdate, String> {
        let view = lod_camera(camera, viewport_height_px);
        let version = scene.source_version();
        let repeated =
            self.prepared_camera == Some(view) && self.prepared_version.as_ref() == Some(&version);
        if !repeated || self.pending {
            let frame = self.runtime.prepare(scene, &view, &self.config)?;
            let deferred = frame
                .frame
                .selected
                .iter()
                .filter(|item| item.fallback)
                .count();
            // A capped prepare defers the coarse levels it could not realize.
            // Keep preparing while a stationary repeat shrinks that deferred
            // set; a repeat that does not shrink it proves those instances are
            // held at `Source` by something the cap cannot fix, so stop
            // repeating rather than preparing every idle frame forever.
            self.pending = deferred > 0 && (!repeated || deferred < self.deferred);
            self.deferred = deferred;
            if frame.geometry_changed {
                self.reinstall = true;
                let counts = crate::detail_runtime::lod_histogram(&frame.frame);
                log::info!(
                    "Wetland detail geometry refreshed: source={} half={} quarter={} deferred={} resident_meshes={}",
                    counts.get(&Lod::Source).copied().unwrap_or(0),
                    counts.get(&Lod::Half).copied().unwrap_or(0),
                    counts.get(&Lod::Quarter).copied().unwrap_or(0),
                    deferred,
                    self.runtime.meshes().len(),
                );
            }
            self.instances = frame.instances;
            self.prepared_camera = Some(view);
            self.prepared_version = Some(version);
        }
        Ok(self.install_update())
    }

    /// What the renderer must do for the current selection.
    fn install_update(&self) -> StaticUpdate {
        if self.reinstall {
            StaticUpdate::Replace
        } else if self.instances != self.installed {
            StaticUpdate::Instances
        } else {
            StaticUpdate::Current
        }
    }

    /// Record that the renderer accepted the current selection.
    fn mark_installed(&mut self) {
        self.reinstall = false;
        self.installed.clone_from(&self.instances);
    }

    /// A failed install leaves the renderer on its previous scene. The next
    /// frame retries with a full replace so an instance-only update can never
    /// reference geometry the renderer does not hold.
    fn mark_install_failed(&mut self) {
        self.reinstall = true;
    }

    /// Resident derived mesh pool; instance prototype indices address this.
    fn meshes(&self) -> &[Mesh] {
        self.runtime.meshes()
    }

    /// The current packed selection.
    fn instances(&self) -> &[StaticInstance] {
        &self.instances
    }
}

/// Realizes the authoritative `Source` mesh of every prototype, preserving the
/// load-time drawability validation the previous whole-scene graphics build
/// performed. [`WetlandDetail`] owns the pool and realizes only the levels the
/// camera selects; this validates a candidate source and warms the engine's
/// derived-mesh cache without cloning any of it.
fn validate_source(scene: &mut DetailScene) -> Result<(), String> {
    for id in scene.prototype_ids() {
        scene
            .prototype_mesh(&id, Lod::Source)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

struct PreparedJournal {
    spawn: [f32; 3],
    physics: Physics,
    collision: DetailCollisionStats,
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
    validate_source(&mut fork)?;
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
        } = match restored {
            Some(value) => value,
            None => {
                let mut physics = Physics::new(&empty_world);
                let collision = physics.replace_detail_scene(&scene)?;
                validate_source(&mut scene)?;
                PreparedJournal {
                    spawn,
                    physics,
                    collision,
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
        let mut detail = WetlandDetail::new();
        detail.warm_source(&mut scene, &camera)?;
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
            detail,
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
            &mut self.scene,
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
        // Realize the edited prototype's authoritative Source before the edit
        // is confirmed. This replaces the previous whole-scene graphics rebuild
        // (which copied every prototype's Source vertices on every edit): only
        // the edited prototype is realized here, and the frame path refreshes
        // exactly the selected levels it draws. An edit whose source the
        // derived-mesh budget cannot serve is rolled back as before.
        let edited = self
            .scene
            .draws()
            .into_iter()
            .find(|draw| draw.instance == hit.instance)
            .ok_or("Object is no longer present")?;
        if let Err(error) = self.scene.prototype_mesh(&edited.prototype, Lod::Source) {
            self.scene
                .edit_instance(&hit.instance, cell, old)
                .map_err(|e| e.to_string())?;
            return Err(error.to_string());
        }
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
                // The reverted source carries a new revision, so the next
                // frame's detail update refreshes the selected levels and
                // reinstalls them. Nothing is forced here: the previous derived
                // geometry stays up rather than blanking.
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
    /// Physics reach distance for the current view: the nearest source hit
    /// along the camera ray, or the full six-metre range on a miss. One ray,
    /// shared by grab and break; throw needs no source query at all.
    fn reach(&mut self) -> f32 {
        wetland_state::raycast(
            &mut self.scene,
            self.camera.position.to_array(),
            self.camera.forward().to_array(),
            6.,
        )
        .map_or(6., |h| h.distance)
    }
    fn playground(&mut self) -> Result<(), String> {
        // A bounded six-body arch containing exactly64 half-metre voxels. The
        // nearest designated route clearing supplies the ground, never a visual LOD.
        let centre = self.clearing;
        let hit = wetland_state::raycast(
            &mut self.scene,
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

/// Queue the audio event for one completed physics step, from its real ground
/// transitions.
///
/// A takeoff is only a takeoff when a requested jump actually left the ground, and
/// a landing only when this step changed airborne to grounded. Both are transitions,
/// so a held key or a standing frame queues nothing.
fn queue_locomotion(
    events: &mut EventQueue,
    grounded_before: bool,
    grounded_after: bool,
    jump_requested: bool,
) {
    if jump_requested && grounded_before && !grounded_after {
        events.push(GameplayEvent::Jump);
    }
    if !grounded_before && grounded_after {
        events.push(GameplayEvent::Land { strength: 1.0 });
    }
}

/// Queue the event for one attempted authoritative edit.
///
/// Only an accepted edit is a world change worth hearing; every rejection path
/// (`Err`) is status text and queues nothing.
fn queue_edit_result(events: &mut EventQueue, result: &Result<String, String>) {
    if result.is_ok() {
        events.push(GameplayEvent::BlockEdit);
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
    /// The shared preferences this sample renders and edits. The owner
    /// (`Experience`) persists changes and applies the audio policy; this
    /// sample never opens a device or a settings file itself.
    settings: SharedSettings,
    /// One-shot flag set when the settings panel changed a preference.
    settings_dirty: bool,
    /// True while the shared settings overlay is open. Modal: the gameplay
    /// zones and action buttons are inactive until it closes.
    settings_open: bool,
    pub sandbox_requested: bool,
    pub voxel_relay_requested: bool,
    pub terrain_lab_requested: bool,
    pub failed: bool,
    status: String,
    last: Instant,
    next_frame: Instant,
    /// Monotonic base for the pacer's nanosecond timestamps. Re-armed whenever the
    /// pacer is rebuilt, so a long-lived session cannot overflow its `u64`.
    frame_epoch: Instant,
    /// When the display's reported refresh period was last checked on the frame path.
    refresh_checked_at: Instant,
    /// Engine frame-loop scheduling. Decides the present cadence from measured
    /// frame production cost; the app owns the clock and the waiting.
    pacer: Pacer,
    focused: bool,
    frames: u64,
    frame_limit: Option<u64>,
    auto_start: bool,
    fps: f32,
    saved_at: Instant,
    replay_checked: bool,
    replay: Option<Replay>,
    /// Gameplay feedback since the last drain by the shared audio owner. Private:
    /// the vocabulary is the accessors, not the field.
    events: EventQueue,
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
            settings: SharedSettings::default(),
            settings_dirty: false,
            settings_open: false,
            sandbox_requested: false,
            voxel_relay_requested: false,
            terrain_lab_requested: false,
            failed: false,
            status: String::new(),
            last: Instant::now(),
            next_frame: Instant::now(),
            frame_epoch: Instant::now(),
            refresh_checked_at: Instant::now(),
            // The app opens in the menu; `resumed` rebuilds this for the real display.
            pacer: Pacer::new(
                PacingConfig::fixed(FALLBACK_REFRESH_PERIOD_NS, MENU_INTERVAL_NS)
                    .expect("constant fallback pacing configuration is valid"),
            ),
            focused: true,
            frames: 0,
            frame_limit,
            auto_start,
            fps: 0.,
            saved_at: Instant::now(),
            replay_checked: false,
            replay: None,
            events: EventQueue::default(),
        }
    }
    /// Remove the oldest queued gameplay event, if any.
    ///
    /// The app's shared audio owner drains this once per frame; nothing else does.
    pub(crate) fn pop_event(&mut self) -> Option<GameplayEvent> {
        self.events.pop()
    }
    /// Drop every queued gameplay event without playing it.
    ///
    /// The audio owner calls this on focus loss, suspension and scope switches.
    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }
    /// Install the owner's shared preferences. Called when the sample is
    /// constructed or switched, never a user change.
    pub fn set_shared_settings(&mut self, settings: SharedSettings) {
        self.settings = settings;
        self.settings_dirty = false;
    }
    /// The one-shot preference change the settings panel queued, if any.
    pub fn take_settings_change(&mut self) -> Option<SharedSettings> {
        if self.settings_dirty {
            self.settings_dirty = false;
            Some(self.settings)
        } else {
            None
        }
    }
    fn open_settings(&mut self) {
        self.settings_open = true;
        self.controls.clear();
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
        if self.settings_open {
            match settings_panel_click(&mut self.settings, p) {
                SettingsPanelClick::Toggled => {
                    // A preference change re-derives the layout: drop held touches
                    // so they cannot activate zones from the previous one.
                    self.settings_dirty = true;
                    self.controls.clear();
                }
                SettingsPanelClick::Close => {
                    self.settings_open = false;
                    self.controls.clear();
                }
                SettingsPanelClick::Outside => {}
            }
            return true;
        }
        if self.menu {
            if contains([880., 20., 100., 42.], p) {
                self.open_settings();
                return true;
            }
            if self.loading.is_none() && contains([70., 290., 420., 72.], p) {
                self.enter();
            }
            if self.loading.is_none() && contains([70., 382., 420., 50.], p) {
                self.sandbox_requested = true;
            }
            if self.loading.is_none() && contains([70., 440., 420., 50.], p) {
                self.voxel_relay_requested = true;
            }
            if self.loading.is_none() && contains([70., 498., 420., 50.], p) {
                self.terrain_lab_requested = true;
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
                "SETTINGS",
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
                        4 => self.open_settings(),
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
        if let Some(action) = action_at(&wetland_layout(&self.settings), p) {
            self.action(action);
            return true;
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
        match action {
            Action::Remove | Action::Place => {
                // `edit` performs the single action ray itself; no extra ray
                // is spent here just to compute an unused range.
                let t = Instant::now();
                let result = r.edit(action == Action::Place);
                queue_edit_result(&mut self.events, &result);
                self.status = match result {
                    Ok(message) | Err(message) => message,
                };
                log::info!(
                    "WETLAND EDIT {:.3}ms {}",
                    t.elapsed().as_secs_f64() * 1000.,
                    self.status
                );
            }
            Action::Grab => {
                let range = r.reach();
                let eye = r.camera.position.to_array();
                let forward = r.camera.forward().to_array();
                r.physics.grab(&r.empty_world, eye, forward, range);
                r.dirty = true;
            }
            Action::Throw => {
                // No ray: throwing releases a held body along the view.
                r.physics.throw(r.camera.forward().to_array());
                r.dirty = true;
            }
            Action::Break => {
                let range = r.reach();
                let eye = r.camera.position.to_array();
                let forward = r.camera.forward().to_array();
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
            h.rect([70., 382., 420., 50.], panel);
            h.text(96., 400., "OPEN YOUR SANDBOX", 1.4, ink);
            h.rect([70., 440., 420., 50.], panel);
            h.text(96., 458., "PLAY VOXEL RELAY", 1.4, ink);
            h.rect([70., 498., 420., 50.], panel);
            h.text(96., 516., "EXPLORE TERRAIN LAB", 1.4, ink);
            h.rect([880., 20., 100., 42.], panel);
            h.text(888., 34., "SETTINGS", 1.1, ink);
            h.text(74., 562., "An original alien wetland", 1.25, gold);
            h.text(
                74.,
                586.,
                &self.status.chars().take(88).collect::<String>(),
                1.,
                ink,
            );
            if self.settings_open {
                draw_settings_panel(&mut h, &self.settings);
            }
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
        let layout = wetland_layout(&self.settings);
        h.rect(layout.move_zone, [0.08, 0.14, 0.14, 0.5]);
        h.text(
            layout.move_zone[0] + 40.,
            layout.move_zone[1] + 65.,
            "MOVE",
            1.3,
            ink,
        );
        h.rect(layout.jump_zone, panel);
        h.text(
            layout.jump_zone[0] + 8.,
            layout.jump_zone[1] + 20.,
            "JUMP",
            1.1,
            ink,
        );
        for (rect, text, _) in layout.actions {
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
                "SETTINGS",
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
        if self.settings_open {
            draw_settings_panel(&mut h, &self.settings);
        }
        h
    }
    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if !self.focused && self.frame_limit.is_none() {
            return;
        }
        self.recheck_pacing();
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
        // Menu and world are different scheduling problems. Switching clears the
        // history, so idle-cap intervals are never read as interactive frame cost.
        let policy = self.pacing_policy();
        if let Err(e) = self.pacer.set_policy(policy) {
            log::error!("Wetland pacing policy rejected: {e}");
        }
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
                let grounded_before = r.physics.grounded();
                let jump_requested = !replay_active && motion.y > 0.;
                row.physics_fixed_steps =
                    Some(r.physics.step(dt, velocity.to_array(), jump_requested) as u32);
                let grounded_after = r.physics.grounded();
                // Real transition only: a blocked or held jump queues nothing, and
                // one step can queue at most one takeoff and one landing.
                queue_locomotion(
                    &mut self.events,
                    grounded_before,
                    grounded_after,
                    jump_requested,
                );
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
            // Camera-driven detail: the merged runtime owns the resident derived
            // geometry, selects a level per instance for this frame's view and
            // reports whether the renderer needs new geometry or only new
            // placements. An unchanged view reprepares nothing.
            match r.detail.update(&mut r.scene, &r.camera, size.height as f32) {
                Ok(StaticUpdate::Current) => {}
                Ok(update) => {
                    let installed = match update {
                        StaticUpdate::Replace => renderer
                            .replace_static_scene(r.detail.meshes(), r.detail.instances())
                            .map(|_| ()),
                        StaticUpdate::Instances => renderer
                            .update_static_instances(r.detail.instances())
                            .map(|_| ()),
                        StaticUpdate::Current => Ok(()),
                    };
                    match installed {
                        Ok(()) => r.detail.mark_installed(),
                        Err(error) => {
                            // Keep the last accepted static scene on screen and
                            // retry a full replace next frame rather than
                            // failing the session or drawing stale placements.
                            r.detail.mark_install_failed();
                            self.status = error;
                            log::error!("Wetland detail install failed: {}", self.status);
                        }
                    }
                }
                Err(error) => {
                    self.status = format!("Detail update failed: {error}");
                    log::error!("Wetland detail update failed: {error}");
                }
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
        let presented = matches!(&result, FrameResult::Presented);
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
        // Frame-loop scheduling. The pacer is given what this frame actually cost to
        // produce, never the interval it was paced to: a paced interval is the pacer's
        // own output plus this loop's wait overshoot, and feeding that back makes every
        // overshoot look like load and ratchets the cadence down until it sticks.
        let decision = if presented {
            let finished = Instant::now();
            self.pacer.observe(FrameSample {
                present_ns: nanos(finished.saturating_duration_since(self.frame_epoch)),
                work_ns: nanos(finished.saturating_duration_since(capture_start)),
            })
        } else {
            // A retried or failed attempt presented nothing, so it is not a frame
            // boundary and must not enter the statistics.
            self.pacer.decision()
        };
        self.next_frame = capture_start + Duration::from_nanos(decision.interval_ns);
    }
    /// Pacing policy for the current mode. The menu holds a low idle cadence; the
    /// world adapts to what its frames actually cost.
    fn pacing_policy(&self) -> PacingPolicy {
        if self.menu {
            PacingPolicy::Fixed {
                interval_ns: MENU_INTERVAL_NS,
            }
        } else {
            PacingPolicy::Adaptive {
                max_divisor: MAX_DIVISOR,
                settle_frames: DEFAULT_SETTLE_FRAMES,
                margin_ns: 0,
            }
        }
    }
    /// Rebuild the pacer for `window`'s display and re-arm the timestamp base.
    /// Called whenever the window and renderer are created, because a recreated
    /// surface can land on a different display with a different refresh rate.
    fn rearm_pacing(&mut self, window: &Window) {
        let refresh_period_ns = display_period_ns(window);
        let config = PacingConfig::new(refresh_period_ns, self.pacing_policy())
            .expect("a non-zero display period and a static policy are always valid");
        log::info!(
            "Wetland pacing: {:.3} ms display period",
            config.refresh_period_ns() as f64 / 1e6
        );
        self.pacer = Pacer::new(config);
        self.frame_epoch = Instant::now();
        self.refresh_checked_at = Instant::now();
    }
    /// Re-arm the pacer when the display's reported refresh period has materially
    /// changed since the last check.
    ///
    /// `rearm_pacing` otherwise runs only from `resumed`, so a variable-refresh panel
    /// or a window moved between monitors leaves the pacer recommending multiples of a
    /// stale period and counting missed deadlines against the wrong reference. The
    /// monitor query costs a platform call, so the frame path runs this at most about
    /// once a second, and only a material change re-arms: re-arming clears the history,
    /// which is correct for a genuine display change but not for jitter in the report.
    fn recheck_pacing(&mut self) {
        if self.refresh_checked_at.elapsed() < REFRESH_RECHECK_INTERVAL {
            return;
        }
        self.refresh_checked_at = Instant::now();
        let Some(window) = self.window.clone() else {
            return;
        };
        let reported_ns = display_period_ns(&window);
        let current_ns = self.pacer.config().refresh_period_ns();
        if !period_changed_materially(current_ns, reported_ns) {
            return;
        }
        log::info!(
            "Wetland pacing: display period changed {:.3} ms -> {:.3} ms; re-arming",
            current_ns as f64 / 1e6,
            reported_ns as f64 / 1e6
        );
        self.rearm_pacing(&window);
    }
    fn point(&self, x: f64, y: f64) -> Vec2 {
        let s = self.window.as_ref().unwrap().inner_size();
        crate::controls::virtual_point(x, y, s.width, s.height)
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
                self.rearm_pacing(&window);
                self.window = Some(window);
                if let Some(r) = &mut self.runtime {
                    r.detail.rearm();
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
        // Timestamps taken before a suspension say nothing about the frames after it.
        self.pacer.reset();
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
                let was_focused = self.focused;
                self.focused = f;
                self.last = Instant::now();
                // The loop stops drawing while unfocused only when no frame limit bounds
                // the run (`draw` returns early only for `!focused && frame_limit.is_none()`).
                // A bounded headless run keeps presenting without focus, so a stray focus
                // event must not wipe the rings and adaptive state mid-flight; only a run
                // whose loop really stops has a gap that is not a frame interval.
                if was_focused != f && self.frame_limit.is_none() {
                    self.pacer.reset();
                }
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
                    if self.settings_open {
                        self.settings_open = false;
                        self.controls.clear();
                    } else if self.menu {
                        event_loop.exit();
                    } else {
                        self.save();
                        self.menu = true;
                        self.controls.clear();
                    }
                    return;
                }
                if self.settings_open {
                    // The modal settings overlay owns input: gameplay keys are
                    // ignored until it closes.
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
                            self.controls
                                .start_wetland(&wetland_layout(&self.settings), t.id, p);
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
        assert_eq!(app.pop_event(), None, "a rejected action queued audio");
        // The same real action with the menu closed performs the edit and emits
        // exactly one block-edit event through the app's bounded queue.
        app.menu = false;
        app.runtime = Some(runtime);
        app.action(Action::Remove);
        runtime = app.runtime.take().unwrap();
        assert_eq!(runtime.scene.counts().expanded_occupied_cells, before - 1);
        assert_eq!(app.pop_event(), Some(GameplayEvent::BlockEdit));
        assert_eq!(app.pop_event(), None);
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

    /// A real `Runtime` on a one-prototype fixture, without generating and meshing
    /// the full showcase map. The edit path (`Runtime::edit`, the graphics rebuild
    /// and the collision cadence) is the production one; only the world is tiny.
    fn fixture_runtime(directory: &std::path::Path) -> Runtime {
        use matterweave_detail::{material, DetailVolume, Scale, Transform};

        let empty_world = World::new(7);
        let mut scene = DetailScene::new();
        let mut rock = DetailVolume::new("test-rock-volume", Scale::new(0.25).unwrap());
        rock.set([0, 0, 0], material::BANK_STONE).unwrap();
        scene.add_prototype(rock).unwrap();
        scene
            .place("test-rock", "test-rock-volume", Transform::identity())
            .unwrap();
        let camera = Camera {
            position: Vec3::new(0.125, 0.125, 2.0),
            yaw: std::f32::consts::PI,
            pitch: 0.0,
        };
        let mut detail = WetlandDetail::new();
        detail.warm_source(&mut scene, &camera).unwrap();
        let mut physics = Physics::new(&empty_world);
        physics.replace_detail_scene(&scene).unwrap();
        Runtime {
            scene,
            detail,
            physics,
            empty_world,
            dynamic: DynamicMeshCache::default(),
            edits: Vec::new(),
            save_path: directory.join("test-wetland.json"),
            spawn: [0.125, 0.125, 2.0],
            terrain: matterweave_detail::Terrain::generate(SEED).expect("showcase terrain grid"),
            clearing: [0.125, 0.125, 0.0],
            route: Vec::new(),
            elevated_route: Vec::new(),
            camera,
            lighting: LightingSettings::default(),
            dirty: false,
            dynamic_dirty: false,
            counts: String::new(),
            collision: DetailCollisionCadence::new(),
            pending_edits: Vec::new(),
        }
    }

    #[test]
    fn real_edit_queues_one_block_edit_and_rejections_queue_nothing() {
        let directory =
            std::env::temp_dir().join(format!("matterweave-wetland-events-{}", std::process::id()));
        let mut app = WetlandApp::new(directory.clone(), false, None);
        app.runtime = Some(fixture_runtime(&directory));

        // The menu rejects the action before any world work: no event, no edit.
        app.action(Action::Remove);
        assert!(app.runtime.as_ref().unwrap().edits.is_empty());
        assert_eq!(app.pop_event(), None);

        // The same aimed action with the menu closed performs the real edit and
        // queues exactly one block-edit event.
        app.menu = false;
        app.action(Action::Remove);
        assert_eq!(app.runtime.as_ref().unwrap().edits.len(), 1);
        assert_eq!(app.pop_event(), Some(GameplayEvent::BlockEdit));
        assert_eq!(app.pop_event(), None);

        // Aiming at nothing is rejected: status only, no event.
        app.runtime.as_mut().unwrap().camera.pitch = 1.0;
        app.action(Action::Remove);
        assert_eq!(app.runtime.as_ref().unwrap().edits.len(), 1);
        assert_eq!(app.pop_event(), None);
    }

    #[test]
    fn locomotion_events_follow_real_ground_transitions_only() {
        let mut events = EventQueue::default();
        // Standing and walking keep the grounded state, and a blocked jump request
        // never leaves the ground: nothing to hear.
        queue_locomotion(&mut events, true, true, false);
        queue_locomotion(&mut events, true, true, true);
        assert_eq!(events.pop(), None);
        // A real takeoff queues exactly one jump.
        queue_locomotion(&mut events, true, false, true);
        assert_eq!(events.pop(), Some(GameplayEvent::Jump));
        assert_eq!(events.pop(), None);
        // Staying airborne repeats nothing, however long the fall.
        queue_locomotion(&mut events, false, false, true);
        assert_eq!(events.pop(), None);
        // The step that touches down queues exactly one landing.
        queue_locomotion(&mut events, false, true, false);
        assert_eq!(events.pop(), Some(GameplayEvent::Land { strength: 1.0 }));
        assert_eq!(events.pop(), None);
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

/// Display-period recheck constants and threshold. The monitor query itself needs a
/// window, so only the pure decision is unit testable here.
#[cfg(test)]
mod pacing_tests {
    use super::period_changed_materially;

    /// Only a change large enough to move a cadence boundary may clear the pacing
    /// history; millisecond-level jitter in the platform's millihertz report may not.
    #[test]
    fn display_period_change_is_material_only_above_one_percent() {
        assert!(!period_changed_materially(8_333_333, 8_333_333));
        // 120 Hz reported a little low: under one percent.
        assert!(!period_changed_materially(8_333_333, 8_400_000));
        // 60 Hz <-> 120 Hz, and 120 Hz -> 90 Hz, are genuine display changes.
        assert!(period_changed_materially(8_333_333, 16_666_667));
        assert!(period_changed_materially(16_666_667, 11_111_111));
    }
}

/// Production detail path: the same [`WetlandDetail`] wrapper the frame path
/// drives, backed by the real [`DetailRuntime`] and real source edits, on small
/// fixtures (never the full showcase map).
#[cfg(test)]
mod detail_tests {
    use super::*;
    use matterweave_detail::{material, DetailVolume, Scale, Transform, SCALE_FINE_M};

    const VIEWPORT_PX: f32 = 1080.0;

    fn dense(id: &str, edge: i32) -> DetailVolume {
        let mut volume = DetailVolume::new(id, Scale::new(SCALE_FINE_M).unwrap());
        for x in 0..edge {
            for y in 0..edge {
                for z in 0..edge {
                    volume.set([x, y, z], material::BANK_STONE).unwrap();
                }
            }
        }
        volume
    }

    fn boulder_scene() -> DetailScene {
        let mut scene = DetailScene::new();
        scene.add_prototype(dense("boulder", 8)).unwrap();
        scene
            .place("near", "boulder", Transform::identity())
            .unwrap();
        scene
    }

    fn boulder_and_keeper_scene() -> DetailScene {
        let mut scene = boulder_scene();
        scene.add_prototype(dense("keeper", 4)).unwrap();
        scene
            .place(
                "keeper_placed",
                "keeper",
                Transform::new([6.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
            )
            .unwrap();
        scene
    }

    /// App camera at `position` looking along -Z (yaw pi), as the wetland route does.
    fn looking_at_origin(position: Vec3) -> Camera {
        Camera {
            position,
            yaw: std::f32::consts::PI,
            pitch: 0.0,
        }
    }

    fn selected_lod(detail: &WetlandDetail, prototype: &str, packed: &StaticInstance) -> Lod {
        [Lod::Source, Lod::Half, Lod::Quarter]
            .into_iter()
            .find(|lod| detail.runtime.instance_index(prototype, *lod) == Some(packed.prototype))
            .expect("published instance must reference a resident level")
    }

    #[test]
    fn production_selection_tracks_the_view_without_mutating_source() {
        let mut scene = boulder_scene();
        let version = scene.source_version();
        let revision = scene.prototype("boulder").unwrap().revision();
        let counts = scene.counts();

        let mut detail = WetlandDetail::new();
        let near = detail
            .update(
                &mut scene,
                &looking_at_origin(Vec3::new(0.2, 0.2, 2.0)),
                VIEWPORT_PX,
            )
            .unwrap();
        assert_eq!(near, StaticUpdate::Replace);
        let near_lod = selected_lod(&detail, "boulder", &detail.instances()[0]);
        assert_eq!(near_lod, Lod::Source);

        let far = detail
            .update(
                &mut scene,
                &looking_at_origin(Vec3::new(0.2, 0.2, 220.0)),
                VIEWPORT_PX,
            )
            .unwrap();
        assert_eq!(far, StaticUpdate::Replace, "a coarse level is new geometry");
        let far_lod = selected_lod(&detail, "boulder", &detail.instances()[0]);
        assert!(far_lod > near_lod, "far camera chose {far_lod:?}");
        assert_eq!(
            detail.runtime.revision("boulder", far_lod),
            Some(revision),
            "the coarse copy carries the live source revision"
        );

        // Selection is derived: authoritative source cells, version and bytes
        // are untouched by any prepare. `counts` also reports engine-side
        // derived-cache statistics, which a prepare is expected to grow.
        let after = scene.counts();
        assert_eq!(
            after.expanded_occupied_cells,
            counts.expanded_occupied_cells
        );
        assert_eq!(after.unique_stored_cells, counts.unique_stored_cells);
        assert_eq!(after.source_bytes, counts.source_bytes);
        assert_eq!(scene.source_version(), version);
        assert_eq!(scene.prototype("boulder").unwrap().revision(), revision);
    }

    #[test]
    fn edited_placement_refreshes_geometry_and_republishes_without_stale_revisions() {
        let mut scene = boulder_and_keeper_scene();
        let mut detail = WetlandDetail::new();
        let camera = looking_at_origin(Vec3::new(0.2, 0.2, 2.0));
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace
        );
        detail.mark_installed();
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Current
        );

        let stale = scene.prototype("boulder").unwrap().revision();
        let keeper_before = detail.runtime.revision("keeper", Lod::Source);
        assert!(scene.edit_instance("near", [4, 4, 4], 0).unwrap());
        assert_ne!(scene.prototype("boulder").unwrap().revision(), stale);

        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace,
            "a real placement edit must refresh derived geometry"
        );
        let edited_prototype = scene
            .draws()
            .into_iter()
            .find(|draw| draw.instance == "near")
            .unwrap()
            .prototype;
        let edited_revision = scene.prototype(&edited_prototype).unwrap().revision();
        let near = detail
            .instances()
            .iter()
            .find(|instance| instance.translation == [0.0, 0.0, 0.0])
            .expect("the edited boulder is still published");
        assert_eq!(detail.meshes()[near.prototype].revision, edited_revision);
        assert_ne!(detail.meshes()[near.prototype].revision, stale);
        assert_eq!(
            detail.runtime.revision("keeper", Lod::Source),
            keeper_before,
            "an untouched prototype is not refreshed"
        );

        detail.mark_installed();
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Current,
            "the edit is published once, not re-installed every frame"
        );
    }

    #[test]
    fn recreated_surface_reinstalls_the_current_selection() {
        let mut scene = boulder_scene();
        let mut detail = WetlandDetail::new();
        let camera = looking_at_origin(Vec3::new(0.2, 0.2, 2.0));
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace
        );
        detail.mark_installed();
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Current
        );
        let selection = detail.instances().to_vec();

        detail.rearm();
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace,
            "a recreated surface lost its geometry and must be repopulated"
        );
        assert_eq!(
            detail.instances(),
            selection.as_slice(),
            "reinstall republishes the current selection, not a new one"
        );
        detail.mark_installed();
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Current
        );
    }

    #[test]
    fn lod_camera_matches_the_render_projection() {
        use glam::Mat4;

        // Pitch stays zero so +Y is preserved in the view basis: the combined
        // matrix's vertical scale then comes straight from the render
        // projection `controls::Camera::view_projection` builds.
        let camera = Camera {
            position: Vec3::new(4.0, 6.0, 8.0),
            yaw: 0.9,
            pitch: 0.0,
        };
        let lod = lod_camera(&camera, 900.0);
        assert_eq!(lod.eye_m, camera.position.to_array());
        assert_eq!(lod.forward_m, camera.forward().to_array());
        assert_eq!(lod.viewport_height_px, 900.0);

        let view_projection = Mat4::from_cols_array_2d(&camera.view_projection(16.0 / 9.0));
        // glam's `perspective_rh` stores 1/tan(fov_y/2) in m11. The engine's
        // pixels-per-metre must use that same vertical scale, or production
        // selection would run against a field of view the renderer never draws.
        let render_y_scale = view_projection.y_axis.y;
        let lod_y_scale = 2.0 * lod.pixels_per_metre(1.0) / 900.0;
        assert!(
            (lod_y_scale - render_y_scale).abs() < 1e-5,
            "lod fov {} disagrees with the render projection",
            WETLAND_FOV_RAD
        );
        // A point exactly `near_m` ahead of the render camera maps to ndc z 0,
        // so the engine clamps depth at the same near plane the renderer uses.
        let near_point = camera.position + camera.forward() * lod.near_m;
        let clip = view_projection * near_point.extend(1.0);
        assert!(
            (clip.z / clip.w).abs() < 1e-4,
            "lod near {} is not the render near plane",
            lod.near_m
        );
    }

    /// Three dense prototypes further apart than the production coarse cap, all
    /// requesting a coarse level from one distant view.
    fn far_triple_scene() -> DetailScene {
        let mut scene = DetailScene::new();
        for (index, name) in ["a", "b", "c"].into_iter().enumerate() {
            scene.add_prototype(dense(name, 8)).unwrap();
            scene
                .place(
                    name,
                    name,
                    Transform::new([index as f32 * 8.0, 0.0, 0.0], Yaw::Deg0).unwrap(),
                )
                .unwrap();
        }
        scene
    }

    /// Resident coarse levels of the [`far_triple_scene`] prototypes.
    fn coarse_resident(detail: &WetlandDetail) -> usize {
        ["a", "b", "c"]
            .into_iter()
            .map(|name| {
                [Lod::Half, Lod::Quarter]
                    .into_iter()
                    .filter(|lod| detail.runtime.instance_index(name, *lod).is_some())
                    .count()
            })
            .sum()
    }

    #[test]
    fn coarse_builds_per_prepare_are_capped_and_capped_instances_fall_back_to_source() {
        let mut scene = far_triple_scene();
        let mut detail = WetlandDetail::new();
        let cap = detail
            .config
            .max_coarse_builds
            .expect("production config is bounded");
        assert!(cap > 0 && cap < 3, "the fixture must exceed the cap: {cap}");
        let update = detail
            .update(
                &mut scene,
                &looking_at_origin(Vec3::new(8.0, 0.2, 400.0)),
                VIEWPORT_PX,
            )
            .unwrap();
        assert_eq!(update, StaticUpdate::Replace);
        assert_eq!(
            coarse_resident(&detail),
            cap,
            "one prepare must realize at most its capped coarse levels"
        );
        assert_eq!(
            detail.instances().len(),
            3,
            "capped instances still draw from Source"
        );
        for instance in detail.instances() {
            assert!(!detail.meshes()[instance.prototype].vertices.is_empty());
        }
    }

    #[test]
    fn stationary_reprepares_converge_capped_coarse_levels_and_then_stop() {
        let mut scene = far_triple_scene();
        let mut detail = WetlandDetail::new();
        let cap = detail
            .config
            .max_coarse_builds
            .expect("production config is bounded");
        assert!(cap > 0 && cap < 3, "the fixture must exceed the cap: {cap}");
        let camera = looking_at_origin(Vec3::new(8.0, 0.2, 400.0));

        // Frame one realizes only the capped number of new coarse levels.
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace
        );
        assert_eq!(coarse_resident(&detail), cap);
        detail.mark_installed();

        // A stationary camera must not strand the deferred level at Source:
        // the next bounded prepare realizes it.
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace,
            "the deferred coarse level is realized on the next stationary frame"
        );
        assert_eq!(
            coarse_resident(&detail),
            3,
            "all requested coarse levels are resident"
        );
        detail.mark_installed();

        // Converged: another stationary frame repeats no selection work and no
        // GPU install.
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Current
        );
    }

    #[test]
    fn emptied_prototype_is_omitted_and_the_rest_still_publishes() {
        let mut scene = boulder_and_keeper_scene();
        let mut detail = WetlandDetail::new();
        let camera = looking_at_origin(Vec3::new(0.2, 0.2, 2.0));
        detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap();
        detail.mark_installed();
        assert_eq!(detail.instances().len(), 2);

        // Clear every cell of one placed prototype: an ordinary destructive edit.
        for x in 0..8 {
            for y in 0..8 {
                for z in 0..8 {
                    scene
                        .edit_prototype("boulder", [x, y, z], material::AIR)
                        .unwrap();
                }
            }
        }
        assert_eq!(scene.prototype("boulder").unwrap().occupied_cells(), 0);
        assert_eq!(
            detail.update(&mut scene, &camera, VIEWPORT_PX).unwrap(),
            StaticUpdate::Replace,
            "an emptied prototype changes derived geometry"
        );
        assert_eq!(
            detail.instances().len(),
            1,
            "the emptied prototype contributes no drawable instance"
        );
        assert_eq!(
            detail.instances()[0].translation,
            [6.0, 0.0, 0.0],
            "the other instance still publishes"
        );
        assert!(!detail.meshes()[detail.instances()[0].prototype]
            .vertices
            .is_empty());
    }
}

#[cfg(test)]
mod shared_settings_ui_tests {
    use super::*;
    use crate::controls::{settings_panel, virtual_point};
    use crate::settings::{Handedness, SettingRow};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn app() -> WetlandApp {
        let directory = std::env::temp_dir().join(format!(
            "matterweave-wetland-settings-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        WetlandApp::new(directory, false, None)
    }

    fn center(rect: [f32; 4]) -> Vec2 {
        Vec2::new(rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.)
    }

    #[test]
    fn chooser_settings_button_opens_the_shared_panel() {
        let mut app = app();
        assert!(app.menu, "the chooser opens first");
        assert!(!app.settings_open);
        assert!(app.click(Vec2::new(930., 41.)), "the panel is reachable");
        assert!(app.settings_open);
        assert!(
            app.take_settings_change().is_none(),
            "opening is not a change"
        );
    }

    #[test]
    fn panel_toggles_queue_one_shot_changes_and_done_closes() {
        let mut app = app();
        assert!(app.click(Vec2::new(930., 41.)));
        let panel = settings_panel();

        assert!(app.click(center(panel.rows[0].0)));
        assert_eq!(app.settings.handedness, Handedness::Right);
        assert_eq!(
            app.take_settings_change(),
            Some(SharedSettings {
                handedness: Handedness::Right,
                ..SharedSettings::default()
            })
        );
        assert!(
            app.take_settings_change().is_none(),
            "a change is reported exactly once"
        );

        assert!(app.click(center(panel.rows[1].0)));
        assert!(app.settings.large_controls);
        assert!(app.take_settings_change().is_some());
        assert!(app.click(center(panel.rows[2].0)));
        assert!(app.settings.muted);
        assert!(app.take_settings_change().is_some());

        assert!(app.click(center(panel.done)));
        assert!(!app.settings_open);
        assert!(
            app.take_settings_change().is_none(),
            "DONE only closes the overlay"
        );
    }

    #[test]
    fn in_game_options_route_reaches_the_same_panel() {
        let mut app = app();
        app.menu = false;
        assert!(
            app.click(Vec2::new(930., 41.)),
            "the MENU button opens options"
        );
        assert!(app.options);
        // SETTINGS is the fifth options row (300..345).
        assert!(app.click(Vec2::new(800., 320.)));
        assert!(app.settings_open);
        let panel = settings_panel();
        assert!(app.click(center(panel.rows[0].0)));
        assert_eq!(
            app.settings.handedness,
            Handedness::Right,
            "the in-game route edits the same preferences"
        );
    }

    #[test]
    fn action_hit_regions_follow_handedness_and_scale_with_the_shared_layout() {
        for settings in [
            SharedSettings::default(),
            SharedSettings {
                handedness: Handedness::Right,
                ..SharedSettings::default()
            },
            SharedSettings {
                handedness: Handedness::Right,
                large_controls: true,
                ..SharedSettings::default()
            },
            SharedSettings {
                large_controls: true,
                ..SharedSettings::default()
            },
        ] {
            let mut app = app();
            app.menu = false;
            app.set_shared_settings(settings);
            let layout = wetland_layout(&app.settings);
            for (rect, _, _) in layout.actions {
                assert!(
                    app.click(center(rect)),
                    "the drawn action {rect:?} must be clickable ({settings:?})"
                );
            }
            // A touch in the movement zone is never an action.
            assert!(!app.click(center(layout.move_zone)));
            assert!(!app.click(center(layout.jump_zone)));
        }
    }

    #[test]
    fn applying_a_layout_change_clears_held_touches() {
        let mut app = app();
        app.menu = false;
        let before = wetland_layout(&app.settings);
        let grip = center(before.move_zone);
        app.controls.start_wetland(&before, 7, grip);
        app.controls.moved(7, grip + Vec2::new(0., -70.));
        assert!(app.controls.consume().0.z > 0.5, "the held touch moves");

        // Two fingers: one on the stick, one opening the panel from MENU.
        assert!(app.click(Vec2::new(930., 41.)), "open options");
        assert!(app.click(Vec2::new(800., 320.)), "open settings");
        assert_eq!(
            app.controls.service.active_pointers(),
            0,
            "opening the modal overlay must drop the held contact"
        );

        // The toggle re-derives the layout; the stale pointer id stays dead.
        let panel = settings_panel();
        assert!(app.click(center(panel.rows[0].0)));
        assert_eq!(app.settings.handedness, Handedness::Right);
        app.controls.moved(7, grip + Vec2::new(0., -70.));
        assert_eq!(
            app.controls.consume(),
            (Vec3::ZERO, Vec2::ZERO),
            "a held touch must not keep driving the previous layout"
        );
    }

    #[test]
    fn owner_install_updates_the_rendered_preferences_without_queueing_a_change() {
        let mut app = app();
        let installed = SharedSettings {
            handedness: Handedness::Right,
            large_controls: true,
            muted: true,
            ..SharedSettings::default()
        };
        app.set_shared_settings(installed);
        assert_eq!(app.settings, installed);
        assert_eq!(app.settings.value_label(SettingRow::Mute), "MUTED");
        assert!(app.take_settings_change().is_none());
        assert_eq!(wetland_layout(&app.settings).move_zone[0], 762.);
    }

    #[test]
    fn narrow_and_wide_viewports_map_touches_into_the_drawn_rectangles() {
        let settings = SharedSettings {
            handedness: Handedness::Right,
            large_controls: true,
            ..SharedSettings::default()
        };
        let layout = wetland_layout(&settings);
        for (width, height) in [(1000_u32, 600_u32), (480, 800), (1600, 720)] {
            for rect in [layout.move_zone, layout.jump_zone, layout.actions[2].0] {
                let center = center(rect);
                let pixel = (
                    center.x / 1000. * width as f32,
                    center.y / 600. * height as f32,
                );
                let mapped = virtual_point(pixel.0 as f64, pixel.1 as f64, width, height);
                assert!(
                    contains(rect, mapped),
                    "{width}x{height} must map into {rect:?}, got {mapped:?}"
                );
            }
        }
    }
}
