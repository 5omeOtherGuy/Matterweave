//! Writes the deterministic wetland support-flora gallery: content manifest,
//! raw mesh buffers (little-endian f32 vertices / u32 indices) and prototype
//! snapshots for source, half and quarter LODs, plus a host-only timing file.
//!
//! Usage: `cargo run -p matterweave-detail --example wetland_flora_gallery -- <out_dir>`
//! No image, window or renderer dependency is used. Layout mirrors
//! `flora_gallery.rs` so the lead host viewer can consume both manifests.
//!
//! `manifest.json` contains only reproducible content; host measurements go to
//! `timing.json` (desktop HOST ONLY, never phone performance). Visual
//! acceptance is NOT performed here.

use matterweave_detail::*;
use std::collections::BTreeMap;
use std::time::Instant;

/// Little-endian by construction, matching `flora_gallery.rs`.
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
        "/mnt/bench/matterweave-dev/performance/completion-01/flora-gallery-wetland".to_string()
    });
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("create output directory");

    let build_started = Instant::now();
    let mut scene = DetailScene::new();
    for id in WETLAND_FLORA_SPECIES {
        scene.add_prototype(wetland_prototype(id)?)?;
    }
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

    let counts = scene.counts();
    let manifest = serde_json::json!({
        "generator": "wetland_flora",
        "snapshot_version": SNAPSHOT_VERSION,
        "content_only": "deterministic; host timings are in timing.json",
        "scene_note": "three wetland support-flora prototypes (twisted woody shrub, tall horsetail spire, broad water lily); NOT full showcase acceptance and NOT a phone claim",
        "origin_convention": "local cell c spans [c*scale, (c+1)*scale] metres; transform is quarter-turn yaw then an arbitrary fractional translation in world metres",
        "species": WETLAND_FLORA_SPECIES,
        "reserved_species": ["bracket_shelf"],
        "material_policy": "woody trunk/branches reuse collidable FLORA_FUNNEL_STIPE (ID 30); all foliage/needles/pads/petals reuse decorative leaf IDs 38-47; no palette changes; BANK_STONE not used for wood",
        "water_placement": "marsh_lily is fully decorative and intended for shallow-water placement (floating pads at local y = 2 over a rhizome bed foot); dry-moss instance_support does not apply to it",
        "counts": {
            "prototypes": counts.prototypes,
            "unique_stored_cells": counts.unique_stored_cells,
            "instances": counts.instances,
            "derived_mesh_builds": counts.mesh_builds,
        },
        "camera": {
            "note": "suggested inspection framing only; no renderer is involved here",
            "views": ["front +z", "side +x", "underside -y", "top -y silhouette"],
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

    println!("wrote wetland flora gallery to {}", out.display());
    Ok(())
}
