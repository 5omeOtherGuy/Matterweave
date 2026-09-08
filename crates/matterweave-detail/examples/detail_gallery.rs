//! Writes the deterministic detail gallery: content manifest, raw mesh buffers
//! and prototype snapshots, plus a separate host timing file.
//!
//! Usage: `cargo run -p matterweave-detail --example detail_gallery -- <out_dir>`
//! No image, window or renderer dependency is used.
//!
//! The scene itself lives in [`matterweave_detail::gallery_scene`] so a native
//! adapter or test can build exactly the same scene without this example.
//!
//! `manifest.json` contains only reproducible content: equal inputs give equal
//! bytes. Host measurements go to `timing.json` and are desktop HOST ONLY
//! numbers, never phone performance.

use matterweave_detail::*;
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
        "/mnt/bench/matterweave-dev/performance/desktop-01/detail-gallery-a2".to_string()
    });
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("create output directory");

    let seed = 2026;
    let build_started = Instant::now();
    let mut scene = gallery_scene(seed)?;
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
    for id in scene.prototype_ids() {
        let volume = scene.prototype(&id).unwrap();
        let snapshot = volume.snapshot();
        let json = serde_json::to_vec(&snapshot).expect("serialize snapshot");
        snapshot_bytes_total += json.len();
        std::fs::write(out.join(format!("{id}.snapshot.json")), &json).expect("write snapshot");
        let bounds = volume.bounds_local().expect("non-empty prototype");
        let mut by_material: std::collections::BTreeMap<&str, usize> = Default::default();
        for (_, material) in volume.iter_cells() {
            *by_material.entry(material_name(material)).or_default() += 1;
        }
        prototypes.push(serde_json::json!({
            "id": id,
            "cell_size_m": volume.scale().metres(),
            "occupied_cells": volume.occupied_cells(),
            "chunks": volume.chunk_count(),
            "source_payload_bytes": volume.source_bytes(),
            "snapshot_json_bytes": json.len(),
            "snapshot_runs": snapshot.runs.len(),
            "revision": volume.revision(),
            "bounds_local_m": { "min": bounds.min, "max": bounds.max },
            "cells_by_material": by_material,
        }));
    }

    let draws: Vec<serde_json::Value> = scene
        .draws()
        .into_iter()
        .map(|draw| {
            serde_json::json!({
                "instance": draw.instance,
                "prototype": draw.prototype,
                "translation_m": draw.transform.translation_m,
                "yaw": format!("{:?}", draw.transform.yaw),
                "occupied_cells": draw.occupied_cells,
            })
        })
        .collect();

    // Ground contact evidence, derived from authoritative source queries.
    let tile = scene.prototype("terrain_tile_16m").expect("tile prototype");
    let cell = tile.scale().metres();
    let contact: Vec<serde_json::Value> = scene
        .draws()
        .into_iter()
        .filter(|draw| draw.prototype == "parasol_mushroom")
        .map(|draw| {
            let [tx, ty, tz] = draw.transform.translation_m;
            let column = ((tx / cell).floor() as i32, (tz / cell).floor() as i32);
            let (top, material) = column_top(tile, column.0, column.1).expect("supported column");
            serde_json::json!({
                "instance": draw.instance,
                "support_column": [column.0, column.1],
                "support_top_cell": top,
                "support_material": material_name(material),
                "gap_m": ty - (top + 1) as f32 * cell,
            })
        })
        .collect();

    let counts = scene.counts();
    let manifest = serde_json::json!({
        "generator_version": FIXTURE_GENERATOR_VERSION,
        "snapshot_version": SNAPSHOT_VERSION,
        "seed": seed,
        "content_only": "deterministic; host timings are in timing.json",
        "scene_note": "sparse demonstration gallery: one tile plus a few reused mushroom placements, not a density claim",
        "origin_convention": "local cell c spans [c*scale, (c+1)*scale] metres; transform is quarter-turn yaw then an arbitrary fractional translation in world metres",
        "counts": {
            "prototypes": counts.prototypes,
            "instances": counts.instances,
            "unique_stored_cells": counts.unique_stored_cells,
            "expanded_occupied_cells": counts.expanded_occupied_cells,
            "expanded_collision_cells": counts.expanded_collision_cells,
            "expanded_liquid_cells": counts.expanded_liquid_cells,
            "derived_mesh_builds": counts.mesh_builds,
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

    println!("{text}");
    println!("wrote gallery to {}", out.display());
    Ok(())
}
