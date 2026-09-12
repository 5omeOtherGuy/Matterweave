//! Bounded, separate wetland save journal and source-space interaction queries.
use matterweave_detail::{material, Bounds, DetailScene, MaterialPolicy};
use matterweave_physics::PhysicsSnapshot;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub const SAVE_FILE: &str = "wetland-session.json";
pub const MAX_EDITS: usize = 4096;
const MAX_BYTES: usize = 2 * 1024 * 1024;
static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub instance: String,
    pub cell: [i32; 3],
    pub material: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedWetland {
    pub version: u32,
    pub generator: u32,
    pub seed: u64,
    pub edits: Vec<Edit>,
    pub physics: PhysicsSnapshot,
    pub yaw: f32,
    pub pitch: f32,
    pub shadows: bool,
}

impl SavedWetland {
    pub fn validate(&self, generator: u32, seed: u64) -> Result<(), String> {
        if self.version != 1
            || self.generator != generator
            || self.seed != seed
            || self.edits.len() > MAX_EDITS
            || !self.yaw.is_finite()
            || !self.pitch.is_finite()
            || self.pitch.abs() > 1.5
            || self.physics.version != 1
            || self.physics.bodies.len() > 64
            || self
                .physics
                .eye
                .iter()
                .any(|n| !n.is_finite() || n.abs() > 16384.)
        {
            return Err("Wetland save has unsupported or invalid values".into());
        }
        for edit in &self.edits {
            if edit.instance.len() > 128
                || edit.instance.is_empty()
                || edit.cell.iter().any(|n| !(-65536..=65536).contains(n))
            {
                return Err("Wetland edit is outside the source bounds".into());
            }
        }
        Ok(())
    }

    pub fn load(path: &Path, generator: u32, seed: u64) -> Result<Option<Self>, String> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Wetland save exceeds byte limit".into());
        }
        let save: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        save.validate(generator, seed)?;
        Ok(Some(save))
    }

    /// Select a valid session or a fresh recovery path. Existing invalid files
    /// stay byte-for-byte intact; recovery selection is bounded and deterministic.
    /// Shallow-only selection kept for callers without a live scene/physics
    /// pair (covered by tests); the wetland runtime uses
    /// [`Self::load_recovering_with`] with real restoration instead.
    #[allow(dead_code)]
    pub fn load_recovering(
        path: &Path,
        generator: u32,
        seed: u64,
    ) -> Result<(std::path::PathBuf, Option<Self>), String> {
        Self::load_recovering_with(path, generator, seed, |_| Ok(()))
    }

    /// Recovery selection where a candidate is accepted only once `restore` has
    /// actually applied it to the live scene and physics. Shallow field checks
    /// cannot see an edit naming a placement that does not exist, a body payload
    /// the physics contract rejects, or any other restoration failure, so a
    /// journal that passes [`Self::validate`] and still cannot be restored is
    /// treated exactly like corrupt JSON: retained byte-for-byte, skipped, and
    /// replaced by an older valid session or a fresh recovery slot.
    ///
    /// `restore` must leave no effect behind when it returns `Err`; only the
    /// accepted candidate's effects survive this call. Candidates are visited
    /// newest-first so the first acceptance is the selected one and accepted
    /// state never has to be rolled back for a later winner.
    pub fn load_recovering_with(
        path: &Path,
        generator: u32,
        seed: u64,
        mut restore: impl FnMut(&Self) -> Result<(), String>,
    ) -> Result<(std::path::PathBuf, Option<Self>), String> {
        match Self::load(path, generator, seed) {
            Ok(None) => return Ok((path.to_path_buf(), None)),
            Ok(Some(save)) => match restore(&save) {
                Ok(()) => return Ok((path.to_path_buf(), Some(save))),
                Err(error) => log::warn!(
                    "Wetland session not restorable at {}: {error}",
                    path.display()
                ),
            },
            Err(error) => log::warn!("Wetland session retained at {}: {error}", path.display()),
        }
        let name = path
            .file_name()
            .ok_or("Wetland save path has no filename")?
            .to_string_lossy()
            .into_owned();
        let slot = |sequence: u32| path.with_file_name(format!("{name}.recovery-{sequence}.json"));
        for sequence in (1..=128).rev() {
            let candidate = slot(sequence);
            match Self::load(&candidate, generator, seed) {
                Ok(Some(save)) => match restore(&save) {
                    Ok(()) => return Ok((candidate, Some(save))),
                    Err(error) => log::warn!(
                        "Wetland recovery not restorable at {}: {error}",
                        candidate.display()
                    ),
                },
                Ok(None) => {}
                Err(error) => log::warn!(
                    "Wetland recovery retained at {}: {error}",
                    candidate.display()
                ),
            }
        }
        for sequence in 1..=128 {
            let candidate = slot(sequence);
            if matches!(Self::load(&candidate, generator, seed), Ok(None)) {
                return Ok((candidate, None));
            }
        }
        Err("Wetland recovery slots are full".into())
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate(self.generator, self.seed)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Wetland save exceeds byte limit".into());
        }
        let temporary = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|e| e.to_string())
    }
}

#[derive(Debug)]
pub struct Hit {
    pub instance: String,
    pub cell: [i32; 3],
    pub previous: Option<[i32; 3]>,
    pub distance: f32,
    pub material: u8,
}

fn interval(bounds: Bounds, origin: [f32; 3], direction: [f32; 3], max: f32) -> Option<(f32, f32)> {
    let (mut near, mut far) = (0f32, max);
    for a in 0..3 {
        if direction[a].abs() < 1e-8 {
            if origin[a] < bounds.min[a] || origin[a] > bounds.max[a] {
                return None;
            }
        } else {
            let t0 = (bounds.min[a] - origin[a]) / direction[a];
            let t1 = (bounds.max[a] - origin[a]) / direction[a];
            near = near.max(t0.min(t1));
            far = far.min(t0.max(t1));
        }
    }
    (far >= near).then_some((near, far))
}

/// Bounded voxel DDA over ray-intersected instance bounds. Called for actions,
/// not every frame. Decorative leaves and liquid do not mask editable solids.
///
/// Instance bounds come from [`DetailScene::instance_bounds_world`], which uses
/// the scene's cached per-revision digest instead of re-censusing every
/// occupied cell of a shared prototype for each of its placements. The full
/// wetland measures 8302 placements over 893 prototype sources holding 3189
/// chunks, so the old per-instance `bounds_world` path paid up to
/// 228,188,160 cell reads per ray where the distinct sources hold 13,062,144
/// — the same full censuses roughly 17.5 times over, on the input thread.
/// Selection (`select_lods`) has already refreshed every digest before an
/// action can run, so this path performs no census work at all.
pub fn raycast(
    scene: &mut DetailScene,
    origin: [f32; 3],
    direction: [f32; 3],
    distance: f32,
) -> Option<Hit> {
    if origin.iter().chain(&direction).any(|n| !n.is_finite()) || !(0.0..=32.0).contains(&distance)
    {
        return None;
    }
    let norm = direction.iter().map(|n| n * n).sum::<f32>().sqrt();
    if !norm.is_finite() || norm < 1e-6 {
        return None;
    }
    let direction = direction.map(|n| n / norm);
    let mut best: Option<Hit> = None;
    for draw in scene.draws() {
        // A missing prototype means the scene changed under the query; abort
        // the whole raycast (unchanged contract), never skip one draw.
        scene.prototype(&draw.prototype)?;
        let bounds = match scene.instance_bounds_world(&draw.instance) {
            Ok(Some(bounds)) => bounds,
            // An unusable transform or empty source is never a hit: skip the
            // draw, exactly as the previous per-instance bounds handling did.
            Ok(None) | Err(_) => continue,
        };
        let volume = scene.prototype(&draw.prototype)?;
        let Some((near, far)) = interval(
            bounds,
            origin,
            direction,
            best.as_ref().map_or(distance, |h| h.distance),
        ) else {
            continue;
        };
        let local_origin = draw.transform.point_to_local(origin);
        let end = draw
            .transform
            .point_to_local(std::array::from_fn(|a| origin[a] + direction[a]));
        let local_dir: [f32; 3] = std::array::from_fn(|a| end[a] - local_origin[a]);
        let scale = volume.scale().metres();
        let mut t = near + scale * 0.0001;
        let mut cell: [i32; 3] =
            std::array::from_fn(|a| ((local_origin[a] + local_dir[a] * t) / scale).floor() as i32);
        // Recover the face immediately outside the bounds for placement when
        // the first traversed cell is solid. Origins inside a solid have no face.
        let mut previous = None;
        if near > scale * 0.0001 {
            let before: [i32; 3] = std::array::from_fn(|a| {
                ((local_origin[a] + local_dir[a] * (near - scale * 0.0001)) / scale).floor() as i32
            });
            // Deterministic face at an edge/corner, never a diagonal placement.
            if let Some(a) = (0..3).find(|&a| before[a] != cell[a]) {
                let mut adjacent = cell;
                adjacent[a] += if local_dir[a] > 0. { -1 } else { 1 };
                previous = Some(adjacent);
            }
        }
        for _ in 0..2048 {
            if t > far + 1e-5 {
                break;
            }
            let m = volume.get(cell);
            if m != material::AIR
                && matterweave_detail::material_policy(m) == MaterialPolicy::Collision
            {
                best = Some(Hit {
                    instance: draw.instance.clone(),
                    cell,
                    previous,
                    distance: t,
                    material: m,
                });
                break;
            }
            let crossings: [f32; 3] = std::array::from_fn(|a| {
                if local_dir[a].abs() < 1e-8 {
                    f32::INFINITY
                } else {
                    let edge = cell[a] + i32::from(local_dir[a] > 0.);
                    (edge as f32 * scale - local_origin[a]) / local_dir[a]
                }
            });
            let next = crossings.into_iter().fold(f32::INFINITY, f32::min);
            if !next.is_finite() {
                break;
            }
            for a in 0..3 {
                if crossings[a] <= next + 1e-6 {
                    cell[a] += if local_dir[a] > 0. { 1 } else { -1 };
                }
            }
            // Advance all tied axes to avoid false hits on cells touched only
            // at an edge. Placement still needs one exposed face, not the
            // diagonal cell occupied before a corner crossing.
            previous = (0..3).find_map(|a| {
                if crossings[a] > next + 1e-6 {
                    return None;
                }
                let mut adjacent = cell;
                adjacent[a] += if local_dir[a] > 0. { -1 } else { 1 };
                (matterweave_detail::material_policy(volume.get(adjacent))
                    != MaterialPolicy::Collision)
                    .then_some(adjacent)
            });
            t = next;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_detail::{DetailVolume, Scale, Transform, Yaw};
    #[test]
    fn save_roundtrip_is_separate_and_rejects_corrupt_or_oversized_data() {
        let dir = std::env::temp_dir().join(format!(
            "matterweave-wetland-state-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SAVE_FILE);
        let legacy = dir.join("world.json");
        fs::write(&legacy, b"existing sandbox sentinel").unwrap();
        let mut save = SavedWetland {
            version: 1,
            generator: 1,
            seed: 7,
            edits: vec![Edit {
                instance: "plant-1".into(),
                cell: [-1, 0, 1],
                material: 0,
            }],
            physics: PhysicsSnapshot {
                version: 1,
                eye: [1., 2., 3.],
                bodies: vec![],
            },
            yaw: 0.5,
            pitch: 0.1,
            shadows: true,
        };
        save.save(&path).unwrap();
        let read = SavedWetland::load(&path, 1, 7).unwrap().unwrap();
        assert_eq!(read.edits, save.edits);
        assert_eq!(read.physics.eye, save.physics.eye);
        assert!(SavedWetland::load(&path, 1, 8).is_err());
        let original = fs::read(&path).unwrap();
        save.pitch = f32::NAN;
        assert!(save.save(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::write(&path, b"{not json}").unwrap();
        assert!(SavedWetland::load(&path, 1, 7).is_err());
        fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(SavedWetland::load(&path, 1, 7).is_err());
        assert_eq!(fs::read(&legacy).unwrap(), b"existing sandbox sentinel");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn corrupt_wetland_recovers_without_overwriting_prior_sessions() {
        let dir = std::env::temp_dir().join(format!(
            "wetland-recovery-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join(SAVE_FILE);
        fs::write(&path, b"broken original").unwrap();
        let (first, save) = SavedWetland::load_recovering(&path, 1, 7).unwrap();
        assert!(save.is_none());
        assert_ne!(first, path);
        fs::write(&first, b"broken recovery").unwrap();
        let (second, save) = SavedWetland::load_recovering(&path, 1, 7).unwrap();
        assert!(save.is_none());
        assert_ne!(first, second);
        let saved = SavedWetland {
            version: 1,
            generator: 1,
            seed: 7,
            edits: vec![],
            physics: PhysicsSnapshot {
                version: 1,
                eye: [2., 3., 4.],
                bodies: vec![],
            },
            yaw: 0.2,
            pitch: 0.1,
            shadows: true,
        };
        saved.save(&second).unwrap();
        let (selected, save) = SavedWetland::load_recovering(&path, 1, 7).unwrap();
        assert_eq!(selected, second);
        assert_eq!(save.unwrap().physics.eye, [2., 3., 4.]);
        let (next_version, save) = SavedWetland::load_recovering(&path, 2, 7).unwrap();
        assert!(save.is_none());
        assert_ne!(next_version, second);
        assert_eq!(fs::read(&path).unwrap(), b"broken original");
        assert_eq!(fs::read(&first).unwrap(), b"broken recovery");
        assert!(SavedWetland::load(&second, 1, 7).unwrap().is_some());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn old_showcase_sessions_are_not_replayed_on_changed_layout() {
        let dir = std::env::temp_dir().join(format!(
            "wetland-layout-version-{}-{}",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join(SAVE_FILE);
        let old = SavedWetland {
            version: 1,
            generator: 1,
            seed: matterweave_detail::SHOWCASE_SEED,
            edits: vec![Edit {
                instance: "flora_989_11".into(),
                cell: [0, 1, 0],
                material: 0,
            }],
            physics: PhysicsSnapshot {
                version: 1,
                eye: [46., 14., 71.],
                bodies: vec![],
            },
            yaw: 0.,
            pitch: 0.,
            shadows: true,
        };
        old.save(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let (selected, save) = SavedWetland::load_recovering(
            &path,
            matterweave_detail::SHOWCASE_GENERATOR_VERSION,
            matterweave_detail::SHOWCASE_SEED,
        )
        .unwrap();
        assert_ne!(
            selected, path,
            "changed placement catalog reuses old generator identity"
        );
        assert!(
            save.is_none(),
            "old instance edits must not attach to new placements"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ray_hits_transformed_source_and_skips_decorative_overlap() {
        let mut scene = DetailScene::new();
        let mut stone = DetailVolume::new("stone", Scale::new(0.125).unwrap());
        stone.set([-2, 0, 0], material::BANK_STONE).unwrap();
        scene.add_prototype(stone).unwrap();
        scene
            .place(
                "stone",
                "stone",
                Transform::new([-2., 0., -2.], Yaw::Deg90).unwrap(),
            )
            .unwrap();
        let hit = raycast(&mut scene, [-1.9375, 0.0625, -1.], [0., 0., -1.], 4.).unwrap();
        assert_eq!(hit.cell, [-2, 0, 0]);
        assert_eq!(hit.instance, "stone");
        assert!(hit.previous.is_some());
        assert_eq!(
            hit.previous
                .unwrap()
                .iter()
                .zip(hit.cell)
                .map(|(a, b)| (a - b).abs())
                .sum::<i32>(),
            1
        );
        assert!(raycast(&mut scene, [f32::NAN, 0., 0.], [0., 0., 1.], 4.).is_none());
        let mut leaf = DetailVolume::new("leaf", Scale::new(0.125).unwrap());
        leaf.set([-2, 0, 0], material::FLORA_FROND_BLADE).unwrap();
        scene.add_prototype(leaf).unwrap();
        scene
            .place(
                "a-leaf",
                "leaf",
                Transform::new([-2., 0., -2.], Yaw::Deg90).unwrap(),
            )
            .unwrap();
        assert_eq!(
            raycast(&mut scene, [-1.9375, 0.0625, -1.], [0., 0., -1.], 4.)
                .unwrap()
                .instance,
            "stone"
        );
    }
    #[test]
    fn internal_corner_hits_offer_only_face_adjacent_placement() {
        let mut scene = DetailScene::new();
        let mut volume = DetailVolume::new("corners", Scale::new(0.25).unwrap());
        volume.set([-1, -1, 0], material::BANK_STONE).unwrap();
        volume.set([1, 1, 0], material::BANK_STONE).unwrap();
        scene.add_prototype(volume).unwrap();
        scene
            .place("corner", "corners", Transform::identity())
            .unwrap();
        for direction in [[1., 1., 0.], [-1., -1., 0.]] {
            let hit = raycast(&mut scene, [0.125; 3], direction, 2.).unwrap();
            let previous = hit.previous.unwrap();
            assert_eq!(
                previous
                    .iter()
                    .zip(hit.cell)
                    .map(|(a, b)| (a - b).abs())
                    .sum::<i32>(),
                1,
                "corner hit gives diagonal placement: {:?} -> {:?}",
                hit.cell,
                previous
            );
        }
        // At an exact corner, an occupied side is not an exposed placement face.
        scene
            .edit_instance("corner", [0, 1, 0], material::BANK_STONE)
            .unwrap();
        let hit = raycast(&mut scene, [0.125; 3], [1., 1., 0.], 2.).unwrap();
        assert_eq!(hit.previous, Some([1, 0, 0]));
        scene
            .edit_instance("corner", [1, 0, 0], material::BANK_STONE)
            .unwrap();
        assert!(raycast(&mut scene, [0.125; 3], [1., 1., 0.], 2.)
            .unwrap()
            .previous
            .is_none());
    }

    /// Regression for the reported Android interaction ANR: the action ray
    /// must resolve instance bounds from the scene's cached per-revision
    /// digests, not re-census every occupied cell of a shared prototype for
    /// each of its placements. `digest_builds` counts actual recomputations,
    /// so a single cold ray over many placements of one prototype costs one
    /// digest build (not one per placement), a warm ray costs none, and an
    /// edit recomputes only the prototype whose source changed.
    #[test]
    fn action_raycast_resolves_bounds_from_cached_revision_digests() {
        let mut scene = DetailScene::new();
        let mut volume = DetailVolume::new("rock", Scale::new(0.25).unwrap());
        volume.set([-2, 0, 0], material::BANK_STONE).unwrap();
        volume.set([1, 0, 0], material::BANK_STONE).unwrap();
        scene.add_prototype(volume).unwrap();
        for i in 0..64 {
            scene
                .place(
                    format!("rock-{i}"),
                    "rock",
                    Transform::new([4. * i as f32, 0., 0.], Yaw::Deg0).unwrap(),
                )
                .unwrap();
        }
        // A differently transformed placement of the same source must not
        // cost a second digest build: the cache is per prototype revision.
        scene
            .place(
                "rock-turned",
                "rock",
                Transform::new([200., 0., -100.], Yaw::Deg180).unwrap(),
            )
            .unwrap();
        assert_eq!(scene.digest_builds(), 0);
        // Instance 3's [-2,0,0] cell spans x [11.5, 11.75), y/z [0, 0.25).
        let origin = [11.6, 0.125, -2.];
        let direction = [0., 0., 1.];
        let hit = raycast(&mut scene, origin, direction, 4.).unwrap();
        assert_eq!(hit.instance, "rock-3");
        assert_eq!(hit.cell, [-2, 0, 0]);
        // One prototype source, one digest build — never one per placement.
        assert_eq!(scene.digest_builds(), 1);
        let _ = raycast(&mut scene, origin, direction, 4.);
        assert_eq!(
            scene.digest_builds(),
            1,
            "warm ray re-censused prototype sources"
        );
        // A copy-on-write edit mints a private prototype; only that source is
        // recomputed (lazily, on the next ray) and the ray must then see the
        // edited geometry.
        scene
            .edit_instance("rock-3", [-2, 0, 0], material::AIR)
            .unwrap();
        assert!(raycast(&mut scene, origin, direction, 4.).is_none());
        assert_eq!(scene.digest_builds(), 2);
        // An edit adding solid material OUTSIDE the previously cached bounds
        // must grow them: a ray that hits only the new cell cannot pass if a
        // stale, too-small bound skips the draw entirely. Instance 3's new
        // [1,5,5] cell spans x [12.25,12.5), y/z [1.25,1.5).
        scene
            .edit_instance("rock-3", [1, 5, 5], material::BANK_STONE)
            .unwrap();
        let hit = raycast(&mut scene, [12.3, 1.375, -2.], direction, 4.).unwrap();
        assert_eq!(hit.instance, "rock-3");
        assert_eq!(hit.cell, [1, 5, 5]);
        assert!(hit.previous.is_some());
        assert_eq!(scene.digest_builds(), 3);
    }
}
