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
    pub fn load_recovering(
        path: &Path,
        generator: u32,
        seed: u64,
    ) -> Result<(std::path::PathBuf, Option<Self>), String> {
        match Self::load(path, generator, seed) {
            Ok(save) => return Ok((path.to_path_buf(), save)),
            Err(error) => log::warn!("Wetland session retained at {}: {error}", path.display()),
        }
        let name = path
            .file_name()
            .ok_or("Wetland save path has no filename")?
            .to_string_lossy();
        let mut available = None;
        let mut latest = None;
        for sequence in 1..=128 {
            let candidate = path.with_file_name(format!("{name}.recovery-{sequence}.json"));
            match Self::load(&candidate, generator, seed) {
                Ok(Some(save)) => latest = Some((candidate, Some(save))),
                Ok(None) if available.is_none() => available = Some(candidate),
                _ => {}
            }
        }
        if let Some(recovered) = latest {
            return Ok(recovered);
        }
        available
            .map(|p| (p, None))
            .ok_or_else(|| "Wetland recovery slots are full".into())
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

/// Bounded voxel DDA over ray-intersected prototype bounds. Called for actions,
/// not every frame. Decorative leaves and liquid do not mask editable solids.
pub fn raycast(
    scene: &DetailScene,
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
        let volume = scene.prototype(&draw.prototype)?;
        let Some(bounds) = volume.bounds_world(&draw.transform).ok().flatten() else {
            continue;
        };
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
            previous = Some(cell);
            for a in 0..3 {
                if crossings[a] <= next + 1e-6 {
                    cell[a] += if local_dir[a] > 0. { 1 } else { -1 };
                }
            }
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
        let hit = raycast(&scene, [-1.9375, 0.0625, -1.], [0., 0., -1.], 4.).unwrap();
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
        assert!(raycast(&scene, [f32::NAN, 0., 0.], [0., 0., 1.], 4.).is_none());
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
            raycast(&scene, [-1.9375, 0.0625, -1.], [0., 0., -1.], 4.)
                .unwrap()
                .instance,
            "stone"
        );
    }
}
