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
//! - No stale derived geometry is handed out. An instance is only returned when
//!   its resident mesh carries the prototype's live source revision; a stale
//!   entry is refreshed from the freshly built scene-cache mesh first.
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
use matterweave_detail::{Camera, DetailScene, Lod, LodConfig, PreparedFrame, SceneVersion, Yaw};
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
    /// Packed instances referencing resident mesh-pool indices.
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
    /// revision, and records each `(prototype, lod)` -> pool-index map. An
    /// empty scene produces an empty runtime, not an error.
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
        let instances = self.instances_for_frame(&frame)?;
        Ok(FrameUpdate {
            frame,
            instances,
            geometry_changed,
        })
    }

    /// Maps a prepared frame's per-instance LOD selection to packed instances
    /// that reference resident geometry. Only the instance list changes between
    /// frames; geometry is never rebuilt here. A frame prepared from a
    /// different source state than this runtime's resident geometry is
    /// rejected, never mis-mapped.
    pub fn instances_for_frame(
        &self,
        frame: &PreparedFrame,
    ) -> Result<Vec<StaticInstance>, String> {
        if frame.source_version != self.source_version {
            return Err("prepared frame and resident geometry source versions differ".into());
        }
        let mut out = Vec::with_capacity(frame.selected.len());
        for item in &frame.selected {
            let prototype = self
                .instance_index(&item.prototype, item.lod)
                .ok_or_else(|| {
                    format!(
                        "selected {} {:?} has no resident geometry",
                        item.prototype, item.lod
                    )
                })?;
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
    /// resident pool changed. A selected prototype with no occupied cells has
    /// no drawable geometry: any resident copy is dropped, so
    /// [`Self::instances_for_frame`] reports it rather than handing out stale
    /// or vertexless geometry.
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
            if source.occupied_cells() == 0 {
                // No drawable geometry. Drop any resident copy so an instance of
                // an emptied prototype is reported rather than drawn from the
                // stale mesh this entry referred to.
                if self.prototypes.remove(&item.prototype).is_some() {
                    changed = true;
                }
                continue;
            }
            self.refresh_entry(scene, &item.prototype, item.lod, source_revision)?;
            changed = true;
        }
        Ok(changed)
    }

    /// Copies one freshly realized derived mesh out of the scene cache into the
    /// resident pool, after verifying it carries `source_revision`.
    fn refresh_entry(
        &mut self,
        scene: &mut DetailScene,
        prototype: &str,
        lod: Lod,
        source_revision: u64,
    ) -> Result<(), String> {
        // Realize (or reuse) the mesh in the engine cache first; drops the
        // `&mut` borrow at the semicolon.
        scene
            .prototype_mesh(prototype, lod)
            .map_err(|e| format!("build {prototype} {lod:?}: {e}"))?;
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
        Ok(())
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
    use matterweave_detail::{material, DetailVolume, Projection, Scale, Transform, SCALE_FINE_M};

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

    /// One dense 8^3 boulder at the origin. Dense solids carry no coarsening
    /// guard, so the engine's distance rule decides between `Source` and
    /// `Quarter` without a hand-written selection rule in this test.
    fn boulder_scene() -> DetailScene {
        let mut volume =
            DetailVolume::new("boulder", Scale::new(SCALE_FINE_M).expect("fine scale"));
        for x in 0..8 {
            for y in 0..8 {
                for z in 0..8 {
                    volume
                        .set([x, y, z], material::BANK_STONE)
                        .expect("solid cell");
                }
            }
        }
        let mut scene = DetailScene::new();
        scene.add_prototype(volume).expect("add boulder");
        scene
            .place("near", "boulder", Transform::identity())
            .expect("place boulder");
        scene
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
    fn emptied_prototype_is_reported_instead_of_drawn_from_stale_geometry() {
        let mut scene = boulder_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        assert!(runtime.instance_index("boulder", Lod::Source).is_some());

        // Remove every occupied cell; the instance still references the prototype.
        for x in 0..8 {
            for y in 0..8 {
                for z in 0..8 {
                    assert!(scene
                        .edit_prototype("boulder", [x, y, z], material::AIR)
                        .unwrap());
                }
            }
        }
        assert_eq!(scene.prototype("boulder").unwrap().occupied_cells(), 0);

        let result = runtime.prepare(&mut scene, &near_camera(), &LodConfig::default());
        assert!(
            result.is_err(),
            "an emptied prototype must not be drawn from stale geometry"
        );
        assert!(runtime.instance_index("boulder", Lod::Source).is_none());
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
