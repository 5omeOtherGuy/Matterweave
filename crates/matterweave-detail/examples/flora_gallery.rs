//! Writes the deterministic flora gallery: content manifest, raw mesh buffers
//! and prototype snapshots for the dense 16 m representative tile, plus a
//! separate host timing file.
//!
//! Usage: `cargo run -p matterweave-detail --example flora_gallery -- <out_dir>`
//! No image, window or renderer dependency is used.
//!
//! The scene itself lives in [`matterweave_detail::dense_tile`], a public
//! reusable function, so a native adapter or test builds exactly the same
//! scene without this example.
//!
//! `manifest.json` contains only reproducible content: equal inputs give equal
//! bytes. Host measurements go to `timing.json` and are desktop HOST ONLY
//! numbers, never phone performance. Visual acceptance is NOT performed here.

use matterweave_detail::*;
use std::collections::BTreeMap;
use std::time::Instant;

/// Little-endian by construction, so exported buffers do not depend on host
/// endianness the way a raw `bytemuck` cast of the vertex struct would.
fn vertices_le(mesh: &matterweave_core::Mesh) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(mesh.vertices.len() * 36);
    for vertex in &mesh.vertices {
        for value in vertex
            .position
            .iter()
            .chain(&vertex.normal)
            .chain(&vertex.color)
        {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn indices_le(mesh: &matterweave_core::Mesh) -> Vec<u8> {
    mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect()
}

fn main() -> Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| {
        "/mnt/bench/matterweave-dev/performance/desktop-02/flora-gallery-a2".to_string()
    });
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("create output directory");

    let seed = FLORA_CANONICAL_SEED;
    let build_started = Instant::now();
    let mut scene = dense_tile(seed)?;
    let build_ms = build_started.elapsed().as_secs_f64() * 1e3;

    let mut meshes = Vec::new();
    let mut mesh_timings = Vec::new();
    let mut mesh_bytes_total = 0usize;
    for id in scene.prototype_ids() {
        let base_scale = scene.prototype(&id).unwrap().scale().metres();
        for lod in [Lod::Source, Lod::Half, Lod::Quarter] {
            let started = Instant::now();
            let mesh = scene.prototype_mesh(&id, lod)?;
            let ms = started.elapsed().as_secs_f64() * 1e3;
            let vertices = vertices_le(mesh);
            let indices = indices_le(mesh);
            let name = format!("{id}.lod{}", lod.factor());
            std::fs::write(out.join(format!("{name}.vertices.le.bin")), &vertices)
                .expect("write vertices");
            std::fs::write(out.join(format!("{name}.indices.le.bin")), &indices)
                .expect("write indices");
            mesh_bytes_total += vertices.len() + indices.len();
            meshes.push(serde_json::json!({
                "prototype": id,
                "class": flora_class(&id),
                "lod_factor": lod.factor(),
                "cell_size_m": base_scale * lod.factor() as f32,
                "source_revision": mesh.revision,
                "vertices": mesh.vertices.len(),
                "triangles": mesh.indices.len() / 3,
                "vertex_bytes": vertices.len(),
                "index_bytes": indices.len(),
                "vertex_file": format!("{name}.vertices.le.bin"),
                "index_file": format!("{name}.indices.le.bin"),
                "byte_order": "little_endian",
            }));
            mesh_timings.push(serde_json::json!({ "mesh": name, "host_only_ms": ms }));
        }
    }

    let mut prototypes = Vec::new();
    let mut snapshot_bytes_total = 0usize;
    let mut scene_bounds_min = [f32::INFINITY; 3];
    let mut scene_bounds_max = [f32::NEG_INFINITY; 3];
    for id in scene.prototype_ids() {
        let volume = scene.prototype(&id).unwrap();
        let snapshot = volume.snapshot();
        let json = serde_json::to_vec(&snapshot).expect("serialize snapshot");
        snapshot_bytes_total += json.len();
        std::fs::write(out.join(format!("{id}.snapshot.json")), &json).expect("write snapshot");
        let bounds = volume.bounds_local().expect("non-empty prototype");
        let mut by_material: BTreeMap<&str, usize> = BTreeMap::new();
        let mut policies: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, material) in volume.iter_cells() {
            *by_material.entry(material_name(material)).or_default() += 1;
            let policy = match material_policy(material) {
                MaterialPolicy::Collision => "collision",
                MaterialPolicy::Liquid => "liquid",
                MaterialPolicy::Decorative => "decorative",
            };
            *policies.entry(policy).or_default() += 1;
        }
        prototypes.push(serde_json::json!({
            "id": id,
            "class": flora_class(&id),
            "cell_size_m": volume.scale().metres(),
            "occupied_cells": volume.occupied_cells(),
            "fits_prototype_mesh_cap_33288": volume.occupied_cells() < 33_288,
            "chunks": volume.chunk_count(),
            "source_payload_bytes": volume.source_bytes(),
            "snapshot_json_bytes": json.len(),
            "snapshot_runs": snapshot.runs.len(),
            "revision": volume.revision(),
            "bounds_local_m": { "min": bounds.min, "max": bounds.max },
            "cells_by_material": by_material,
            "cells_by_policy": policies,
        }));
    }

    // Ground contact evidence and scene bounds, from authoritative source queries.
    let tile = scene
        .prototype("terrain_tile_16m")
        .expect("tile prototype")
        .clone();
    let mut contact = Vec::new();
    let mut instances_by_prototype: BTreeMap<String, usize> = BTreeMap::new();
    let draws: Vec<serde_json::Value> = scene
        .draws()
        .into_iter()
        .map(|draw| {
            *instances_by_prototype
                .entry(draw.prototype.clone())
                .or_default() += 1;
            let volume = scene.prototype(&draw.prototype).expect("placed prototype");
            if let Ok(Some(bounds)) = volume.bounds_world(&draw.transform) {
                for axis in 0..3 {
                    scene_bounds_min[axis] = scene_bounds_min[axis].min(bounds.min[axis]);
                    scene_bounds_max[axis] = scene_bounds_max[axis].max(bounds.max[axis]);
                }
            }
            if draw.prototype != "terrain_tile_16m" {
                match instance_support(&tile, volume, &draw.prototype, &draw.transform) {
                    Some((top, material, gap)) => contact.push(serde_json::json!({
                        "instance": draw.instance,
                        "prototype": draw.prototype,
                        "support_top_cell": top,
                        "support_material": material,
                        "gap_m": gap,
                    })),
                    None => contact.push(serde_json::json!({
                        "instance": draw.instance,
                        "prototype": draw.prototype,
                        "support": "UNSUPPORTED",
                    })),
                }
            }
            serde_json::json!({
                "instance": draw.instance,
                "prototype": draw.prototype,
                "class": flora_class(&draw.prototype),
                "translation_m": draw.transform.translation_m,
                "yaw": format!("{:?}", draw.transform.yaw),
                "occupied_cells": draw.occupied_cells,
            })
        })
        .collect();

    let counts = scene.counts();
    let tile_cells = tile.occupied_cells();
    let flora_cells = counts.expanded_occupied_cells - tile_cells;
    let vegetation = counts.instances - 1;
    let distinct_types = instances_by_prototype
        .iter()
        .filter(|(id, count)| id.as_str() != "terrain_tile_16m" && **count > 0)
        .count();
    let thresholds_met = vegetation >= DENSE_VEGETATION_MIN
        && flora_cells >= DENSE_FLORA_CELLS_MIN
        && distinct_types >= DENSE_TYPES_MIN
        && FLORA_SPECIES.iter().all(|id| {
            instances_by_prototype.get(*id).copied().unwrap_or(0) >= DENSE_PER_SPECIES_MIN
        });
    let manifest = serde_json::json!({
        "generator_version": FLORA_GENERATOR_VERSION,
        "fixture_generator_version": FIXTURE_GENERATOR_VERSION,
        "snapshot_version": SNAPSHOT_VERSION,
        "seed": seed,
        "content_only": "deterministic; host timings are in timing.json",
        "scene_note": "dense 16 m representative tile: a density sample toward the full 128 m map goal, NOT full showcase acceptance and NOT a phone claim",
        "origin_convention": "local cell c spans [c*scale, (c+1)*scale] metres; transform is quarter-turn yaw then an arbitrary fractional translation in world metres",
        "species": FLORA_SPECIES,
        "reserved_species": ["bracket_shelf"],
        "counts": {
            "prototypes": counts.prototypes,
            "instances": counts.instances,
            "vegetation_instances": vegetation,
            "instances_by_prototype": instances_by_prototype,
            "distinct_flora_types": distinct_types,
            "unique_stored_cells": counts.unique_stored_cells,
            "expanded_occupied_cells": counts.expanded_occupied_cells,
            "terrain_cells": tile_cells,
            "expanded_flora_cells": flora_cells,
            "expanded_collision_cells": counts.expanded_collision_cells,
            "expanded_liquid_cells": counts.expanded_liquid_cells,
            "derived_mesh_builds": counts.mesh_builds,
            "excluded_from_flora_cells": "the single terrain tile instance",
        },
        "thresholds": {
            "vegetation_min": DENSE_VEGETATION_MIN,
            "flora_cells_min": DENSE_FLORA_CELLS_MIN,
            "types_min": DENSE_TYPES_MIN,
            "per_species_min": DENSE_PER_SPECIES_MIN,
            "met": thresholds_met,
        },
        "camera": {
            "note": "suggested inspection framing only; no renderer is involved here",
            "scene_bounds_m": { "min": scene_bounds_min, "max": scene_bounds_max },
            "target_m": [
                (scene_bounds_min[0] + scene_bounds_max[0]) * 0.5,
                (scene_bounds_min[1] + scene_bounds_max[1]) * 0.5,
                (scene_bounds_min[2] + scene_bounds_max[2]) * 0.5,
            ],
            "views": ["front +z", "side +x", "underside -y", "top -y silhouette"],
        },
        "corridor": {
            "note": "north-south plant-origin exclusion; canopy overhang possible; native traversal NOT RUN",
            "tile_columns_x": [CORRIDOR_TILE_X.0, CORRIDOR_TILE_X.1],
            "world_x_range_m": [
                CORRIDOR_TILE_X.0 as f32 * 0.25,
                CORRIDOR_TILE_X.1 as f32 * 0.25,
            ],
            "z_extent_m": [-8.0, 8.0],
        },
        "memory_bytes": {
            "source_payload": counts.source_bytes,
            "cached_mesh": counts.cached_mesh_bytes,
            "snapshot_json_total": snapshot_bytes_total,
            "exported_mesh_total": mesh_bytes_total,
            "excludes": "BTreeMap/allocator overhead, instance records, GPU buffers, any renderer or engine state",
        },
        "budgets": {
            "max_volume_chunks": MAX_VOLUME_CHUNKS,
            "max_volume_source_bytes": MAX_VOLUME_SOURCE_BYTES,
            "max_volume_cells": MAX_VOLUME_CELLS,
            "max_scene_source_bytes": MAX_SCENE_SOURCE_BYTES,
            "max_scene_cache_bytes": MAX_SCENE_CACHE_BYTES,
            "max_mesh_bytes": MAX_MESH_BYTES,
            "max_snapshot_runs": MAX_SNAPSHOT_RUNS,
            "max_snapshot_json_bytes": MAX_SNAPSHOT_JSON_BYTES,
            "max_cell_coord": MAX_CELL_COORD,
            "note": "conservative desktop prototype budgets, enforced before allocation; not a phone feasibility claim",
        },
        "prototypes": prototypes,
        "meshes": meshes,
        "instances": draws,
        "ground_contact": contact,
    });
    let text = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
    std::fs::write(out.join("manifest.json"), format!("{text}\n")).expect("write manifest");

    let timing = serde_json::json!({
        "note": "HOST ONLY desktop generation/meshing measurements; NOT phone performance",
        "scene_build_ms": build_ms,
        "meshes": mesh_timings,
    });
    std::fs::write(
        out.join("timing.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&timing).expect("serialize timing")
        ),
    )
    .expect("write timing");

    println!("wrote flora gallery to {}", out.display());
    Ok(())
}
