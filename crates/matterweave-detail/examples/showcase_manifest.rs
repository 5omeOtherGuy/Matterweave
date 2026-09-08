//! Generates the full showcase map and writes its content/resource manifest.
//!
//! Usage: `cargo run --release -p matterweave-detail --example showcase_manifest -- <out_dir>`
//! Default output directory: `docs/performance/showcase/fullmap-a1`.
//!
//! Every number written here is measured from the authoritative generated
//! scene in this process. Nothing is a target, a projection or a phone claim.

use matterweave_detail::{
    build_showcase, composition_hash, showcase_class, DetailError, Lod, MAX_PROTOTYPE_CELLS,
    MAX_SCENE_CACHE_BYTES, MAX_SCENE_SOURCE_BYTES, SHOWCASE_GENERATOR_VERSION, SHOWCASE_SEED,
    WALK_SPEED_M_S,
};
use serde_json::json;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "docs/performance/showcase/fullmap-a1".to_string());

    let build_start = Instant::now();
    let mut showcase = build_showcase(SHOWCASE_SEED)?;
    let build_ms = build_start.elapsed().as_secs_f64() * 1000.0;

    let counts = showcase.scene.counts();

    // Per-prototype Lod::Source mesh preflight and byte accounting. Each mesh is
    // built, measured and then dropped from the cache, so the aggregate below is
    // a measured sum, not a claim that all of them are resident at once.
    let mesh_start = Instant::now();
    let mut aggregate_source_mesh_bytes = 0usize;
    let mut largest_mesh_bytes = 0usize;
    let mut meshed = 0usize;
    for id in showcase.scene.prototype_ids() {
        showcase.scene.prototype_mesh(&id, Lod::Source)?;
        let bytes = showcase.scene.cached_mesh_bytes();
        aggregate_source_mesh_bytes += bytes;
        largest_mesh_bytes = largest_mesh_bytes.max(bytes);
        meshed += 1;
        showcase.scene.invalidate(&id);
        if showcase.scene.cached_mesh_bytes() != 0 {
            return Err(Box::new(DetailError::BudgetExceeded(
                "mesh cache not released",
            )));
        }
    }
    let mesh_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;

    // Resident working set actually held in the cache at once: the two shared
    // deep-base prototypes plus every flora prototype, i.e. the geometry a
    // renderer keeps for the whole map independent of streamed terrain.
    let mut shared_bytes = 0usize;
    for id in showcase.scene.prototype_ids() {
        if showcase_class(&id) == "terrain" && id.starts_with("shell_") {
            continue;
        }
        showcase.scene.prototype_mesh(&id, Lod::Source)?;
        shared_bytes = showcase.scene.cached_mesh_bytes();
    }

    let elevated_length_m: f32 = showcase
        .elevated_route
        .windows(2)
        .map(|w| {
            let d: [f32; 3] = std::array::from_fn(|a| w[1][a] - w[0][a]);
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .sum();

    let manifest = &showcase.manifest;
    let classes: serde_json::Map<String, serde_json::Value> = manifest
        .classes
        .iter()
        .map(|(name, c)| {
            (
                name.clone(),
                json!({
                    "prototypes": c.prototypes,
                    "instances": c.instances,
                    "unique_stored_cells": c.unique_stored_cells,
                    "instance_expanded_cells": c.expanded_cells,
                }),
            )
        })
        .collect();

    let document = json!({
        "artifact": "full-showcase",
        "generator": "matterweave_detail::showcase::build_showcase",
        "generator_version": SHOWCASE_GENERATOR_VERSION,
        "seed": SHOWCASE_SEED,
        "composition_hash": format!("{:016x}", composition_hash(&showcase)),
        "measured_on": "host; not a phone measurement",
        "debug_assertions": cfg!(debug_assertions),
        "timing_ms": {
            "generate_full_map": build_ms,
            "mesh_every_prototype_source_lod": mesh_ms,
            "meshed_prototypes": meshed,
        },
        "counts": {
            "prototypes": counts.prototypes,
            "instances": counts.instances,
            "unique_stored_cells": counts.unique_stored_cells,
            "instance_expanded_occupied_cells": counts.expanded_occupied_cells,
            "instance_expanded_collision_cells": counts.expanded_collision_cells,
            "instance_expanded_liquid_cells": counts.expanded_liquid_cells,
            "classes": classes,
            "species_instances": manifest.species_instances,
            "flora_instances": manifest.flora_instances,
            "flora_instance_expanded_cells": manifest.flora_expanded_cells,
            "terrain_instance_expanded_cells": manifest.terrain_expanded_cells,
            "terrain_shell_prototypes": manifest.terrain_shell_prototypes,
            "terrain_base_prototypes": manifest.terrain_base_prototypes,
            "terrain_base_instances": manifest.terrain_base_instances,
            "overhang_columns": manifest.overhang_columns,
        },
        "source": {
            "terrain_cell_m": matterweave_detail::TERRAIN_CELL_M,
            "source_bytes": counts.source_bytes,
            "source_budget_bytes": MAX_SCENE_SOURCE_BYTES,
            "largest_prototype_cells": manifest.largest_prototype_cells,
            "prototype_cell_ceiling": MAX_PROTOTYPE_CELLS,
        },
        "cache": {
            "aggregate_source_mesh_bytes_measured_one_at_a_time": aggregate_source_mesh_bytes,
            "largest_single_prototype_mesh_bytes": largest_mesh_bytes,
            "resident_shared_prototype_mesh_bytes": shared_bytes,
            "cache_budget_bytes": MAX_SCENE_CACHE_BYTES,
            "aggregate_fits_cache_budget": aggregate_source_mesh_bytes <= MAX_SCENE_CACHE_BYTES,
        },
        "navigation": {
            "walk_speed_m_s": WALK_SPEED_M_S,
            "ground_loop_length_m": manifest.route_length_m,
            "ground_loop_seconds_host_estimate": manifest.route_walk_seconds,
            "ground_loop_points": showcase.route.len(),
            "waterside_route_m": showcase.waterside_route(),
            "ground_route_m": showcase.route,
            "elevated_route_m": showcase.elevated_route,
            "elevated_route_length_m": elevated_length_m,
            "spawn_eye_m": showcase.spawn_eye,
        },
        "landmarks": showcase.landmarks.iter().map(|l| json!({
            "name": l.name, "kind": l.kind, "position_m": l.position_m,
        })).collect::<Vec<_>>(),
    });

    std::fs::create_dir_all(&out_dir)?;
    let path = std::path::Path::new(&out_dir).join("manifest.json");
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )?;
    println!("wrote {}", path.display());
    println!("{}", serde_json::to_string_pretty(&document["counts"])?);
    println!("{}", serde_json::to_string_pretty(&document["cache"])?);
    Ok(())
}
