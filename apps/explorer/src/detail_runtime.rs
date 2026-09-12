//! Production automatic-detail runtime for the wetland sample.
//!
//! Owns the resident copy of a [`DetailScene`]'s derived geometry and drives the
//! per-frame LOD path: [`DetailRuntime::prepare`] runs
//! [`DetailScene::prepare_batches`] for the frame camera, refreshes resident
//! geometry whose prototype source revision moved on (a source edit), and maps
//! the resulting selection onto packed [`StaticInstance`] records whose
//! `prototype` field indexes this runtime's resident mesh pool.
//!
//! Invariants:
//! - Source voxels stay authoritative. This module never mutates the scene and
//!   never derives collision; physics keeps the existing path.
//! - No stale derived geometry is handed out. Every selected `(prototype, lod)`
//!   yielded by a frame is validated against the prototype's live source
//!   revision before it is mapped, so a level that has not been refreshed is
//!   rejected rather than drawn. A prototype with no occupied cells has no
//!   drawable geometry and simply contributes no instance.
//! - No panics on the frame path. Every fallible step returns
//!   `Result<_, String>`, matching the existing adapter.
//!
//! Refresh is selection-driven: exactly the `(prototype, lod)` pairs the engine
//! selected for the frame are refreshed within that frame. That keeps an edit's
//! refresh inside the same `LodConfig::max_coarse_builds` budget as the frame
//! (an unavailable coarse mesh falls back to `Source` in the engine, and no
//! coarse mesh is built behind the cap). A coarser level of an edited prototype
//! is refreshed on the next frame that selects it, and
//! [`FrameUpdate::geometry_changed`] tells the caller that frame requires a
//! static-scene re-install rather than an instance-only update.

use matterweave_core::Mesh;
use matterweave_detail::{
    Camera, DetailError, DetailScene, Lod, LodConfig, PreparedFrame, SceneVersion, Yaw,
};
use matterweave_render::StaticInstance;
use std::collections::BTreeMap;

/// Derived levels every nonempty prototype keeps resident.
pub const RESIDENT_LODS: [Lod; 3] = [Lod::Source, Lod::Half, Lod::Quarter];

/// Quarter-turn yaw encoding consumed by [`StaticInstance`].
pub(crate) fn yaw_quarters(yaw: Yaw) -> u8 {
    match yaw {
        Yaw::Deg0 => 0,
        Yaw::Deg90 => 1,
        Yaw::Deg180 => 2,
        Yaw::Deg270 => 3,
    }
}

/// Count of each chosen LOD in a prepared frame, for reports and checks.
pub fn lod_histogram(frame: &PreparedFrame) -> BTreeMap<Lod, usize> {
    let mut counts = BTreeMap::new();
    for item in &frame.selected {
        *counts.entry(item.lod).or_insert(0) += 1;
    }
    counts
}

/// One prepared frame with its drawable instances.
#[derive(Clone, Debug)]
pub struct FrameUpdate {
    /// The engine's selection and budget outcome for this frame.
    pub frame: PreparedFrame,
    /// Packed instances referencing resident mesh-pool indices. An instance
    /// whose prototype has no occupied cells contributes no drawable geometry
    /// and is omitted, so this may be shorter than `frame.selected`.
    pub instances: Vec<StaticInstance>,
    /// True when a resident mesh was added or replaced during this prepare, so
    /// the renderer must re-install the static scene (geometry changed, not
    /// just placement). False means an instance-only update is sufficient.
    pub geometry_changed: bool,
}

/// Resident derived geometry of one prototype: the mesh-pool index and the
/// authoritative source revision behind the copy, per derived level.
#[derive(Default)]
struct ResidentPrototype {
    index: [Option<usize>; 3],
    revision: [Option<u64>; 3],
}

/// Owns the resident derived geometry for one [`DetailScene`] and maps each
/// prepared frame's selection onto it.
#[derive(Default)]
pub struct DetailRuntime {
    meshes: Vec<Mesh>,
    prototypes: BTreeMap<String, ResidentPrototype>,
    source_version: SceneVersion,
}

impl DetailRuntime {
    /// An empty runtime. The first [`Self::prepare`] realizes only the levels
    /// the frame selects; use [`Self::preload`] to realize every level of every
    /// nonempty prototype up front.
    pub fn new() -> Self {
        Self::default()
    }

    /// Realizes `Source`/`Half`/`Quarter` for every nonempty prototype exactly
    /// once, verifying each derived mesh carries the authoritative source
    /// revision, and records each `(prototype, lod)` -> pool-index map. A level
    /// the engine cannot build within the scale or derived-mesh cache budget
    /// (`DetailError::InvalidScale` or `DetailError::BudgetExceeded`) is simply
    /// left non-resident, matching how `DetailScene::ensure_mesh` degrades to
    /// `Lod::Source` instead of failing. An empty scene produces an empty
    /// runtime, not an error.
    pub fn preload(scene: &mut DetailScene) -> Result<Self, String> {
        let mut runtime = Self::new();
        let mut ids = scene.prototype_ids();
        ids.sort();
        for id in ids {
            let occupied = scene
                .prototype(&id)
                .ok_or_else(|| format!("prototype {id} vanished"))?
                .occupied_cells();
            if occupied == 0 {
                continue;
            }
            let revision = scene
                .prototype(&id)
                .ok_or_else(|| format!("prototype {id} vanished"))?
                .revision();
            for lod in RESIDENT_LODS {
                if runtime.revision(&id, lod) == Some(revision) {
                    continue;
                }
                // An unbuildable level stays non-resident; selection falls
                // back to `Source` exactly as the engine's own path does.
                runtime.refresh_entry(scene, &id, lod, revision)?;
            }
        }
        runtime.source_version = scene.source_version();
        Ok(runtime)
    }

    /// Prepares one frame: asks the engine to select a level per instance and
    /// realize the selected derived meshes within budget, refreshes any
    /// selected resident copy whose source revision changed, and maps the
    /// selection onto the resident pool. The returned instances are always
    /// built from the frame's live source revision.
    pub fn prepare(
        &mut self,
        scene: &mut DetailScene,
        camera: &Camera,
        config: &LodConfig,
    ) -> Result<FrameUpdate, String> {
        let frame = scene
            .prepare_batches(camera, config)
            .map_err(|e| e.to_string())?;
        if frame.source_version != scene.source_version() {
            return Err("prepared frame source version disagrees with the scene".into());
        }
        let geometry_changed = self.refresh_selected(scene, &frame)?;
        self.source_version = frame.source_version.clone();
        let instances = self.instances_for_frame(scene, &frame)?;
        Ok(FrameUpdate {
            frame,
            instances,
            geometry_changed,
        })
    }

    /// Maps a prepared frame's per-instance LOD selection to packed instances
    /// that reference resident geometry, validating each selected level against
    /// `scene`'s live source revision. Only the instance list changes between
    /// frames; geometry is never rebuilt here. An instance whose prototype has
    /// no occupied cells contributes no drawable geometry and is omitted. A
    /// frame prepared from a different source state than this runtime's
    /// resident geometry, or one whose selected level is superseded, is
    /// rejected, never mis-mapped.
    pub fn instances_for_frame(
        &self,
        scene: &DetailScene,
        frame: &PreparedFrame,
    ) -> Result<Vec<StaticInstance>, String> {
        if frame.source_version != self.source_version {
            return Err("prepared frame and resident geometry source versions differ".into());
        }
        let mut out = Vec::with_capacity(frame.selected.len());
        for item in &frame.selected {
            let source = scene
                .prototype(&item.prototype)
                .ok_or_else(|| format!("selected prototype {} vanished", item.prototype))?;
            if source.occupied_cells() == 0 {
                // No drawable geometry; the instance contributes nothing.
                continue;
            }
            let prototype = self
                .instance_index(&item.prototype, item.lod)
                .ok_or_else(|| {
                    format!(
                        "selected {} {:?} has no resident geometry",
                        item.prototype, item.lod
                    )
                })?;
            let source_revision = source.revision();
            let resident_revision = self.revision(&item.prototype, item.lod);
            if resident_revision != Some(source_revision) {
                return Err(format!(
                    "selected {} {:?} resident revision {resident_revision:?} is superseded by {source_revision}",
                    item.prototype, item.lod
                ));
            }
            out.push(StaticInstance {
                prototype,
                translation: item.transform.translation_m,
                yaw_quarters: yaw_quarters(item.transform.yaw),
            });
        }
        Ok(out)
    }

    /// The resident mesh pool; pool indices index this slice.
    pub fn meshes(&self) -> &[Mesh] {
        &self.meshes
    }

    /// Pool index of the resident geometry for one `(prototype, Lod)`.
    pub fn instance_index(&self, prototype: &str, lod: Lod) -> Option<usize> {
        self.prototypes.get(prototype)?.index[lod.index()]
    }

    /// Source revision of the resident geometry for one `(prototype, Lod)`.
    pub fn revision(&self, prototype: &str, lod: Lod) -> Option<u64> {
        self.prototypes.get(prototype)?.revision[lod.index()]
    }

    /// Refreshes the selected `(prototype, lod)` pairs whose resident copy is
    /// missing or built from an older source revision. Returns whether the
    /// resident pool changed. A selected prototype with no occupied cells is
    /// refreshed to its empty derived mesh, reusing its pool slot, and
    /// [`Self::instances_for_frame`] omits it as undrawable.
    fn refresh_selected(
        &mut self,
        scene: &mut DetailScene,
        frame: &PreparedFrame,
    ) -> Result<bool, String> {
        let mut changed = false;
        for item in &frame.selected {
            let source = scene
                .prototype(&item.prototype)
                .ok_or_else(|| format!("selected prototype {} vanished", item.prototype))?;
            let source_revision = source.revision();
            if self.revision(&item.prototype, item.lod) == Some(source_revision) {
                continue;
            }
            if self.refresh_entry(scene, &item.prototype, item.lod, source_revision)? {
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Copies one freshly realized derived mesh out of the scene cache into the
    /// resident pool, after verifying it carries `source_revision`. Returns
    /// `false` when the engine reports the level as unbuildable within the
    /// scale or derived-mesh cache budget; the level then stays non-resident
    /// and selection falls back to `Source`, matching
    /// `DetailScene::ensure_mesh`.
    fn refresh_entry(
        &mut self,
        scene: &mut DetailScene,
        prototype: &str,
        lod: Lod,
        source_revision: u64,
    ) -> Result<bool, String> {
        // Realize (or reuse) the mesh in the engine cache first; the borrow is
        // dropped before the cache is read back.
        match scene.prototype_mesh(prototype, lod) {
            Ok(_) => {}
            Err(DetailError::BudgetExceeded(_)) | Err(DetailError::InvalidScale) => {
                return Ok(false)
            }
            Err(other) => return Err(format!("build {prototype} {lod:?}: {other}")),
        }
        let cached = scene
            .cached_prototype_mesh(prototype, lod)
            .ok_or_else(|| format!("{prototype} {lod:?} not resident after build"))?;
        if cached.revision != source_revision {
            return Err(format!(
                "{prototype} {lod:?} derived revision {} does not match source revision {source_revision}",
                cached.revision
            ));
        }
        self.store(prototype, lod, cached);
        Ok(true)
    }

    /// Copies one mesh into the pool, reusing an existing index so previously
    /// handed-out instance indices stay valid.
    fn store(&mut self, prototype: &str, lod: Lod, source: &Mesh) {
        let mesh = Mesh {
            vertices: source.vertices.clone(),
            indices: source.indices.clone(),
            revision: source.revision,
        };
        let entry = self.prototypes.entry(prototype.to_string()).or_default();
        match entry.index[lod.index()] {
            Some(index) => self.meshes[index] = mesh,
            None => {
                entry.index[lod.index()] = Some(self.meshes.len());
                self.meshes.push(mesh);
            }
        }
        entry.revision[lod.index()] = Some(source.revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_detail::{
        material, DetailVolume, Projection, Scale, Transform, Yaw, SCALE_FINE_M,
    };

    /// Look target shared by both cameras, inside the boulder's footprint.
    const TARGET_M: [f32; 3] = [0.2, 0.2, 0.0];
    const VIEWPORT_PX: f32 = 1080.0;
    const FOV_RAD: f32 = std::f32::consts::PI / 3.0;

    fn camera(eye_m: [f32; 3]) -> Camera {
        Camera {
            eye_m,
            forward_m: [
                TARGET_M[0] - eye_m[0],
                TARGET_M[1] - eye_m[1],
                TARGET_M[2] - eye_m[2],
            ],
            viewport_height_px: VIEWPORT_PX,
            near_m: 0.1,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV_RAD,
            },
        }
    }

    fn near_camera() -> Camera {
        camera([0.2, 0.2, 2.0])
    }

    fn far_camera() -> Camera {
        camera([0.2, 0.2, 220.0])
    }

    fn very_far_camera() -> Camera {
        camera([0.2, 0.2, 1000.0])
    }

    fn dense_volume(id: &str, edge: i32, scale_m: f32) -> DetailVolume {
        let mut volume = DetailVolume::new(id, Scale::new(scale_m).expect("cell scale"));
        for x in 0..edge {
            for y in 0..edge {
                for z in 0..edge {
                    volume
                        .set([x, y, z], material::BANK_STONE)
                        .expect("solid cell");
                }
            }
        }
        volume
    }

    /// One dense 8^3 boulder at the origin. Dense solids carry no coarsening
    /// guard, so the engine's distance rule decides between `Source` and
    /// `Quarter` without a hand-written selection rule in this test.
    fn boulder_scene() -> DetailScene {
        let mut scene = DetailScene::new();
        scene
            .add_prototype(dense_volume("boulder", 8, SCALE_FINE_M))
            .expect("add boulder");
        scene
            .place("near", "boulder", Transform::identity())
            .expect("place boulder");
        scene
    }

    /// The boulder plus a second, independent dense prototype so a frame can
    /// prove other instances survive a destructive edit to one object.
    fn boulder_and_keeper_scene() -> DetailScene {
        let mut scene = boulder_scene();
        scene
            .add_prototype(dense_volume("keeper", 4, SCALE_FINE_M))
            .expect("add keeper");
        scene
            .place(
                "keeper_placed",
                "keeper",
                Transform::new([3.0, 0.0, 0.0], Yaw::Deg0).expect("keeper transform"),
            )
            .expect("place keeper");
        scene
    }

    fn clear_prototype(scene: &mut DetailScene, id: &str, edge: i32) {
        for x in 0..edge {
            for y in 0..edge {
                for z in 0..edge {
                    scene.edit_prototype(id, [x, y, z], material::AIR).unwrap();
                }
            }
        }
    }

    #[test]
    fn prepare_maps_selection_to_resident_geometry() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let update = runtime
            .prepare(&mut scene, &near_camera(), &LodConfig::default())
            .unwrap();

        assert!(!update.frame.selected.is_empty());
        assert_eq!(update.instances.len(), update.frame.selected.len());
        for (instance, selected) in update.instances.iter().zip(&update.frame.selected) {
            let index = runtime
                .instance_index(&selected.prototype, selected.lod)
                .expect("selected level must be resident");
            assert_eq!(instance.prototype, index);
            assert!(index < runtime.meshes().len());
            assert_eq!(instance.translation, selected.transform.translation_m);
            assert_eq!(instance.yaw_quarters, yaw_quarters(selected.transform.yaw));
        }
    }

    #[test]
    fn edit_refreshes_selected_geometry_and_never_hands_out_stale_revisions() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let stale_revision = scene.prototype("boulder").unwrap().revision();

        assert!(scene
            .edit_prototype("boulder", [12, 12, 12], material::BANK_STONE)
            .unwrap());
        let fresh_revision = scene.prototype("boulder").unwrap().revision();
        assert_ne!(fresh_revision, stale_revision);

        let mut coarse_seen = false;
        for camera in [near_camera(), far_camera()] {
            let update = runtime
                .prepare(&mut scene, &camera, &LodConfig::default())
                .unwrap();
            assert!(
                update.geometry_changed,
                "an edited prototype must force a resident-geometry refresh"
            );
            for item in &update.frame.selected {
                let resident = runtime
                    .revision(&item.prototype, item.lod)
                    .expect("selected level must be resident");
                assert_eq!(
                    resident,
                    scene.prototype(&item.prototype).unwrap().revision(),
                    "{} {:?} was handed out from a stale revision",
                    item.prototype,
                    item.lod
                );
                assert_ne!(resident, stale_revision);
                if item.lod != Lod::Source {
                    coarse_seen = true;
                }
            }
            for (instance, selected) in update.instances.iter().zip(&update.frame.selected) {
                let index = runtime
                    .instance_index(&selected.prototype, selected.lod)
                    .unwrap();
                assert_eq!(instance.prototype, index);
                assert_eq!(runtime.meshes()[index].revision, fresh_revision);
            }
        }
        assert!(coarse_seen, "the far camera must select a coarse level");
    }

    #[test]
    fn near_camera_selects_a_finer_level_than_far_camera() {
        let mut near_scene = boulder_scene();
        let mut near_runtime = DetailRuntime::preload(&mut near_scene).unwrap();
        let near = near_runtime
            .prepare(&mut near_scene, &near_camera(), &LodConfig::default())
            .unwrap();

        let mut far_scene = boulder_scene();
        let mut far_runtime = DetailRuntime::preload(&mut far_scene).unwrap();
        let far = far_runtime
            .prepare(&mut far_scene, &far_camera(), &LodConfig::default())
            .unwrap();

        let near_lod = near.frame.selected[0].lod;
        let far_lod = far.frame.selected[0].lod;
        assert_eq!(near_lod, Lod::Source);
        assert!(far_lod > near_lod, "far camera chose {far_lod:?}");
        assert_ne!(lod_histogram(&near.frame), lod_histogram(&far.frame));
    }

    #[test]
    fn zero_coarse_budget_falls_back_to_source_without_refreshing_coarse_copies() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let stale_quarter = runtime.revision("boulder", Lod::Quarter).unwrap();

        scene
            .edit_prototype("boulder", [12, 12, 12], material::BANK_STONE)
            .unwrap();
        let config = LodConfig {
            max_coarse_builds: Some(0),
            ..LodConfig::default()
        };
        let update = runtime.prepare(&mut scene, &far_camera(), &config).unwrap();

        assert!(update
            .frame
            .selected
            .iter()
            .all(|item| item.lod == Lod::Source && item.fallback));
        assert_eq!(update.frame.mesh_builds_this_call, 1);
        // The capped frame refreshed the authoritative Source only: the stale
        // coarse copy was neither handed out nor implicitly rebuilt.
        assert_eq!(
            runtime.revision("boulder", Lod::Quarter),
            Some(stale_quarter)
        );
        assert_eq!(
            runtime.revision("boulder", Lod::Source),
            Some(scene.prototype("boulder").unwrap().revision())
        );
        assert_eq!(
            update.instances[0].prototype,
            runtime.instance_index("boulder", Lod::Source).unwrap()
        );
    }

    #[test]
    fn emptied_prototype_contributes_no_instance_and_refill_restores_geometry() {
        let mut scene = boulder_and_keeper_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let before = runtime
            .prepare(&mut scene, &near_camera(), &LodConfig::default())
            .unwrap();
        assert_eq!(
            before.instances.len(),
            2,
            "both objects draw before the edit"
        );

        // Clearing every cell of a placed object is an ordinary destructive edit.
        clear_prototype(&mut scene, "boulder", 8);
        assert_eq!(scene.prototype("boulder").unwrap().occupied_cells(), 0);

        let emptied = runtime
            .prepare(&mut scene, &near_camera(), &LodConfig::default())
            .unwrap();
        assert!(emptied.geometry_changed);
        // The engine still selects the emptied instance, but it draws nothing.
        assert!(emptied.frame.selected.iter().any(|s| s.instance == "near"));
        assert_eq!(emptied.instances.len(), emptied.frame.selected.len() - 1);
        let boulder_index = runtime.instance_index("boulder", Lod::Source).unwrap();
        assert!(emptied
            .instances
            .iter()
            .all(|instance| instance.prototype != boulder_index));
        // Other instances in the same frame are still returned correctly.
        let keeper = emptied
            .frame
            .selected
            .iter()
            .find(|s| s.instance == "keeper_placed")
            .expect("keeper must still be selected");
        let keeper_index = runtime.instance_index("keeper", keeper.lod).unwrap();
        assert_eq!(
            emptied
                .instances
                .iter()
                .filter(|instance| instance.prototype == keeper_index)
                .count(),
            1
        );
        assert_eq!(
            runtime.revision("keeper", keeper.lod),
            Some(scene.prototype("keeper").unwrap().revision())
        );

        // Refilling restores the boulder's geometry and its instance.
        assert!(scene
            .edit_prototype("boulder", [0, 0, 0], material::BANK_STONE)
            .unwrap());
        let refilled = runtime
            .prepare(&mut scene, &near_camera(), &LodConfig::default())
            .unwrap();
        assert_eq!(refilled.instances.len(), refilled.frame.selected.len());
        let near = refilled
            .frame
            .selected
            .iter()
            .find(|s| s.instance == "near")
            .expect("refilled boulder must be selected");
        let near_index = runtime.instance_index("boulder", near.lod).unwrap();
        assert_eq!(
            runtime.revision("boulder", near.lod),
            Some(scene.prototype("boulder").unwrap().revision())
        );
        assert!(!runtime.meshes()[near_index].vertices.is_empty());
        assert_eq!(
            refilled
                .instances
                .iter()
                .filter(|instance| instance.prototype == near_index)
                .count(),
            1
        );
    }

    #[test]
    fn partially_refreshed_runtime_rejects_a_foreign_frame() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        scene
            .edit_prototype("boulder", [12, 12, 12], material::BANK_STONE)
            .unwrap();

        // The near prepare refreshes only the level it selected (`Source`).
        let near = runtime
            .prepare(&mut scene, &near_camera(), &LodConfig::default())
            .unwrap();
        assert_eq!(near.frame.selected[0].lod, Lod::Source);

        // A frame prepared directly from the scene for a far camera selects a
        // level this runtime has not refreshed. Mapping it must not hand out
        // the superseded copy.
        let far = scene
            .prepare_batches(&far_camera(), &LodConfig::default())
            .unwrap();
        let far_lod = far.selected[0].lod;
        assert!(far_lod > Lod::Source, "far camera chose {far_lod:?}");
        assert_ne!(
            runtime.revision("boulder", far_lod),
            Some(scene.prototype("boulder").unwrap().revision())
        );
        assert!(runtime.instances_for_frame(&scene, &far).is_err());
    }

    #[test]
    fn preload_degrades_when_a_coarse_level_is_out_of_scale_range() {
        // `Quarter` of 0.3 m cells is 1.2 m, past `MAX_SCALE_M` (1.0 m).
        let mut scene = DetailScene::new();
        scene
            .add_prototype(dense_volume("coarse_boulder", 4, 0.3))
            .expect("add coarse boulder");
        scene
            .place("far", "coarse_boulder", Transform::identity())
            .expect("place coarse boulder");

        let mut runtime =
            DetailRuntime::preload(&mut scene).expect("preload must degrade, not fail");
        assert!(runtime
            .instance_index("coarse_boulder", Lod::Source)
            .is_some());
        assert!(runtime
            .instance_index("coarse_boulder", Lod::Half)
            .is_some());
        assert!(runtime
            .instance_index("coarse_boulder", Lod::Quarter)
            .is_none());

        // The in-range coarse level still renders; the out-of-range one is
        // never selected, so nothing falls back into an unbuildable state.
        let update = runtime
            .prepare(&mut scene, &very_far_camera(), &LodConfig::default())
            .unwrap();
        assert_eq!(update.frame.selected[0].lod, Lod::Half);
        assert_eq!(update.instances.len(), 1);
        assert_eq!(
            update.instances[0].prototype,
            runtime.instance_index("coarse_boulder", Lod::Half).unwrap()
        );
    }

    #[test]
    fn preload_degrades_when_a_level_exceeds_the_mesh_budget() {
        // 33^3 cells cross the 32 MiB derived-mesh output upper bound before any
        // allocation: the same condition `ensure_mesh` degrades on. The scene is
        // still cheap in source bytes (9 chunks), so this stays a fast test.
        let mut scene = DetailScene::new();
        scene
            .add_prototype(dense_volume("huge_boulder", 33, SCALE_FINE_M))
            .expect("add huge boulder");
        scene
            .place("huge", "huge_boulder", Transform::identity())
            .expect("place huge boulder");

        let runtime = DetailRuntime::preload(&mut scene).expect("preload must degrade, not fail");
        for lod in RESIDENT_LODS {
            assert!(
                runtime.instance_index("huge_boulder", lod).is_none(),
                "{lod:?} must stay non-resident"
            );
        }
    }

    #[test]
    fn empty_and_refill_cycles_reuse_pool_slots() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let baseline = runtime.meshes().len();
        assert_eq!(baseline, 3, "three resident levels for one prototype");

        for cycle in 0..3 {
            clear_prototype(&mut scene, "boulder", 8);
            let emptied = runtime
                .prepare(&mut scene, &near_camera(), &LodConfig::default())
                .unwrap();
            assert!(emptied.geometry_changed, "cycle {cycle}");
            assert_eq!(
                runtime.meshes().len(),
                baseline,
                "emptying grew the pool in cycle {cycle}"
            );

            assert!(scene
                .edit_prototype("boulder", [0, 0, 0], material::BANK_STONE)
                .unwrap());
            let refilled = runtime
                .prepare(&mut scene, &near_camera(), &LodConfig::default())
                .unwrap();
            assert_eq!(
                refilled.instances.len(),
                refilled.frame.selected.len(),
                "cycle {cycle}"
            );
            assert_eq!(
                runtime.meshes().len(),
                baseline,
                "refilling grew the pool in cycle {cycle}"
            );
        }
    }

    #[test]
    fn prepare_never_mutates_the_authoritative_scene() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let version = scene.source_version();
        let counts = scene.counts();

        runtime
            .prepare(&mut scene, &far_camera(), &LodConfig::default())
            .unwrap();

        assert_eq!(scene.source_version(), version);
        assert_eq!(scene.counts(), counts);
    }

    #[test]
    fn lazy_runtime_builds_only_the_selected_levels() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::new();
        assert!(runtime.meshes().is_empty());

        let update = runtime
            .prepare(&mut scene, &far_camera(), &LodConfig::default())
            .unwrap();
        assert!(update.geometry_changed);
        let chosen = update.frame.selected[0].lod;
        assert!(chosen > Lod::Source, "far camera chose {chosen:?}");
        assert!(runtime.revision("boulder", chosen).is_some());
        for other in RESIDENT_LODS.iter().filter(|&&lod| lod != chosen) {
            assert!(
                runtime.revision("boulder", *other).is_none(),
                "{other:?} was built although it was never selected"
            );
        }
        assert_eq!(
            update.instances[0].prototype,
            runtime.instance_index("boulder", chosen).unwrap()
        );

        let again = runtime
            .prepare(&mut scene, &far_camera(), &LodConfig::default())
            .unwrap();
        assert!(!again.geometry_changed);
        assert_eq!(again.frame.mesh_builds_this_call, 0);
    }
}
