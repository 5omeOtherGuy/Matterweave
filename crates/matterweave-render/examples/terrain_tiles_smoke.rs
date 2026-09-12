//! Coarse terrain visual smoke: one authoritative hillside scene is meshed at
//! fine resolution, then as a mixed fine-left / coarse-right scene derived from
//! the read-only `World::coarse_tile` source, then re-derived after an edit.
//!
//! This is an example-only consumer. It never changes authoritative world data
//! for LOD, never publishes derived state through the core, and makes no
//! renderer, LOD, collision, Android or performance adoption claim. It exists
//! to prove that coarse tiles can be adapted to real world-space geometry
//! through the existing `World::mesh` path and drawn by the same `Renderer`
//! bootstrap used by the other render smokes.
//!
//! # Geometry contract
//!
//! `World::mesh` emits one quad per exposed *unit* voxel face. The coarse
//! adapter re-meshes a bounded 16^3 tile-local temporary world through the same
//! function, then bakes the tile origin and the `1 << level` fine-cell scale
//! into the resulting vertices. Each drawn quad therefore spans one whole
//! coarse cell (`2` or `4` fine cells per edge); a tile is never expanded into
//! `(1 << level)^3` fine cubes. The CPU tests below assert that contract
//! directly: triangle count equals two per exposed coarse face, every edge is
//! the coarse cell scale (or that quad's diagonal), and every vertex lies on
//! the coarse lattice anchored at the tile origin.
//!
//! The mixed scene keeps the source state separate: the fine side is a bounded
//! temporary copy of authoritative cells, the coarse side is a fresh tile-local
//! world per derived tile, and the authoritative world is only ever read by
//! both. The `SourceStamp` identity is application-owned; derivation provenance
//! (`seed` + conservative whole-world `source_revision`) is always checked
//! against it and an old tile is never reused for a newer source.
//!
//! # Scene and evidence
//!
//! A deterministic hillside spans fine `x[-32, 32)`, `z[0, 32)`, `y[0, 16)`
//! with a low western shelf and a higher eastern plateau. A removable 4x4x4
//! fine mineral block stands on a pedestal on the eastern plateau; it is
//! exactly one level-2 coarse cell, so removing it changes coarse aggregate
//! occupancy. Four frames are presented from one fixed camera: all-fine
//! reference, mixed level-2 fine-left/coarse-right, mixed after the edit, and
//! all-fine after the edit. Every frame uploads one aggregated scene mesh
//! through `Renderer::upload`; no arbitrary chunk-key namespace is invented.
//! Per-frame evidence prints the source revision, CPU triangle counts, coarse
//! tile facts and face scale. `Presented` alone is never treated as visual
//! correctness; see the matching engineering log for the recorded run.
//!
//! ```text
//! cargo test -p matterweave-render --example terrain_tiles_smoke
//! cargo run  -p matterweave-render --example terrain_tiles_smoke [-- --hold]
//! ```

use glam::{Mat4, Vec3};
use matterweave_core::{
    coarse::{tile_span, COARSE_TILE_EDGE},
    CoarseTile, Mesh, Vertex, World,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer};
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

/// Application-owned source identity for this scene. It is never inferred from
/// a world revision or from derived tile contents; two worlds with equal
/// revisions are still different sources.
const SCENE_ID: u64 = 0x5EED_0000_C0A5_7E11;
/// Authoritative source seed, used for the whole run.
const SCENE_SEED: u64 = 0xC0A5_7E11_5EED_0001;

/// Scene fine-cell bounds, half-open per axis. Every drawn vertex must lie in
/// the closed box `SCENE_MIN..=SCENE_MAX`.
const SCENE_MIN: [i32; 3] = [-32, 0, 0];
const SCENE_MAX: [i32; 3] = [32, 16, 32];

/// Coarse level used for the mixed scene and the removable feature.
const MIXED_LEVEL: u8 = 2;
/// Fine x plane where the fine side ends and the coarse side begins.
const LOD_SPLIT_X: i32 = 0;

/// Removable feature: exactly one level-2 coarse cell (4^3 fine cells).
const FEATURE_MIN: [i32; 3] = [16, 12, 16];
const FEATURE_EDGE: i32 = 4;
const FEATURE_MATERIAL: u8 = 7;
/// One fine stone layer directly under the feature footprint.
const PEDESTAL_Y: i32 = 11;
const FEATURE_CELLS: usize = (FEATURE_EDGE * FEATURE_EDGE * FEATURE_EDGE) as usize;

/// Fixed presentation frames; the default run terminates after these.
const TOTAL_FRAMES: u32 = 4;
const FRAME_LABELS: [&str; TOTAL_FRAMES as usize] = [
    "all-fine reference",
    "mixed fine-left/level-2-coarse-right",
    "mixed after coarse-occupancy edit",
    "all-fine after edit",
];

/// Half-open fine-cell box. `max` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bounds {
    min: [i32; 3],
    max: [i32; 3],
}

impl Bounds {
    const SCENE: Self = Self {
        min: SCENE_MIN,
        max: SCENE_MAX,
    };
    const FINE_LEFT: Self = Self {
        min: SCENE_MIN,
        max: [LOD_SPLIT_X, SCENE_MAX[1], SCENE_MAX[2]],
    };
    const COARSE_RIGHT: Self = Self {
        min: [LOD_SPLIT_X, SCENE_MIN[1], SCENE_MIN[2]],
        max: SCENE_MAX,
    };
    const FEATURE: Self = Self {
        min: FEATURE_MIN,
        max: [
            FEATURE_MIN[0] + FEATURE_EDGE,
            FEATURE_MIN[1] + FEATURE_EDGE,
            FEATURE_MIN[2] + FEATURE_EDGE,
        ],
    };
}

/// Application-owned identity plus revision of the authoritative source that a
/// derived mesh came from. Equal revisions only mean anything inside one scene;
/// a consumer re-derives when any field differs and never treats tile equality
/// as proof of validity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceStamp {
    scene: u64,
    seed: u64,
    revision: u64,
}

impl SourceStamp {
    fn of(world: &World) -> Self {
        Self {
            scene: SCENE_ID,
            seed: world.seed(),
            revision: world.revision(),
        }
    }

    /// Conservative provenance check against a derived tile: matching requires
    /// the same application scene, the same source seed and the exact
    /// whole-world revision the tile was derived at.
    fn matches(self, tile: &CoarseTile) -> bool {
        self.scene == SCENE_ID
            && tile.seed() == self.seed
            && tile.source_revision() == self.revision
    }
}

// ---------------------------------------------------------------------------
// Deterministic scene
// ---------------------------------------------------------------------------

/// Integer terrain height. The west (`x < 0`) is a low shelf of 2..=4 fine
/// cells; the east is a higher plateau of 8..=12 that terraces in eight-cell
/// steps and alternates by z band. Every constant here is fixed by the test
/// expectations below.
fn terrain_height(x: i32, z: i32) -> i32 {
    if x < 0 {
        2 + (x + 32).div_euclid(8) % 3
    } else {
        let ramp = (x / 8).min(3);
        8 + ramp + (z / 8) % 2
    }
}

/// Material layering from the surface down, shared by every mesh path because
/// both paths mesh through `World::mesh`. Material ids match the core palette.
fn terrain_material(y: i32, height: i32) -> u8 {
    if y == height {
        if height <= 4 {
            4 // sand shelf
        } else {
            1 // moss plateau
        }
    } else if y >= height - 2 {
        2 // soil
    } else {
        3 // weathered stone
    }
}

/// Builds the single authoritative world this example owns.
fn build_scene() -> World {
    let mut world = World::new(SCENE_SEED);
    for x in SCENE_MIN[0]..SCENE_MAX[0] {
        for z in SCENE_MIN[2]..SCENE_MAX[2] {
            let height = terrain_height(x, z);
            for y in SCENE_MIN[1]..=height {
                assert!(world.set([x, y, z], terrain_material(y, height)));
            }
        }
    }
    for x in Bounds::FEATURE.min[0]..Bounds::FEATURE.max[0] {
        for z in Bounds::FEATURE.min[2]..Bounds::FEATURE.max[2] {
            assert!(world.set([x, PEDESTAL_Y, z], 3));
        }
    }
    write_feature(&mut world, FEATURE_MATERIAL);
    world
}

/// Writes the whole feature box. Returns how many authoritative cells changed;
/// removing it must report exactly `FEATURE_CELLS`.
fn write_feature(world: &mut World, material: u8) -> usize {
    let mut changed = 0;
    for x in Bounds::FEATURE.min[0]..Bounds::FEATURE.max[0] {
        for y in Bounds::FEATURE.min[1]..Bounds::FEATURE.max[1] {
            for z in Bounds::FEATURE.min[2]..Bounds::FEATURE.max[2] {
                if world.set([x, y, z], material) {
                    changed += 1;
                }
            }
        }
    }
    changed
}

/// Removes the removable feature; this is the edit that must change aggregate
/// occupancy and force re-derivation.
fn remove_feature(world: &mut World) -> usize {
    write_feature(world, 0)
}

/// Deterministic content checksum over the scene bounds, used to prove coarse
/// derivation does not mutate authoritative data.
fn scene_checksum(world: &World) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for x in Bounds::SCENE.min[0]..Bounds::SCENE.max[0] {
        for y in Bounds::SCENE.min[1]..Bounds::SCENE.max[1] {
            for z in Bounds::SCENE.min[2]..Bounds::SCENE.max[2] {
                hash = (hash ^ u64::from(world.get([x, y, z]))).wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

// ---------------------------------------------------------------------------
// Example-only coarse adapter
// ---------------------------------------------------------------------------

const COARSE_FACE_NORMALS: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// Exposed-face count of a tile under the same rule the mesher uses: a face is
/// exposed when its neighbor is air or outside the 16^3 tile.
fn exposed_faces(tile: &CoarseTile) -> usize {
    let mut faces = 0;
    for z in 0..COARSE_TILE_EDGE {
        for y in 0..COARSE_TILE_EDGE {
            for x in 0..COARSE_TILE_EDGE {
                if tile.material([x, y, z]) == Some(0) {
                    continue;
                }
                for normal in COARSE_FACE_NORMALS {
                    let neighbor = [x + normal[0], y + normal[1], z + normal[2]];
                    let inside = neighbor
                        .iter()
                        .all(|&coordinate| (0..COARSE_TILE_EDGE).contains(&coordinate));
                    if inside && tile.material(neighbor) != Some(0) {
                        continue;
                    }
                    faces += 1;
                }
            }
        }
    }
    faces
}

/// Example-only adapter: puts each coarse material into a fresh tile-local
/// temporary world (local index `x + 16 * (y + 16 * z)`), meshes it through the
/// unchanged `World::mesh`, and bakes the tile fine-cell origin and the
/// `1 << level` coarse scale into every vertex. The authoritative world is not
/// passed in and cannot be mutated here.
fn coarse_tile_mesh(tile: &CoarseTile) -> Mesh {
    let mut local = World::new(tile.seed());
    let mut cells = 0;
    for z in 0..COARSE_TILE_EDGE {
        for y in 0..COARSE_TILE_EDGE {
            for x in 0..COARSE_TILE_EDGE {
                let material = tile
                    .material([x, y, z])
                    .expect("local tile cell is inside 16^3");
                if material != 0 {
                    assert!(
                        local.set([x, y, z], material),
                        "tile-local cell is writable"
                    );
                    cells += 1;
                }
            }
        }
    }
    assert_eq!(
        cells,
        tile.solid_cells(),
        "tile material count is consistent"
    );
    let mut mesh = local.mesh();
    let scale = (1u32 << tile.level()) as f32;
    let origin = tile.origin();
    for vertex in &mut mesh.vertices {
        for (axis, &origin) in origin.iter().enumerate() {
            vertex.position[axis] = origin as f32 + vertex.position[axis] * scale;
        }
    }
    mesh
}

/// Asserts the coarse mesh is built from whole coarse faces before upload:
/// one quad per exposed coarse face, two triangles per quad, every vertex on
/// the coarse lattice at the tile origin, and no edge shorter than one coarse
/// cell. Returns the measured cell scale.
fn assert_coarse_face_contract(tile: &CoarseTile, mesh: &Mesh) -> f32 {
    let scale = (1u32 << tile.level()) as f32;
    let origin = tile.origin().map(|value| value as f32);
    let faces = exposed_faces(tile);
    assert!(faces > 0, "face contract applies to a non-empty tile");
    assert_eq!(
        mesh.vertices.len(),
        faces * 4,
        "one quad per exposed coarse face"
    );
    assert_eq!(
        mesh.indices.len(),
        faces * 6,
        "two triangles per exposed coarse face"
    );
    for vertex in &mesh.vertices {
        for (axis, &origin) in origin.iter().enumerate() {
            let local = (vertex.position[axis] - origin) / scale;
            assert!(
                (0.0..=COARSE_TILE_EDGE as f32).contains(&local) && local.fract() == 0.0,
                "vertex {axis} must sit on the coarse lattice (got {local})"
            );
        }
    }
    let tolerance = scale * 1.0e-4;
    for triangle in mesh.indices.chunks_exact(3) {
        let positions: [[f32; 3]; 3] =
            std::array::from_fn(|i| mesh.vertices[triangle[i] as usize].position);
        let smallest = triangle_edges(&positions)
            .into_iter()
            .fold(f32::INFINITY, f32::min);
        assert!(
            smallest >= scale - tolerance,
            "a coarse face edge ({smallest}) is finer than one coarse cell ({scale}); \
             the tile was expanded into fine cubes"
        );
        for edge in triangle_edges(&positions) {
            assert!(
                (edge - scale).abs() <= tolerance
                    || (edge - scale * std::f32::consts::SQRT_2).abs() <= tolerance,
                "coarse face edge {edge} is not a {scale} cell edge or its quad diagonal"
            );
        }
    }
    scale
}

/// Edges of one triangle, in winding order.
fn triangle_edges(positions: &[[f32; 3]; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        let a = positions[i];
        let b = positions[(i + 1) % 3];
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    })
}

/// Counts triangles that lie completely on the fine plane and face `+x`.
fn on_plane_positive_faces(mesh: &Mesh, plane_x: i32) -> usize {
    mesh.indices
        .chunks_exact(3)
        .filter(|triangle| {
            triangle.iter().all(|&index| {
                let vertex = mesh.vertices[index as usize];
                vertex.position[0] == plane_x as f32 && vertex.normal[0] > 0.5
            })
        })
        .count()
}

// ---------------------------------------------------------------------------
// Bounded fine and derived coarse scene parts
// ---------------------------------------------------------------------------

/// Copies authoritative cells inside `bounds` into a bounded temporary world.
/// Cells keep world coordinates, so `World::mesh` emits world-space vertices.
fn copy_bounded(source: &World, bounds: Bounds) -> World {
    let mut copy = World::new(source.seed());
    for x in bounds.min[0]..bounds.max[0] {
        for y in bounds.min[1]..bounds.max[1] {
            for z in bounds.min[2]..bounds.max[2] {
                let material = source.get([x, y, z]);
                if material != 0 {
                    assert!(
                        copy.set([x, y, z], material),
                        "bounded copy cell is writable"
                    );
                }
            }
        }
    }
    copy
}

/// Removes triangles that exist only because the bounded fine copy lacks the
/// authoritative neighbor across `plane_x`. A face is removed when all three
/// vertices lie on the plane, its normal points at `+x`, and the authoritative
/// world is solid at the fine cell it would open into: the coarse side owns
/// that plane, and keeping the duplicate would be coplanar double geometry.
fn cull_occluded_plane_faces(source: &World, mesh: Mesh, plane_x: i32) -> Mesh {
    let mut kept = Mesh {
        revision: mesh.revision,
        ..Mesh::default()
    };
    let mut remap = vec![u32::MAX; mesh.vertices.len()];
    for triangle in mesh.indices.chunks_exact(3) {
        let indices: [u32; 3] = std::array::from_fn(|i| triangle[i]);
        let vertices: [Vertex; 3] = std::array::from_fn(|i| mesh.vertices[indices[i] as usize]);
        let solid_across = {
            let cell_y = vertices
                .iter()
                .map(|vertex| vertex.position[1])
                .fold(f32::INFINITY, f32::min) as i32;
            let cell_z = vertices
                .iter()
                .map(|vertex| vertex.position[2])
                .fold(f32::INFINITY, f32::min) as i32;
            source.get([plane_x, cell_y, cell_z]) != 0
        };
        let drop = solid_across
            && vertices
                .iter()
                .all(|vertex| vertex.position[0] == plane_x as f32)
            && vertices[0].normal[0] > 0.5;
        if drop {
            continue;
        }
        for &index in &indices {
            let slot = &mut remap[index as usize];
            if *slot == u32::MAX {
                *slot = kept.vertices.len() as u32;
                kept.vertices.push(mesh.vertices[index as usize]);
            }
            kept.indices.push(*slot);
        }
    }
    kept
}

/// Bounded fine side of the mixed scene: fine geometry from a temporary copy
/// of authoritative cells, with the LOD boundary plane de-duplicated.
fn fine_side_mesh(source: &World, bounds: Bounds, lod_plane_x: i32) -> Mesh {
    let copy = copy_bounded(source, bounds);
    let mesh = cull_occluded_plane_faces(source, copy.mesh(), lod_plane_x);
    assert_eq!(
        on_plane_positive_faces(&mesh, lod_plane_x),
        0,
        "the coarse side owns the LOD plane; no duplicate fine wall may remain"
    );
    mesh
}

/// Derives every non-empty coarse tile intersecting `bounds` and adapts each
/// one. Empty tiles are valid and simply contribute no geometry. Every tile is
/// checked against the application source stamp before its mesh is accepted.
fn coarse_side_meshes(
    source: &World,
    stamp: SourceStamp,
    level: u8,
    bounds: Bounds,
) -> Result<Vec<(CoarseTile, Mesh)>, String> {
    let span = tile_span(level).ok_or_else(|| format!("unsupported coarse level {level}"))?;
    let first = bounds.min.map(|coordinate| coordinate.div_euclid(span));
    let last = bounds
        .max
        .map(|coordinate| (coordinate - 1).div_euclid(span));
    let mut derived = Vec::new();
    for kz in first[2]..=last[2] {
        for ky in first[1]..=last[1] {
            for kx in first[0]..=last[0] {
                let key = [kx, ky, kz];
                let tile = source
                    .coarse_tile(level, key)
                    .map_err(|error| format!("coarse tile level {level} key {key:?}: {error}"))?;
                assert_eq!(tile.level(), level);
                assert_eq!(tile.span(), span);
                assert_eq!(tile.origin(), key.map(|coordinate| coordinate * span));
                assert!(
                    stamp.matches(&tile),
                    "derived tile must match the application source stamp"
                );
                if tile.is_empty() {
                    continue;
                }
                let mesh = coarse_tile_mesh(&tile);
                derived.push((tile, mesh));
            }
        }
    }
    Ok(derived)
}

/// Concatenates world-space parts into one scene mesh for a single
/// `Renderer::upload`. The upload revision is an ordering guard only: source
/// provenance lives in [`SourceStamp`] and is never inferred from mesh bytes.
fn aggregate(parts: &[&Mesh], upload_revision: u64) -> Mesh {
    let mut combined = Mesh {
        revision: upload_revision,
        ..Mesh::default()
    };
    for part in parts {
        let base =
            u32::try_from(combined.vertices.len()).expect("scene mesh exceeds u32 index range");
        combined.vertices.extend_from_slice(&part.vertices);
        combined
            .indices
            .extend(part.indices.iter().map(|index| index + base));
    }
    combined
}

/// CPU facts about one prepared mixed scene, printed as frame evidence.
struct MixedScene {
    mesh: Mesh,
    level: u8,
    fine_triangles: usize,
    coarse_triangles: usize,
    coarse_solid_cells: usize,
    coarse_tiles: usize,
    face_size: f32,
}

/// Builds the mixed scene: bounded fine geometry on the left, derived coarse
/// tiles on the right. `stamp` is the application source identity and must
/// describe `source` exactly.
fn mixed_scene(
    source: &World,
    stamp: SourceStamp,
    level: u8,
    upload_revision: u64,
) -> Result<MixedScene, String> {
    assert_eq!(stamp.scene, SCENE_ID, "application-owned scene identity");
    assert_eq!(stamp.seed, source.seed(), "source seed must be unchanged");
    assert_eq!(
        stamp.revision,
        source.revision(),
        "source revision must be current"
    );
    let checksum = scene_checksum(source);
    let fine = fine_side_mesh(source, Bounds::FINE_LEFT, LOD_SPLIT_X);
    let coarse = coarse_side_meshes(source, stamp, level, Bounds::COARSE_RIGHT)?;
    let mut face_size: f32 = 0.0;
    let mut coarse_solid_cells = 0;
    for (tile, mesh) in &coarse {
        face_size = face_size.max(assert_coarse_face_contract(tile, mesh));
        coarse_solid_cells += tile.solid_cells();
    }
    let fine_triangles = fine.indices.len() / 3;
    let mut parts: Vec<&Mesh> = vec![&fine];
    for (_, mesh) in &coarse {
        parts.push(mesh);
    }
    let mesh = aggregate(&parts, upload_revision);
    assert_mesh_within_scene_bounds(&mesh);
    assert_eq!(
        scene_checksum(source),
        checksum,
        "coarse derivation must not mutate the authoritative source"
    );
    let coarse_triangles = mesh.indices.len() / 3 - fine_triangles;
    Ok(MixedScene {
        mesh,
        level,
        fine_triangles,
        coarse_triangles,
        coarse_solid_cells,
        coarse_tiles: coarse.len(),
        face_size,
    })
}

/// Full fine reference mesh of the authoritative world.
fn fine_reference_mesh(world: &World, upload_revision: u64) -> Mesh {
    let mut mesh = world.mesh();
    assert_mesh_within_scene_bounds(&mesh);
    mesh.revision = upload_revision;
    mesh
}

/// Every drawn vertex must lie in the explicit closed scene bounds.
fn assert_mesh_within_scene_bounds(mesh: &Mesh) {
    for vertex in &mesh.vertices {
        for axis in 0..3 {
            assert!(
                vertex.position[axis] >= SCENE_MIN[axis] as f32 - 1.0e-4
                    && vertex.position[axis] <= SCENE_MAX[axis] as f32 + 1.0e-4,
                "vertex {axis} outside scene bounds: {:?}",
                vertex.position
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Fixed camera
// ---------------------------------------------------------------------------

/// Explicit fixed camera for all four frames: same eye, target, up, vertical
/// field of view and depth range; only the aspect follows the window.
struct Camera {
    eye: [f32; 3],
    target: [f32; 3],
    fov_y: f32,
    near: f32,
    far: f32,
}

impl Camera {
    const SCENE: Self = Self {
        eye: [-34.0, 30.0, 86.0],
        target: [0.0, 6.0, 14.0],
        fov_y: 0.8,
        near: 0.1,
        far: 500.0,
    };

    fn view_projection(&self, aspect: f32) -> Mat4 {
        let eye = Vec3::from_array(self.eye);
        let view = Mat4::look_at_rh(eye, Vec3::from_array(self.target), Vec3::Y);
        Mat4::perspective_rh(self.fov_y, aspect, self.near, self.far) * view
    }
}

// ---------------------------------------------------------------------------
// winit application
// ---------------------------------------------------------------------------

/// One prepared frame's CPU facts, recorded before upload and printed when the
/// frame is actually presented.
#[derive(Clone, Copy, Debug)]
struct FrameEvidence {
    label: &'static str,
    world_revision: u64,
    upload_revision: u64,
    level: u8,
    triangles: usize,
    fine_triangles: usize,
    coarse_triangles: usize,
    coarse_tiles: usize,
    coarse_solid_cells: usize,
    face_size: f32,
}

impl FrameEvidence {
    fn fine(label: &'static str, stamp: SourceStamp, upload_revision: u64, mesh: &Mesh) -> Self {
        Self {
            label,
            world_revision: stamp.revision,
            upload_revision,
            level: 0,
            triangles: mesh.indices.len() / 3,
            fine_triangles: mesh.indices.len() / 3,
            coarse_triangles: 0,
            coarse_tiles: 0,
            coarse_solid_cells: 0,
            face_size: 1.0,
        }
    }

    fn mixed(
        label: &'static str,
        stamp: SourceStamp,
        upload_revision: u64,
        scene: &MixedScene,
    ) -> Self {
        Self {
            label,
            world_revision: stamp.revision,
            upload_revision,
            level: scene.level,
            triangles: scene.mesh.indices.len() / 3,
            fine_triangles: scene.fine_triangles,
            coarse_triangles: scene.coarse_triangles,
            coarse_tiles: scene.coarse_tiles,
            coarse_solid_cells: scene.coarse_solid_cells,
            face_size: scene.face_size,
        }
    }

    fn print(&self, frame: u32, renderer: &Renderer) {
        println!(
            "terrain_tiles_smoke[frame {frame}] {}: source_revision={} upload_revision={} \
             coarse_level={} triangles={} (fine={} coarse={}) coarse_tiles={} \
             coarse_solid_cells={} coarse_face_size={} gpu_mesh_bytes={} shadow_casters={} \
             shadow_map_updated={}",
            self.label,
            self.world_revision,
            self.upload_revision,
            self.level,
            self.triangles,
            self.fine_triangles,
            self.coarse_triangles,
            self.coarse_tiles,
            self.coarse_solid_cells,
            self.face_size,
            renderer.mesh_bytes,
            renderer.shadow_caster_meshes(),
            renderer.shadow_map_updated(),
        );
    }
}

struct App {
    hold: bool,
    world: World,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    upload_revision: u64,
    frame: u32,
    prepared: Option<u32>,
    evidence: Vec<FrameEvidence>,
    hold_announced: bool,
}

impl App {
    fn new(hold: bool) -> Self {
        Self {
            hold,
            world: build_scene(),
            window: None,
            renderer: None,
            upload_revision: 0,
            frame: 0,
            prepared: None,
            evidence: Vec::new(),
            hold_announced: false,
        }
    }

    /// Prepares the geometry for one frame on the CPU. Frame 2 performs the
    /// authoritative edit and asserts that the re-derived coarse occupancy and
    /// both geometry paths changed by the amounts the contract predicts.
    fn prepare_scene(&mut self, frame: u32) -> (Mesh, FrameEvidence) {
        self.upload_revision += 1;
        let upload_revision = self.upload_revision;
        match frame {
            0 | 3 => {
                let stamp = SourceStamp::of(&self.world);
                let mesh = fine_reference_mesh(&self.world, upload_revision);
                let evidence = FrameEvidence::fine(
                    FRAME_LABELS[frame as usize],
                    stamp,
                    upload_revision,
                    &mesh,
                );
                if frame == 3 {
                    let before = self.evidence[0];
                    assert_eq!(before.world_revision + FEATURE_CELLS as u64, stamp.revision);
                    assert_eq!(
                        before.triangles,
                        evidence.triangles + 128,
                        "removing the 4^3 block must drop 64 exposed fine faces"
                    );
                }
                (mesh, evidence)
            }
            1 => self.prepare_mixed(frame, upload_revision),
            2 => {
                let removed = remove_feature(&mut self.world);
                assert_eq!(
                    removed, FEATURE_CELLS,
                    "the removable feature is exactly one level-2 coarse cell"
                );
                let before = self.evidence[1];
                let (mesh, evidence) = self.prepare_mixed(frame, upload_revision);
                assert_eq!(
                    before.world_revision + FEATURE_CELLS as u64,
                    evidence.world_revision
                );
                assert_eq!(
                    before.coarse_solid_cells,
                    evidence.coarse_solid_cells + 1,
                    "the edited aggregate must become empty"
                );
                assert_eq!(
                    before.coarse_triangles,
                    evidence.coarse_triangles + 8,
                    "five exposed faces lost and one pedestal top gained = 8 triangles"
                );
                (mesh, evidence)
            }
            other => panic!("unexpected frame {other}"),
        }
    }

    fn prepare_mixed(&self, frame: u32, upload_revision: u64) -> (Mesh, FrameEvidence) {
        let stamp = SourceStamp::of(&self.world);
        let mixed = mixed_scene(&self.world, stamp, MIXED_LEVEL, upload_revision)
            .unwrap_or_else(|error| panic!("mixed scene: {error}"));
        let evidence =
            FrameEvidence::mixed(FRAME_LABELS[frame as usize], stamp, upload_revision, &mixed);
        (mixed.mesh, evidence)
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_none() || self.window.is_none() {
            return;
        }
        if self.frame < TOTAL_FRAMES && self.prepared != Some(self.frame) {
            let (mesh, evidence) = self.prepare_scene(self.frame);
            let renderer = self.renderer.as_mut().unwrap();
            renderer
                .upload(&mesh)
                .unwrap_or_else(|error| panic!("scene upload failed: {error}"));
            assert_eq!(
                renderer.mesh_revision,
                Some(evidence.upload_revision),
                "the aggregated scene upload must replace the previous one"
            );
            self.evidence.push(evidence);
            self.prepared = Some(self.frame);
        }
        let window = self.window.as_ref().unwrap();
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let camera = Camera::SCENE;
        let view_projection = camera.view_projection(width as f32 / height as f32);
        let lighting = LightingSettings {
            shadows: true,
            ..Default::default()
        };
        let renderer = self.renderer.as_mut().unwrap();
        match renderer.render_with_lighting(
            view_projection.to_cols_array_2d(),
            camera.eye,
            &Hud::new(width as f32, height as f32),
            &lighting,
        ) {
            FrameResult::Presented => {
                if let Some(evidence) = self.evidence.get(self.frame as usize).copied() {
                    if self.frame == 0 {
                        assert!(
                            renderer.shadow_map_updated(),
                            "the first frame must build the shadow map"
                        );
                        assert!(
                            renderer.shadow_caster_meshes() >= 1,
                            "the uploaded scene mesh must be a shadow caster"
                        );
                    }
                    evidence.print(self.frame, renderer);
                }
                self.frame += 1;
                if self.frame == TOTAL_FRAMES {
                    if self.hold {
                        if !self.hold_announced {
                            self.hold_announced = true;
                            println!(
                                "terrain_tiles_smoke: --hold is active; close the window or \
                                 press Escape to exit"
                            );
                        }
                    } else {
                        println!(
                            "terrain_tiles_smoke: {TOTAL_FRAMES} frames presented (all-fine, \
                             mixed, edited mixed, edited all-fine); scene identity {SCENE_ID:#018x}; \
                             Android device, image correctness and performance claims NOT RUN"
                        );
                        self.renderer = None;
                        self.window = None;
                        event_loop.exit();
                    }
                }
            }
            FrameResult::Retry => {}
            other => panic!("unexpected render result: {other:?}"),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Matterweave coarse terrain validation")
                        .with_inner_size(winit::dpi::PhysicalSize::new(960, 640)),
                )
                .unwrap(),
        );
        let mut initialization = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(result) = initialization
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        else {
            panic!("Renderer initialization unexpectedly suspended");
        };
        let renderer = result.unwrap();
        println!("{}", renderer.capabilities);
        println!(
            "terrain_tiles_smoke: identity {SCENE_ID:#018x} seed {SCENE_SEED:#018x} \
             bounds {SCENE_MIN:?}..{SCENE_MAX:?} lod_split_x={LOD_SPLIT_X} \
             level={MIXED_LEVEL} feature={FEATURE_MIN:?}+{FEATURE_EDGE} camera_eye={:?}",
            Camera::SCENE.eye
        );
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                return;
            }
            WindowEvent::RedrawRequested => {}
            _ => return,
        }
        self.redraw(event_loop);
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn main() {
    let mut hold = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--hold" => hold = true,
            other => {
                eprintln!("terrain_tiles_smoke: unknown argument {other:?}; supported: --hold");
                std::process::exit(2);
            }
        }
    }
    EventLoop::new()
        .unwrap()
        .run_app(&mut App::new(hold))
        .unwrap();
}

// ---------------------------------------------------------------------------
// CPU geometry and edit contract
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Coarse faces span whole coarse cells at both supported levels, and the
    /// removable feature is exactly one coarse cell at either level.
    #[test]
    fn coarse_adapter_keeps_coarse_face_scale() {
        let world = build_scene();
        for level in [1u8, 2u8] {
            let span = tile_span(level).unwrap();
            let tile = world.coarse_tile(level, [0, 0, 0]).unwrap();
            assert_eq!(tile.level(), level);
            assert_eq!(tile.span(), span);
            assert_eq!(tile.origin(), [0, 0, 0]);
            assert_eq!(tile.seed(), SCENE_SEED);
            assert_eq!(tile.source_revision(), world.revision());
            assert!(tile.solid_cells() > 0);

            let mesh = coarse_tile_mesh(&tile);
            let scale = assert_coarse_face_contract(&tile, &mesh);
            assert_eq!(scale, (1 << level) as f32);

            let local = FEATURE_MIN.map(|coordinate| coordinate / (1 << level));
            assert_eq!(tile.material(local), Some(FEATURE_MATERIAL));
            assert_eq!(tile.world_cell(local), Some(FEATURE_MIN));
        }

        // At level 2 the 4^3 feature is one coarse box: its top is a single
        // coarse quad at world y = 16 covered by exactly two triangles, and
        // each edge spans four fine cells, not 4^2 cube tops.
        let tile = world.coarse_tile(2, [0, 0, 0]).unwrap();
        let mesh = coarse_tile_mesh(&tile);
        let feature_top_triangles = mesh
            .indices
            .chunks_exact(3)
            .filter(|triangle| {
                triangle.iter().all(|&index| {
                    let position = mesh.vertices[index as usize].position;
                    position[1] == 16.0
                        && (16.0..=20.0).contains(&position[0])
                        && (16.0..=20.0).contains(&position[2])
                })
            })
            .count();
        assert_eq!(
            feature_top_triangles, 2,
            "one coarse-sized quad covers the whole feature top"
        );
    }

    /// The mixed scene is fine on the left and coarse on the right, both from
    /// the same authoritative source, and the LOD plane has no duplicate fine
    /// wall fighting with the coarse wall.
    #[test]
    fn mixed_scene_is_fine_left_and_coarse_right() {
        let world = build_scene();
        let stamp = SourceStamp::of(&world);
        let mixed = mixed_scene(&world, stamp, MIXED_LEVEL, 7).unwrap();
        assert_eq!(mixed.mesh.revision, 7);
        assert_eq!(mixed.level, MIXED_LEVEL);
        assert_eq!(mixed.face_size, (1 << MIXED_LEVEL) as f32);
        assert_eq!(
            mixed.coarse_tiles, 1,
            "one level-2 tile covers the right half"
        );
        assert!(mixed.fine_triangles > 0);
        assert!(mixed.coarse_triangles > 0);
        assert!(mixed.coarse_solid_cells > 0);
        assert_eq!(
            mixed.mesh.indices.len() / 3,
            mixed.fine_triangles + mixed.coarse_triangles
        );

        // Region contract by triangle: triangles left of the split carry unit
        // edges, triangles starting at the split carry coarse-cell edges.
        let scale = (1 << MIXED_LEVEL) as f32;
        for triangle in mixed.mesh.indices.chunks_exact(3) {
            let positions: [[f32; 3]; 3] =
                std::array::from_fn(|i| mixed.mesh.vertices[triangle[i] as usize].position);
            let min_x = positions
                .iter()
                .map(|position| position[0])
                .fold(f32::INFINITY, f32::min);
            let expected = if min_x < LOD_SPLIT_X as f32 {
                1.0
            } else {
                scale
            };
            let smallest = triangle_edges(&positions)
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            assert!(
                (smallest - expected).abs() <= expected * 1.0e-4,
                "triangle edge {smallest} does not match its region scale {expected}"
            );
        }
        assert_mesh_within_scene_bounds(&mixed.mesh);

        // The bounded copy really does expose the seam faces; the mixed fine
        // side removes every one the coarse side owns.
        let raw = copy_bounded(&world, Bounds::FINE_LEFT).mesh();
        assert!(
            on_plane_positive_faces(&raw, LOD_SPLIT_X) > 0,
            "the test must exercise the duplicated boundary wall"
        );
        assert_eq!(on_plane_positive_faces(&mixed.mesh, LOD_SPLIT_X), 0);
        let culled = cull_occluded_plane_faces(&world, raw, LOD_SPLIT_X);
        assert_eq!(mixed.fine_triangles, culled.indices.len() / 3);
    }

    /// Coarse derivation reads the authoritative world without changing it and
    /// is deterministic across repeated requests.
    #[test]
    fn derivation_is_read_only_and_repeatable() {
        let world = build_scene();
        let revision = world.revision();
        let stats = world.stats();
        let checksum = scene_checksum(&world);
        for level in [1u8, 2u8] {
            let first = world.coarse_tile(level, [0, 0, 0]).unwrap();
            let _mesh = coarse_tile_mesh(&first);
            let second = world.coarse_tile(level, [0, 0, 0]).unwrap();
            assert_eq!(first, second);
        }
        assert_eq!(world.revision(), revision);
        assert_eq!(world.stats(), stats);
        assert_eq!(scene_checksum(&world), checksum);
    }

    /// The edit changes coarse aggregate occupancy, invalidates the old source
    /// stamp, and changes both the coarse and the fine geometry by exact,
    /// predicted amounts.
    #[test]
    fn edit_changes_aggregate_occupancy_and_rederives_both_paths() {
        let mut world = build_scene();
        let before = SourceStamp::of(&world);
        let before_tile = world.coarse_tile(MIXED_LEVEL, [0, 0, 0]).unwrap();
        let before_coarse = coarse_tile_mesh(&before_tile);
        let before_fine = world.mesh();
        let before_mixed = mixed_scene(&world, before, MIXED_LEVEL, 1).unwrap();
        assert!(before.matches(&before_tile));
        assert_eq!(before_tile.material([4, 3, 4]), Some(FEATURE_MATERIAL));

        let removed = remove_feature(&mut world);
        assert_eq!(removed, FEATURE_CELLS);
        assert_eq!(world.revision(), before.revision + FEATURE_CELLS as u64);

        let after = SourceStamp::of(&world);
        assert_ne!(after, before);
        assert!(
            !after.matches(&before_tile),
            "an old tile must never match a newer source"
        );

        let after_tile = world.coarse_tile(MIXED_LEVEL, [0, 0, 0]).unwrap();
        assert!(after.matches(&after_tile));
        assert_eq!(after_tile.solid_cells(), before_tile.solid_cells() - 1);
        assert_eq!(after_tile.material([4, 3, 4]), Some(0));

        let after_coarse = coarse_tile_mesh(&after_tile);
        let after_fine = world.mesh();
        let after_mixed = mixed_scene(&world, after, MIXED_LEVEL, 2).unwrap();

        // The floating block loses five exposed coarse faces and the pedestal
        // below gains its top face: four fewer faces, eight fewer triangles.
        assert_eq!(exposed_faces(&before_tile) - exposed_faces(&after_tile), 4);
        assert_eq!(
            before_coarse.indices.len() - after_coarse.indices.len(),
            8 * 3
        );
        // Fine geometry loses the block's 80 exposed faces and gains the 16
        // pedestal tops: 64 fewer faces, 128 fewer triangles.
        assert_eq!(
            before_fine.indices.len() - after_fine.indices.len(),
            128 * 3
        );
        // The mixed fine side never contained the feature (it is on the coarse
        // half), so only the coarse side of the mixed scene changes.
        assert_eq!(before_mixed.fine_triangles, after_mixed.fine_triangles);
        assert_eq!(
            before_mixed.coarse_triangles - after_mixed.coarse_triangles,
            8
        );
        assert_eq!(
            before_mixed.coarse_solid_cells,
            after_mixed.coarse_solid_cells + 1
        );
    }

    /// A valid all-air request is `Ok` with an empty tile and no geometry, and
    /// the coarse walk skips it instead of emitting anything.
    #[test]
    fn valid_empty_tiles_produce_no_geometry() {
        let world = build_scene();
        // Level-1 key [0, 1, 0] covers fine y 32..64: representable and air.
        let tile = world.coarse_tile(1, [0, 1, 0]).unwrap();
        assert!(tile.is_empty());
        assert_eq!(tile.solid_cells(), 0);
        assert_eq!(tile.source_revision(), world.revision());
        assert!(tile.materials().iter().all(|&material| material == 0));
        let mesh = coarse_tile_mesh(&tile);
        assert!(mesh.vertices.is_empty());
        assert!(mesh.indices.is_empty());

        let bounds = Bounds {
            min: [0, 0, 0],
            max: [32, 64, 32],
        };
        let parts = coarse_side_meshes(&world, SourceStamp::of(&world), 1, bounds).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].0.key(), [0, 0, 0]);
    }

    /// Unsupported levels surface as a walk error rather than silently
    /// degrading to fine geometry.
    #[test]
    fn unsupported_level_is_reported_by_the_walk() {
        let world = build_scene();
        let error = mixed_scene(&world, SourceStamp::of(&world), 3, 1)
            .map(|_| ())
            .unwrap_err();
        assert!(error.contains("level 3"), "{error}");
    }
}
