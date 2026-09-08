//! Deterministic host example of automatic view-dependent LOD selection.
//!
//! Usage: `cargo run -p matterweave-detail --example lod_sweep`
//!
//! It builds a fixed scene (a solid body plus a thin sheet, each placed twice),
//! sweeps a perspective camera along one axis, and prints a reproducible manifest
//! for each camera position: the per-level instance counts, the derived mesh bytes
//! per `(prototype, lod)` batch, and the authoritative source counts. The source
//! counts are printed every frame precisely to show they never change as the
//! camera moves: LOD is a render decision, world data is not.
//!
//! This is a HOST correctness/determinism artifact. It makes no performance claim
//! and does not touch a GPU, window or renderer. Equal inputs produce equal bytes.

use matterweave_detail::{
    material, Camera, DetailScene, DetailVolume, Lod, LodConfig, Projection, Result, Scale,
    Transform, Yaw, SCALE_FINE_M,
};

fn solid_block(id: &str, edge: i32, mat: u8) -> Result<DetailVolume> {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M)?);
    for x in 0..edge {
        for y in 0..edge {
            for z in 0..edge {
                v.set([x, y, z], mat)?;
            }
        }
    }
    Ok(v)
}

fn thin_sheet(id: &str, mat: u8) -> Result<DetailVolume> {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M)?);
    for x in 0..16 {
        for y in 0..32 {
            v.set([x, y, 0], mat)?;
        }
    }
    Ok(v)
}

/// Deterministic FNV-1a over the emitted text, so a reviewer can diff one hash.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn main() -> Result<()> {
    let mut scene = DetailScene::new();
    scene.add_prototype(solid_block("block", 8, material::BANK_STONE)?)?;
    scene.add_prototype(thin_sheet("sheet", material::MOSS_TURF)?)?;
    scene.place("block_a", "block", Transform::identity())?;
    scene.place(
        "block_b",
        "block",
        Transform::new([4.0, 0.0, 0.0], Yaw::Deg90)?,
    )?;
    scene.place(
        "sheet_a",
        "sheet",
        Transform::new([8.0, 0.0, 0.0], Yaw::Deg0)?,
    )?;
    scene.place(
        "sheet_b",
        "sheet",
        Transform::new([12.0, 0.0, 0.0], Yaw::Deg180)?,
    )?;

    let config = LodConfig::default();
    let counts = scene.counts();
    let mut out = String::new();
    out.push_str("# matterweave automatic LOD sweep (host, deterministic)\n");
    out.push_str(&format!(
        "config: budget_px={} hysteresis={} max_lod={:?} max_dilation={}\n",
        config.error_budget_px, config.hysteresis, config.max_lod, config.max_dilation_fraction
    ));
    out.push_str(&format!(
        "source(stable): prototypes={} instances={} unique_stored_cells={} source_bytes={} collision_cells={}\n",
        counts.prototypes,
        counts.instances,
        counts.unique_stored_cells,
        counts.source_bytes,
        counts.expanded_collision_cells,
    ));

    // Deliberately non-monotonic to exercise hysteresis: out, in, out, in.
    let distances = [2.0f32, 40.0, 120.0, 400.0, 120.0, 40.0, 2.0];
    for (frame, &distance) in distances.iter().enumerate() {
        let camera = Camera {
            eye_m: [0.25, 0.25, 0.5 + distance],
            forward_m: [0.0, 0.0, -1.0],
            viewport_height_px: 1080.0,
            near_m: 0.1,
            projection: Projection::Perspective {
                vertical_fov_rad: std::f32::consts::PI / 3.0,
            },
        };
        let prepared = scene.prepare_batches(&camera, &config)?;

        let mut level_counts = [0usize; 3];
        for item in &prepared.selected {
            level_counts[item.lod.index()] += 1;
        }
        out.push_str(&format!(
            "frame {frame} distance_m={distance}: source={} half={} quarter={} builds={} cache_bytes={}\n",
            level_counts[Lod::Source.index()],
            level_counts[Lod::Half.index()],
            level_counts[Lod::Quarter.index()],
            prepared.mesh_builds_this_call,
            prepared.cached_mesh_bytes,
        ));
        for batch in &prepared.batches {
            out.push_str(&format!(
                "  batch {}#{:?}: instances={} mesh_bytes={}\n",
                batch.prototype,
                batch.lod,
                batch.instances.len(),
                batch.mesh_bytes,
            ));
        }
        // Source authority is invariant to the camera; assert it here so the
        // example fails loudly if a future change leaks view state into source.
        let now = scene.counts();
        assert_eq!(now.source_bytes, counts.source_bytes);
        assert_eq!(now.unique_stored_cells, counts.unique_stored_cells);
        assert_eq!(
            now.expanded_collision_cells,
            counts.expanded_collision_cells
        );
    }

    out.push_str(&format!(
        "manifest_fnv1a=0x{:016x}\n",
        fnv1a(out.as_bytes())
    ));
    print!("{out}");
    Ok(())
}
