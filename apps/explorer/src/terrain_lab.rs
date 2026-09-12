//! Terrain Lab: a bounded native scene that validates voxel LOD and edits.
//!
//! One authoritative fine [`World`] holds every cell of a deterministic hillside
//! inside `x, z` in `-32..32` and `y` in `0..16`. The lab presents that source in
//! two ways at the same camera:
//!
//! - **Fine**: the whole source world meshed exactly, every cell present.
//! - **Mixed**: the near half (`z >= 0`) as fine chunks and the far half
//!   (`z < 0`) as level-1 [`CoarseTile`]s. A tile is written into a tile-local
//!   temporary world, meshed with the shared palette, then scaled by
//!   `1 << level` with the tile origin baked into the vertices. No fine cell is
//!   expanded and no second authoritative world exists.
//!
//! Mode changes never modify the source world. DIG removes and BUILD fills one
//! aligned `2 x 2 x 2` fine region derived from the ray hit, which is exactly
//! one level-1 coarse cell, so the far half visibly follows the edit. SAVE and
//! RELOAD use only `terrain-lab.json` in the supplied directory.
//!
//! This is a fly-camera validation lab, not streaming: the whole source is
//! resident, there is no collision, and no performance claim is made. Meshes are
//! rebuilt synchronously after an edit or a mode switch, never per frame, and no
//! background task exists. The GPU renderer is dropped before suspension returns
//! while the authoritative CPU state stays in memory.

use crate::controls::{Camera, Controls};
use glam::{Vec2, Vec3};
use matterweave_core::{
    coarse::{tile_span, COARSE_TILE_EDGE},
    CoarseTile, Mesh, RayHit, World,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use std::{path::PathBuf, sync::Arc, time::Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};

/// Deterministic source terrain used when the lab has no save to load.
const LAB_SEED: u64 = 20261214;
/// Fine-cell area of the lab: `x` and `z` span `LAB_MIN..LAB_MAX`.
const LAB_MIN: i32 = -32;
const LAB_MAX: i32 = 32;
/// Exclusive upper `y` bound for source cells and edits.
const LAB_TOP_Y: i32 = 16;
/// Coarse level of the mixed mode's far half; one tile spans 32 fine cells.
const COARSE_LEVEL: u8 = 1;
/// Aligned edit edge. One level-1 coarse cell is `1 << COARSE_LEVEL` fine cells.
const EDIT_EDGE: i32 = 2;
/// Material written by BUILD; matches the shared [`World::mesh`] palette.
const BUILD_MATERIAL: u8 = 3;
/// Longest ray the lab edits along, independent of the world coordinate limit.
const EDIT_REACH: f32 = 72.;
/// The only file the lab reads or writes.
const SAVE_FILE: &str = "terrain-lab.json";
/// Upper bounds accepted from a loaded save before any mesh work starts. The lab
/// area already caps a save at 16 chunks; these guard against future widening.
const MAX_LAB_CHUNKS: usize = 64;
const MAX_LAB_SOLID: usize = 64 * 4096;

/// Visual presentation of the single authoritative source world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Fine,
    Mixed,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::Fine => "FINE REFERENCE - EVERY CELL EXACT",
            Self::Mixed => "MIXED - NEAR HALF FINE, FAR HALF LEVEL 1",
        }
    }
}

/// Lab controls besides touch move/look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LabAction {
    Fine,
    Mixed,
    Dig,
    Build,
    Save,
    Reload,
    Back,
}

impl LabAction {
    fn id(self) -> u32 {
        self as u32
    }

    fn from_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(Self::Fine),
            1 => Some(Self::Mixed),
            2 => Some(Self::Dig),
            3 => Some(Self::Build),
            4 => Some(Self::Save),
            5 => Some(Self::Reload),
            6 => Some(Self::Back),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Fine => "FINE",
            Self::Mixed => "MIXED",
            Self::Dig => "DIG",
            Self::Build => "BUILD",
            Self::Save => "SAVE",
            Self::Reload => "RELOAD",
            Self::Back => "BACK",
        }
    }
}

/// Labeled HUD buttons in the 1000x600 HUD space, also the touch hit zones.
const BUTTONS: [(LabAction, [f32; 4]); 7] = [
    (LabAction::Fine, [760., 20., 110., 48.]),
    (LabAction::Mixed, [880., 20., 110., 48.]),
    (LabAction::Dig, [760., 76., 110., 56.]),
    (LabAction::Build, [880., 76., 110., 56.]),
    (LabAction::Save, [760., 140., 110., 48.]),
    (LabAction::Reload, [880., 140., 110., 48.]),
    (LabAction::Back, [760., 196., 230., 48.]),
];
const MOVE_ZONE: [f32; 4] = [20., 400., 170., 170.];
const ELEVATION_ZONES: [[f32; 4]; 2] = [[210., 460., 90., 64.], [310., 460., 90., 64.]];

/// Fixed 64-bit integer mixer; the stdlib has no stable deterministic hash and
/// the core generator keeps its own mixer private, so the lab derives terrain
/// from the same class of integer noise instead of platform-dependent floats.
fn mix(seed: u64, x: i32, z: i32) -> u64 {
    let mut value = seed
        ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (z as u64).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

/// Deterministic rolling-hills height, `0..=9`, interpolated across an 8-cell
/// lattice with a coarser ridge term. Integer-only, so equal inputs give equal
/// ground on every platform and every run.
fn hill_height(seed: u64, x: i32, z: i32) -> i32 {
    const STEP: i64 = 8;
    let cx = x.div_euclid(STEP as i32);
    let cz = z.div_euclid(STEP as i32);
    let fx = i64::from(x.rem_euclid(STEP as i32));
    let fz = i64::from(z.rem_euclid(STEP as i32));
    let lattice = |dx, dz| (mix(seed, cx + dx, cz + dz) % 8) as i64;
    let a = lattice(0, 0) * (STEP - fx) + lattice(1, 0) * fx;
    let b = lattice(0, 1) * (STEP - fx) + lattice(1, 1) * fx;
    let interpolated = (a * (STEP - fz) + b * fz) / (STEP * STEP);
    let ridge = (mix(seed ^ 0x5eed_1ab5, x.div_euclid(16), z.div_euclid(16)) % 3) as i32;
    interpolated as i32 + ridge
}

/// Builds the deterministic source world: a hillside with a moss/soil/stone
/// profile and scattered stone outcrops. Colocated with the lab so the source is
/// generated exactly once and never depends on the sandbox island fixture.
fn generate_source(seed: u64) -> World {
    let mut world = World::new(seed);
    for x in LAB_MIN..LAB_MAX {
        for z in LAB_MIN..LAB_MAX {
            let height = hill_height(seed, x, z);
            for y in 0..=height {
                let material = if y == height {
                    1
                } else if y + 2 >= height {
                    2
                } else {
                    3
                };
                world.set([x, y, z], material);
            }
        }
    }
    // Stone outcrops on a sparse lattice, visible from the default camera and
    // coarse enough that digging one coarse cell leaves an obvious scar.
    let mut x = LAB_MIN + 1;
    while x < LAB_MAX - 1 {
        let mut z = LAB_MIN + 1;
        while z < LAB_MAX - 1 {
            let noise = mix(seed ^ 0x5eed_1ab5, x, z);
            if noise.is_multiple_of(9) {
                let base = hill_height(seed, x, z) + 1;
                let raised = (noise % 3) as i32;
                for dx in 0..EDIT_EDGE {
                    for dz in 0..EDIT_EDGE {
                        for dy in 0..=raised {
                            world.set([x + dx, base + dy, z + dz], 3);
                        }
                    }
                }
            }
            z += 5;
        }
        x += 5;
    }
    world
}

/// Snaps a fine cell down to the `2 x 2 x 2` region aligned to the level-1
/// coarse grid that contains it. Euclidean division keeps negative coordinates
/// aligned to the same lattice as positive ones.
fn align_to_coarse(cell: [i32; 3]) -> [i32; 3] {
    cell.map(|value| value.div_euclid(EDIT_EDGE) * EDIT_EDGE)
}

/// Aligned coarse region a BUILD fills: the region containing the hit surface
/// stepped one whole coarse cell along the hit face normal. Stepping from the
/// aligned hit region keeps the edit coarse-granular even when the hit cell is
/// itself unaligned: one fine step from an unaligned cell can land inside its
/// own region instead of the neighboring one.
fn build_region(hit: &RayHit) -> [i32; 3] {
    let hit_region = align_to_coarse(hit.cell);
    std::array::from_fn(|axis| hit_region[axis] + hit.normal[axis] * EDIT_EDGE)
}

/// Appends one derived part to an aggregate mesh, rebasing its indices.
fn append_mesh(target: &mut Mesh, part: Mesh) {
    let base = target.vertices.len() as u32;
    target.vertices.extend(part.vertices);
    target
        .indices
        .extend(part.indices.into_iter().map(|index| index + base));
}

/// Meshes one coarse tile inside a tile-local temporary world, then bakes the
/// coarse scale and tile origin into the vertices. Only `COARSE_TILE_EDGE^3`
/// cells are written, so the tile never expands into fine cells, and the shared
/// [`World::mesh`] palette applies unchanged.
fn coarse_tile_mesh(tile: &CoarseTile) -> Mesh {
    let mut local = World::new(tile.seed());
    for z in 0..COARSE_TILE_EDGE {
        for y in 0..COARSE_TILE_EDGE {
            for x in 0..COARSE_TILE_EDGE {
                if let Some(material) = tile.material([x, y, z]) {
                    if material != 0 {
                        local.set([x, y, z], material);
                    }
                }
            }
        }
    }
    let mut mesh = local.mesh();
    let scale = (1 << tile.level()) as f32;
    let [ox, oy, oz] = tile.origin();
    for vertex in &mut mesh.vertices {
        vertex.position = [
            ox as f32 + vertex.position[0] * scale,
            oy as f32 + vertex.position[1] * scale,
            oz as f32 + vertex.position[2] * scale,
        ];
    }
    mesh.revision = 0;
    mesh
}

/// Rejects a loaded world that is not bounded by the lab area before any mesh
/// work can start. Bounds are checked per chunk so an arbitrary save cannot
/// steer meshing at far coordinates or demand unbounded geometry. Streaming
/// worlds are rejected outright: their stored chunks are only the resident set,
/// so per-chunk bounds prove nothing about the procedural terrain the source
/// would derive and the far-half coarse tiles would read.
fn validate_lab_world(world: World) -> Result<World, String> {
    if world.is_streaming() {
        return Err("streaming save rejected: resident chunks do not bound lab work".to_string());
    }
    let stats = world.stats();
    if stats.chunks > MAX_LAB_CHUNKS {
        return Err(format!(
            "save holds {} chunks, lab limit is {MAX_LAB_CHUNKS}",
            stats.chunks
        ));
    }
    if stats.solid_voxels > MAX_LAB_SOLID {
        return Err(format!(
            "save holds {} voxels, lab limit is {MAX_LAB_SOLID}",
            stats.solid_voxels
        ));
    }
    let x_keys = LAB_MIN.div_euclid(16)..=(LAB_MAX - 1).div_euclid(16);
    let y_keys = 0..=(LAB_TOP_Y - 1).div_euclid(16);
    let z_keys = x_keys.clone();
    for key in world.chunk_keys() {
        if !x_keys.contains(&key[0]) || !y_keys.contains(&key[1]) || !z_keys.contains(&key[2]) {
            return Err(format!("chunk {key:?} is outside the lab area"));
        }
    }
    Ok(world)
}

/// Default fly camera: above the near edge, looking across the seam at the far
/// coarse half.
fn lab_camera() -> Camera {
    Camera {
        position: Vec3::new(0., 20., 46.),
        yaw: std::f32::consts::PI,
        pitch: -0.32,
    }
}

pub struct TerrainLab {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    /// Authoritative fine cells. A mode switch or a save never mutates them;
    /// only a bounded dig/build edit does.
    source: World,
    camera: Camera,
    controls: Controls,
    lighting: LightingSettings,
    mode: Mode,
    /// Monotonic identity of the derived aggregate. The renderer ignores an
    /// upload whose revision is older than its cache, so a reload with a lower
    /// world revision must still produce a newer identity.
    mesh_revision: u64,
    upload_pending: bool,
    status: String,
    save_path: PathBuf,
    frames: u64,
    frame_limit: Option<u64>,
    last_frame: Instant,
    frame_ms: f64,
    cpu_ms: f64,
    focused: bool,
    /// Requested by BACK; the owning experience switches menus.
    pub return_to_menu: bool,
    pub failed: bool,
}

impl TerrainLab {
    /// Loads `directory/terrain-lab.json` when it is present and bounded, then
    /// generates the deterministic hillside otherwise. A save that fails to load
    /// is left byte-for-byte untouched.
    pub fn new(directory: PathBuf, frame_limit: Option<u64>) -> Self {
        let save_path = directory.join(SAVE_FILE);
        let (source, status) = if save_path.exists() {
            match World::load(&save_path)
                .map_err(|error| error.to_string())
                .and_then(validate_lab_world)
            {
                Ok(world) => (world, "Loaded terrain-lab.json".to_string()),
                Err(error) => {
                    log::error!("Terrain Lab load failed: {error}");
                    (
                        generate_source(LAB_SEED),
                        format!("LOAD FAILED: {error}; generated a fresh lab"),
                    )
                }
            }
        } else {
            (
                generate_source(LAB_SEED),
                "Generated hillside. Dig, build, save.".to_string(),
            )
        };
        let mut lab = Self {
            renderer: None,
            window: None,
            source,
            camera: lab_camera(),
            controls: Controls::default(),
            lighting: LightingSettings {
                sun: Sun {
                    direction_to_sun: [0.4, 0.85, 0.3],
                    intensity: 0.85,
                },
                shadows: true,
                shadow_map_size: 1024,
            },
            mode: Mode::Fine,
            mesh_revision: 0,
            upload_pending: true,
            status,
            save_path,
            frames: 0,
            frame_limit,
            last_frame: Instant::now(),
            frame_ms: 0.,
            cpu_ms: 0.,
            focused: true,
            return_to_menu: false,
            failed: false,
        };
        lab.configure_zones();
        log::info!(
            "Terrain Lab controls: WASD move, Space/Shift up/down, right-drag look; F fine, M mixed, G dig, B build, F5 save, R reload, Esc back"
        );
        lab
    }

    /// Registers the labeled button zones and the touch move/look zones on the
    /// shared [`Controls`] input service.
    fn configure_zones(&mut self) {
        let service = &mut self.controls.service;
        service.clear_button_zones();
        service.clear_motion_zones();
        service.clear_look_zones();
        for (action, rect) in BUTTONS {
            service.add_button_zone(rect, action.id());
        }
        service.set_move_zone(MOVE_ZONE, 70.);
        service.add_motion_zone(ELEVATION_ZONES[0], [0., 1., 0.], false);
        service.add_motion_zone(ELEVATION_ZONES[1], [0., -1., 0.], false);
    }

    fn wants_frames(&self) -> bool {
        self.focused || self.frame_limit.is_some()
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.upload_pending = true;
        self.status = format!("Mode: {}", mode.label());
    }

    fn trigger(&mut self, action: LabAction) {
        match action {
            LabAction::Fine => self.set_mode(Mode::Fine),
            LabAction::Mixed => self.set_mode(Mode::Mixed),
            LabAction::Dig => self.dig(),
            LabAction::Build => self.build(),
            LabAction::Save => self.save(),
            LabAction::Reload => {
                let _ = self.reload();
            }
            LabAction::Back => {
                self.return_to_menu = true;
                self.status = "Back to menu".to_string();
            }
        }
    }

    /// The authoritative surface cell under the camera, if any.
    fn aim(&self) -> Option<RayHit> {
        self.source.raycast(
            self.camera.position.to_array(),
            self.camera.forward().to_array(),
            EDIT_REACH,
        )
    }

    /// Removes the aligned coarse cell containing the hit surface.
    fn dig(&mut self) {
        let Some(hit) = self.aim() else {
            self.status = "No surface within reach".to_string();
            return;
        };
        let base = align_to_coarse(hit.cell);
        match self.stamp_region(base, 0) {
            Ok(0) => self.status = "That coarse cell is already empty".to_string(),
            Ok(changed) => self.status = format!("Dug {changed} cells"),
            Err(error) => self.status = error,
        }
    }

    /// Fills the aligned coarse cell adjacent to the hit surface, so a build
    /// stacks on the aimed face instead of rewriting it in place.
    fn build(&mut self) {
        let Some(hit) = self.aim() else {
            self.status = "No surface within reach".to_string();
            return;
        };
        let base = build_region(&hit);
        match self.stamp_region(base, BUILD_MATERIAL) {
            Ok(0) => self.status = "That coarse cell is already solid".to_string(),
            Ok(changed) => self.status = format!("Built {changed} cells"),
            Err(error) => self.status = error,
        }
    }

    /// Applies one aligned `2 x 2 x 2` region to the authoritative world. All
    /// rejection happens before the first cell changes, so an edit is
    /// all-or-nothing, and the returned count is the number of changed cells.
    ///
    /// The revision preflight is exact: it counts the cells that would actually
    /// change and verifies `revision + changing` cannot overflow, so a world at
    /// the revision limit is rejected instead of applying a partial stamp. For a
    /// non-streaming world `World::set` fails only on a no-op or an exhausted
    /// revision, both excluded here, so every counted cell must commit.
    fn stamp_region(&mut self, base: [i32; 3], material: u8) -> Result<usize, String> {
        let inside = base[0] >= LAB_MIN
            && base[0] + EDIT_EDGE <= LAB_MAX
            && base[1] >= 0
            && base[1] + EDIT_EDGE <= LAB_TOP_Y
            && base[2] >= LAB_MIN
            && base[2] + EDIT_EDGE <= LAB_MAX;
        if !inside {
            return Err(format!(
                "Edit [{}, {}, {}] is outside the lab area",
                base[0], base[1], base[2]
            ));
        }
        let mut changing = 0usize;
        for x in base[0]..base[0] + EDIT_EDGE {
            for y in base[1]..base[1] + EDIT_EDGE {
                for z in base[2]..base[2] + EDIT_EDGE {
                    if self.source.get([x, y, z]) != material {
                        changing += 1;
                    }
                }
            }
        }
        if changing == 0 {
            return Ok(0);
        }
        if self
            .source
            .revision()
            .checked_add(changing as u64)
            .is_none()
        {
            return Err(format!(
                "Edit rejected: {changing} cells would pass the world revision limit"
            ));
        }
        let mut changed = 0;
        for x in base[0]..base[0] + EDIT_EDGE {
            for y in base[1]..base[1] + EDIT_EDGE {
                for z in base[2]..base[2] + EDIT_EDGE {
                    if self.source.set([x, y, z], material) {
                        changed += 1;
                    }
                }
            }
        }
        debug_assert_eq!(changed, changing, "preflight and commit disagree");
        if changed > 0 {
            self.upload_pending = true;
        }
        Ok(changed)
    }

    /// Writes the authoritative world to `directory/terrain-lab.json` through the
    /// core's atomic save. Failures stay visible and never report success.
    fn save(&mut self) {
        match self.source.save(&self.save_path) {
            Ok(()) => {
                self.status = "Saved terrain-lab.json".to_string();
                log::info!(
                    "Terrain Lab saved {} (revision {})",
                    self.save_path.display(),
                    self.source.revision()
                );
            }
            Err(error) => {
                self.status = format!("SAVE FAILED: {error}");
                log::error!("Terrain Lab save failed: {error}");
                eprintln!("Terrain Lab save failed: {error}");
            }
        }
    }

    /// Replaces the source world with the validated lab save. Failure keeps the
    /// current in-memory world and the file untouched.
    fn reload(&mut self) -> Result<(), String> {
        match World::load(&self.save_path)
            .map_err(|error| error.to_string())
            .and_then(validate_lab_world)
        {
            Ok(world) => {
                self.source = world;
                self.upload_pending = true;
                self.status = "Reloaded terrain-lab.json".to_string();
                log::info!("Terrain Lab reloaded {}", self.save_path.display());
                Ok(())
            }
            Err(error) => {
                self.status = format!("RELOAD FAILED: {error}; scene kept");
                log::error!("Terrain Lab reload failed: {error}");
                eprintln!("Terrain Lab reload failed: {error}");
                Err(error)
            }
        }
    }

    /// Derived geometry for the current mode. Fine mode meshes the whole source;
    /// mixed mode merges near-half fine chunks with far-half coarse tiles.
    fn aggregate_mesh(&self) -> Result<Mesh, String> {
        match self.mode {
            Mode::Fine => Ok(self.source.mesh()),
            Mode::Mixed => self.mixed_mesh(),
        }
    }

    fn mixed_mesh(&self) -> Result<Mesh, String> {
        let mut merged = Mesh::default();
        for key in self.source.chunk_keys() {
            if key[2] >= 0 {
                append_mesh(&mut merged, self.source.mesh_chunk(key));
            }
        }
        let span = tile_span(COARSE_LEVEL)
            .ok_or_else(|| format!("coarse level {COARSE_LEVEL} is not supported"))?;
        for kz in LAB_MIN.div_euclid(span)..=(LAB_MAX - 1).div_euclid(span) {
            if kz * span >= 0 {
                continue;
            }
            for kx in LAB_MIN.div_euclid(span)..=(LAB_MAX - 1).div_euclid(span) {
                for ky in 0..=(LAB_TOP_Y - 1).div_euclid(span) {
                    let tile = self
                        .source
                        .coarse_tile(COARSE_LEVEL, [kx, ky, kz])
                        .map_err(|error| error.to_string())?;
                    if tile.is_empty() {
                        continue;
                    }
                    append_mesh(&mut merged, coarse_tile_mesh(&tile));
                }
            }
        }
        Ok(merged)
    }

    /// Rebuilds and uploads the derived aggregate as one renderer mesh. The
    /// whole-world compatibility upload replaces the legacy cache, so no chunk
    /// key is aliased between fine chunks and coarse tiles.
    fn upload_geometry(&mut self) -> Result<(), String> {
        if self.renderer.is_none() || !self.upload_pending {
            return Ok(());
        }
        let begin = Instant::now();
        let mut mesh = self.aggregate_mesh()?;
        self.mesh_revision += 1;
        mesh.revision = self.mesh_revision;
        let triangles = mesh.indices.len() / 3;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.upload(&mesh)?;
        }
        self.upload_pending = false;
        log::info!(
            "Terrain Lab {:?} mesh: {triangles} triangles in {:.1} ms",
            self.mode,
            begin.elapsed().as_secs_f64() * 1000.
        );
        Ok(())
    }

    fn point(&self, x: f64, y: f64) -> Vec2 {
        let size = self
            .window
            .as_ref()
            .map(|window| window.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1000, 600));
        Vec2::new(
            x as f32 / size.width.max(1) as f32 * 1000.,
            y as f32 / size.height.max(1) as f32 * 600.,
        )
    }

    fn touch_start(&mut self, id: u64, point: Vec2) {
        if let Some(action_id) = self.controls.service.pointer_down(id, [point.x, point.y]) {
            if let Some(action) = LabAction::from_id(action_id) {
                self.trigger(action);
            }
        }
    }

    fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.89, 0.95, 0.97, 1.];
        let muted = [0.57, 0.72, 0.77, 1.];
        let accent = [0.52, 0.94, 0.72, 1.];
        let warn = [1., 0.45, 0.35, 1.];
        let panel = [0.025, 0.055, 0.075, 0.87];
        let stats = self.source.stats();
        hud.rect([16., 16., 700., 128.], panel);
        hud.text(30., 28., "MATTERWEAVE TERRAIN LAB", 2., white);
        hud.text(
            30.,
            52.,
            &format!("MODE {}", self.mode.label()),
            1.25,
            accent,
        );
        hud.text(
            30.,
            71.,
            &format!(
                "SOURCE {} VOXELS IN {} CHUNKS | FLY CAMERA, NO COLLISION",
                stats.solid_voxels, stats.chunks
            ),
            1.,
            white,
        );
        hud.text(
            30.,
            88.,
            &format!(
                "FRAME {:.1} MS | MAIN {:.1} MS | FLIGHT: SPACE UP, SHIFT DOWN",
                self.frame_ms, self.cpu_ms
            ),
            1.,
            muted,
        );
        hud.text(
            30.,
            105.,
            &format!(
                "POS {:.0} {:.0} {:.0} | SEED {}",
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                self.source.seed()
            ),
            1.,
            muted,
        );
        if let Some(hit) = self.aim() {
            let base = align_to_coarse(hit.cell);
            hud.text(
                30.,
                122.,
                &format!(
                    "TARGET [{}, {}, {}] AT {:.0} M | DIG ALIGNS [{}, {}, {}]",
                    hit.cell[0], hit.cell[1], hit.cell[2], hit.distance, base[0], base[1], base[2]
                ),
                1.,
                white,
            );
        } else {
            hud.text(30., 122., "NO SURFACE WITHIN REACH", 1., muted);
        }
        let cross = if self.aim().is_some() { accent } else { white };
        hud.rect([491., 299., 18., 2.], cross);
        hud.rect([499., 291., 2., 18.], cross);
        for (action, rect) in BUTTONS {
            let active = matches!(
                (action, self.mode),
                (LabAction::Fine, Mode::Fine) | (LabAction::Mixed, Mode::Mixed)
            );
            hud.rect(
                rect,
                if active {
                    [0.12, 0.34, 0.26, 0.9]
                } else {
                    panel
                },
            );
            hud.text(
                rect[0] + 12.,
                rect[1] + rect[3] / 2. - 5.,
                action.label(),
                1.25,
                if active { accent } else { white },
            );
        }
        hud.rect(MOVE_ZONE, [0.04, 0.10, 0.13, 0.52]);
        hud.text(MOVE_ZONE[0] + 14., MOVE_ZONE[1] + 14., "MOVE", 1.25, white);
        hud.text(MOVE_ZONE[0] + 14., MOVE_ZONE[1] + 36., "DRAG", 1., muted);
        let center = [
            MOVE_ZONE[0] + MOVE_ZONE[2] / 2.,
            MOVE_ZONE[1] + MOVE_ZONE[3] / 2.,
        ];
        hud.rect([center[0] - 18., center[1] - 1., 36., 2.], accent);
        hud.rect([center[0] - 1., center[1] - 18., 2., 36.], accent);
        for (rect, label) in ELEVATION_ZONES.into_iter().zip(["UP", "DOWN"]) {
            hud.rect(rect, panel);
            hud.text(rect[0] + 16., rect[1] + 26., label, 1.25, white);
        }
        hud.text(210., 540., "HOLD UP/DOWN TO FLY", 1., muted);
        hud.text(60., 350., "DRAG ANYWHERE ELSE TO LOOK", 1., muted);
        hud.rect([210., 392., 700., 46.], panel);
        hud.text(
            223.,
            402.,
            &self.status.chars().take(80).collect::<String>(),
            1.,
            if self.status.contains("FAILED") {
                warn
            } else {
                accent
            },
        );
        hud.text(
            223.,
            420.,
            "DIG/BUILD EDIT THE HIT COARSE CELL | SAVE W: terrain-lab.json",
            1.,
            muted,
        );
        #[cfg(not(target_os = "android"))]
        hud.text(
            30.,
            588.,
            "WASD MOVE | SPACE/SHIFT UP/DOWN | RMB LOOK | F FINE | M MIXED | G DIG | B BUILD | F5 SAVE | R RELOAD | ESC BACK",
            1.,
            white,
        );
        #[cfg(target_os = "android")]
        hud.text(
            30.,
            588.,
            "TOUCH PAD MOVES | DRAG LOOKS | TAP LABELED BUTTONS",
            1.,
            white,
        );
        hud
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) {
        if !self.wants_frames() {
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 || self.renderer.is_none() {
            return;
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms = if self.frames == 0 {
            0.
        } else {
            self.frame_ms * 0.9 + f64::from(dt) * 100.
        };
        let (motion, look) = self.controls.consume();
        self.camera.update(motion, look, dt);
        if self.upload_pending {
            if let Err(error) = self.upload_geometry() {
                log::error!("Terrain Lab geometry failed: {error}");
                eprintln!("Terrain Lab geometry failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        }
        let hud = self.hud();
        let matrix = self
            .camera
            .view_projection(size.width as f32 / size.height as f32);
        let eye = self.camera.position.to_array();
        let outcome =
            self.renderer
                .as_mut()
                .unwrap()
                .render_with_lighting(matrix, eye, &hud, &self.lighting);
        match outcome {
            FrameResult::Presented => self.frames += 1,
            FrameResult::Retry => {}
            FrameResult::OutOfMemory => {
                log::error!("Terrain Lab GPU out of memory");
                eprintln!("Terrain Lab GPU out of memory");
                self.failed = true;
                event_loop.exit();
                return;
            }
            FrameResult::Fatal(error) => {
                log::error!("Terrain Lab render failed: {error}");
                eprintln!("Terrain Lab render failed: {error}");
                self.failed = true;
                self.renderer = None;
                event_loop.exit();
                return;
            }
        }
        self.cpu_ms = now.elapsed().as_secs_f64() * 1000.;
        if !self.failed && self.frame_limit.is_some_and(|limit| self.frames >= limit) {
            let stats = self.source.stats();
            eprintln!(
                "TERRAIN LAB SMOKE PASS: {} frames; mode {:?}; {} voxels in {} chunks",
                self.frames, self.mode, stats.solid_voxels, stats.chunks
            );
            event_loop.exit();
        }
    }
}

impl ApplicationHandler for TerrainLab {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        log::info!("Terrain Lab lifecycle resumed");
        self.controls.clear();
        self.last_frame = Instant::now();
        self.focused = true;
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Matterweave | Terrain Lab")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 768)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("Terrain Lab window creation failed: {error}");
                eprintln!("Terrain Lab window creation failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(renderer) => {
                log::info!("Terrain Lab graphics: {}", renderer.capabilities);
                eprintln!("Terrain Lab graphics: {}", renderer.capabilities);
                self.renderer = Some(renderer);
                self.window = Some(window);
                // A recreated renderer holds no buffers: upload the aggregate again.
                self.upload_pending = true;
            }
            Err(error) => {
                log::error!("Terrain Lab renderer initialization failed: {error}");
                eprintln!("Terrain Lab renderer initialization failed: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        log::info!("Terrain Lab lifecycle suspended");
        // GPU objects must be gone before the Android suspend callback returns.
        // The authoritative world and pending edits stay in CPU memory.
        self.renderer = None;
        self.window = None;
        self.controls.clear();
        self.focused = false;
        self.upload_pending = true;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().is_none_or(|w| w.id() != window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.controls.clear();
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.last_frame = Instant::now();
                if !focused {
                    self.controls.clear();
                }
            }
            WindowEvent::RedrawRequested => self.draw(event_loop),
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if pressed && event.logical_key == Key::Named(NamedKey::BrowserBack) {
                    self.trigger(LabAction::Back);
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if pressed && !event.repeat {
                        match code {
                            KeyCode::Escape => self.trigger(LabAction::Back),
                            KeyCode::KeyF => self.trigger(LabAction::Fine),
                            KeyCode::KeyM => self.trigger(LabAction::Mixed),
                            KeyCode::KeyG => self.trigger(LabAction::Dig),
                            KeyCode::KeyB => self.trigger(LabAction::Build),
                            KeyCode::F5 => self.trigger(LabAction::Save),
                            KeyCode::KeyR => self.trigger(LabAction::Reload),
                            _ => {}
                        }
                    }
                    if pressed {
                        self.controls.keys.insert(code);
                    } else {
                        self.controls.keys.remove(&code);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let point = self.point(position.x, position.y);
                self.controls.mouse(point);
            }
            WindowEvent::MouseInput { state, button, .. } => match button {
                MouseButton::Right => {
                    self.controls.mouse_look = state == ElementState::Pressed;
                }
                MouseButton::Left => match state {
                    ElementState::Pressed => {
                        let point = self.controls.cursor.unwrap_or(Vec2::new(500., 300.));
                        self.touch_start(0, point);
                    }
                    ElementState::Released => self.controls.end(0),
                },
                _ => {}
            },
            WindowEvent::Touch(touch) => {
                let point = self.point(touch.location.x, touch.location.y);
                match touch.phase {
                    TouchPhase::Started => self.touch_start(touch.id, point),
                    TouchPhase::Moved => self.controls.moved(touch.id, point),
                    TouchPhase::Ended => self.controls.end(touch.id),
                    TouchPhase::Cancelled => self.controls.clear(),
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        if self.wants_frames() && self.renderer.is_some() {
            if let Some(window) = &self.window {
                let size = window.inner_size();
                if size.width > 0 && size.height > 0 {
                    window.request_redraw();
                }
            }
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "matterweave-terrain-lab-{}-{name}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fixture(name: &str) -> (PathBuf, TerrainLab) {
        let dir = temp_dir(name);
        let app = TerrainLab::new(dir.clone(), None);
        (dir, app)
    }

    fn button_center(action: LabAction) -> Vec2 {
        let (_, rect) = BUTTONS
            .iter()
            .find(|(candidate, _)| *candidate == action)
            .expect("button exists");
        Vec2::new(rect[0] + rect[2] / 2., rect[1] + rect[3] / 2.)
    }

    /// Writes a minimal valid save with an exact revision. The core owns the
    /// save schema; this fixture only provides the fields `World::load` reads.
    fn write_revision_snapshot(path: &std::path::Path, revision: u64) {
        let mut voxels = vec![0u8; 4096];
        voxels[0] = 3;
        let snapshot = serde_json::json!({
            "format_version": 2,
            "generator_version": 1,
            "seed": 1,
            "revision": revision,
            "chunks": [{"position": [0, 0, 0], "voxels": voxels}],
        });
        std::fs::write(path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    }

    #[test]
    fn touch_elevation_moves_camera_and_releasing_stops_it() {
        let (dir, mut app) = fixture("touch-elevation");
        for (id, rect, direction) in [(71, ELEVATION_ZONES[0], 1.), (72, ELEVATION_ZONES[1], -1.)] {
            app.touch_start(id, Vec2::new(rect[0] + 20., rect[1] + 20.));
            let motion = app.controls.service.consume_motion();
            assert_eq!(motion, [0., direction, 0.]);
            let before = app.camera.position.y;
            app.camera.update(Vec3::from_array(motion), Vec2::ZERO, 0.1);
            assert!((app.camera.position.y - before) * direction > 0.);
            app.controls.end(id);
            assert_eq!(app.controls.service.consume_motion(), [0.; 3]);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_generation_is_deterministic_and_grounded() {
        let first = generate_source(LAB_SEED);
        let second = generate_source(LAB_SEED);
        assert_eq!(first.stats(), second.stats());
        assert_eq!(first.revision(), second.revision());
        assert!(first.stats().solid_voxels > 4000, "a real surface exists");
        for (x, z) in [(LAB_MIN, LAB_MIN), (0, 0), (LAB_MAX - 1, LAB_MAX - 1)] {
            assert_ne!(first.get([x, 0, z]), 0, "ground at {x},{z}");
        }
        assert_eq!(first.get([LAB_MIN - 1, 0, 0]), 0);
        assert_eq!(first.get([LAB_MAX, 0, 0]), 0);
        for key in first.chunk_keys() {
            validate_lab_world(World::new(LAB_SEED)).unwrap();
            assert!(key[0].abs() <= 2 && key[1] == 0 && key[2].abs() <= 2);
        }
    }

    #[test]
    fn mode_toggle_never_touches_the_authoritative_world() {
        let (dir, mut app) = fixture("mode");
        let revision = app.source.revision();
        let stats = app.source.stats();
        let fine = app.aggregate_mesh().unwrap();
        assert!(!fine.vertices.is_empty());
        app.set_mode(Mode::Mixed);
        let mixed = app.aggregate_mesh().unwrap();
        assert_eq!(app.mode, Mode::Mixed);
        assert_eq!(app.source.revision(), revision);
        assert_eq!(app.source.stats(), stats);
        assert!(app.upload_pending);
        assert!(
            mixed.vertices.len() < fine.vertices.len(),
            "coarse far half must be smaller than exact geometry"
        );
        assert!(!mixed.vertices.is_empty());
        for vertex in &mixed.vertices {
            for (axis, value) in vertex.position.iter().enumerate() {
                let (low, high) = if axis == 1 {
                    (0., LAB_TOP_Y as f32)
                } else {
                    (LAB_MIN as f32, LAB_MAX as f32)
                };
                assert!(
                    (low..=high).contains(value),
                    "vertex {vertex:?} escaped the lab area"
                );
            }
        }
        app.set_mode(Mode::Fine);
        assert_eq!(app.source.revision(), revision);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn coarse_tile_mesh_scales_and_bakes_the_tile_origin() {
        let mut world = World::new(7);
        world.set([LAB_MIN, 0, LAB_MIN], 3);
        let span = tile_span(COARSE_LEVEL).unwrap();
        let key = [LAB_MIN.div_euclid(span), 0, LAB_MIN.div_euclid(span)];
        let tile = world.coarse_tile(COARSE_LEVEL, key).unwrap();
        assert_eq!(tile.solid_cells(), 1);
        let mesh = coarse_tile_mesh(&tile);
        let origin = tile.origin().map(|value| value as f32);
        let scale = (1 << COARSE_LEVEL) as f32;
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for vertex in &mesh.vertices {
            for (axis, value) in vertex.position.iter().enumerate() {
                min[axis] = min[axis].min(*value);
                max[axis] = max[axis].max(*value);
            }
        }
        assert_eq!(min, origin, "tile origin is baked into every vertex");
        assert_eq!(
            max,
            [origin[0] + scale, origin[1] + scale, origin[2] + scale],
            "one coarse cell covers scale fine cells"
        );
        assert_eq!(mesh.vertices.len(), 24, "six faces, four corners each");
        assert_eq!(mesh.indices.len(), 36);
    }

    #[test]
    fn aligned_edits_stamp_exactly_one_coarse_region_and_reject_outside() {
        let (dir, mut app) = fixture("stamp");
        assert_eq!(align_to_coarse([-31, 5, 17]), [-32, 4, 16]);
        let changed = app
            .stamp_region([0, LAB_TOP_Y - 2, 0], BUILD_MATERIAL)
            .unwrap();
        assert_eq!(changed, EDIT_EDGE.pow(3) as usize);
        assert_eq!(app.source.get([1, LAB_TOP_Y - 1, 1]), BUILD_MATERIAL);
        assert!(app.upload_pending);
        let revision = app.source.revision();
        assert!(app
            .stamp_region([LAB_MAX - 1, LAB_TOP_Y - 2, 0], BUILD_MATERIAL)
            .is_err());
        assert!(app
            .stamp_region([0, LAB_TOP_Y - 1, 0], BUILD_MATERIAL)
            .is_err());
        assert_eq!(app.source.revision(), revision, "rejection changes nothing");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn edits_persist_through_the_lab_save_and_a_reload() {
        let (dir, mut app) = fixture("persist");
        assert_eq!(app.save_path, dir.join(SAVE_FILE));
        app.stamp_region([0, LAB_TOP_Y - 2, 0], BUILD_MATERIAL)
            .unwrap();
        app.save();
        assert_eq!(app.status, "Saved terrain-lab.json");
        let sentinel = dir.join("world.json");
        std::fs::write(&sentinel, b"sample sentinel").unwrap();
        app.stamp_region([0, LAB_TOP_Y - 2, 0], 0).unwrap();
        assert_eq!(app.source.get([0, LAB_TOP_Y - 2, 0]), 0);
        app.reload().unwrap();
        assert_eq!(app.source.get([0, LAB_TOP_Y - 2, 0]), BUILD_MATERIAL);
        assert!(app.upload_pending);
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"sample sentinel");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_startup_save_with_edits_is_loaded_once_at_construction() {
        let dir = temp_dir("startup");
        let path = dir.join(SAVE_FILE);
        let mut saved = generate_source(LAB_SEED);
        saved.set([0, LAB_TOP_Y - 1, 0], 7);
        saved.save(&path).unwrap();
        let app = TerrainLab::new(dir.clone(), None);
        assert_eq!(app.source.get([0, LAB_TOP_Y - 1, 0]), 7);
        assert_eq!(app.status, "Loaded terrain-lab.json");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_invalid_startup_save_is_kept_and_a_fresh_lab_is_generated() {
        let dir = temp_dir("broken");
        let path = dir.join(SAVE_FILE);
        std::fs::write(&path, b"not a save").unwrap();
        let app = TerrainLab::new(dir.clone(), None);
        assert!(app.status.starts_with("LOAD FAILED"));
        assert!(app.source.stats().solid_voxels > 4000);
        assert_eq!(std::fs::read(&path).unwrap(), b"not a save");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reload_rejects_invalid_and_out_of_area_saves_without_writing() {
        let (dir, mut app) = fixture("reload");
        let revision = app.source.revision();
        std::fs::write(&app.save_path, b"{ broken").unwrap();
        let broken = std::fs::read(&app.save_path).unwrap();
        assert!(app.reload().is_err());
        assert!(app.status.contains("RELOAD FAILED"));
        assert_eq!(app.source.revision(), revision, "current world kept");
        assert_eq!(std::fs::read(&app.save_path).unwrap(), broken);

        let mut outside = World::new(LAB_SEED);
        outside.set([LAB_MAX + 16, 2, 0], 3);
        outside.save(&app.save_path).unwrap();
        let outside_bytes = std::fs::read(&app.save_path).unwrap();
        assert!(app.reload().is_err());
        assert!(app.status.contains("outside the lab area"));
        assert_eq!(app.source.revision(), revision);
        assert_eq!(std::fs::read(&app.save_path).unwrap(), outside_bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hud_buttons_route_to_real_actions() {
        let (dir, mut app) = fixture("buttons");
        assert_eq!(app.mode, Mode::Fine);
        app.touch_start(1, button_center(LabAction::Mixed));
        assert_eq!(app.mode, Mode::Mixed);
        app.touch_start(2, button_center(LabAction::Fine));
        assert_eq!(app.mode, Mode::Fine);
        app.touch_start(3, button_center(LabAction::Dig));
        assert!(app.upload_pending);
        app.touch_start(4, button_center(LabAction::Save));
        assert_eq!(app.status, "Saved terrain-lab.json");
        app.touch_start(5, button_center(LabAction::Reload));
        assert_eq!(app.status, "Reloaded terrain-lab.json");
        app.touch_start(6, button_center(LabAction::Back));
        assert!(app.return_to_menu);
        app.controls.end(1);
        app.controls.end(2);
        app.controls.end(3);
        app.controls.end(4);
        app.controls.end(5);
        app.controls.end(6);
        assert_eq!(app.controls.service.active_pointers(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_save_is_reported_and_not_silent() {
        let (dir, mut app) = fixture("savefail");
        app.save_path = dir.join("missing").join(SAVE_FILE);
        app.save();
        assert!(app.status.starts_with("SAVE FAILED"));
        assert!(!app.save_path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn build_steps_one_whole_coarse_cell_from_the_aligned_hit_region() {
        // Unaligned hit cell stepping towards -x: one fine step from the hit
        // cell stays inside the same aligned region, so BUILD must step a whole
        // coarse cell from the hit region instead.
        let mut world = World::new(1);
        world.set([3, 5, 2], 3);
        let hit = world
            .raycast([-1., 5.5, 2.5], [1., 0., 0.], 8.)
            .expect("ray hits the block");
        assert_eq!(hit.cell, [3, 5, 2]);
        assert_eq!(hit.normal, [-1, 0, 0]);
        assert_eq!(build_region(&hit), [0, 4, 2]);
        let fine_step = align_to_coarse(std::array::from_fn(|axis| {
            hit.cell[axis] + hit.normal[axis]
        }));
        assert_ne!(
            build_region(&hit),
            fine_step,
            "fine-step derivation is wrong here"
        );

        // End to end through the real build path and authoritative world.
        let (dir, mut app) = fixture("build");
        app.source = World::new(1);
        app.source.set([3, 5, 2], 3);
        app.camera.position = Vec3::new(-1., 5.5, 2.5);
        app.camera.yaw = std::f32::consts::FRAC_PI_2;
        app.camera.pitch = 0.;
        app.build();
        assert_eq!(app.status, "Built 8 cells");
        assert_eq!(app.source.get([0, 4, 2]), BUILD_MATERIAL);
        assert_eq!(
            app.source.get([2, 4, 2]),
            0,
            "the fine-step region must stay untouched"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_streaming_save_is_rejected_before_bounds_can_be_trusted() {
        let (dir, mut app) = fixture("streaming");
        let revision = app.source.revision();
        // A streaming snapshot with no resident chunks passes every per-chunk
        // bound yet describes procedural terrain the resident set cannot bound.
        let snapshot = serde_json::json!({
            "format_version": 2,
            "generator_version": 1,
            "seed": 1,
            "revision": 0,
            "chunks": [],
            "streaming": {"extension_version": 1, "empty_overrides": []},
        });
        std::fs::write(&app.save_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
        let bytes = std::fs::read(&app.save_path).unwrap();
        let error = app.reload().unwrap_err();
        assert!(error.contains("streaming"), "got: {error}");
        assert_eq!(app.source.revision(), revision, "current world kept");
        assert_eq!(std::fs::read(&app.save_path).unwrap(), bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn revision_exhaustion_rejects_partial_stamps_and_commits_whole_ones() {
        let (dir, mut app) = fixture("revision");
        // Eight changing cells against one unit of headroom must reject before
        // the first cell commits, not stop after it.
        write_revision_snapshot(&app.save_path, u64::MAX - 1);
        app.reload().unwrap();
        assert_eq!(app.source.revision(), u64::MAX - 1);
        let stats = app.source.stats();
        let error = app
            .stamp_region([0, LAB_TOP_Y - 2, 0], BUILD_MATERIAL)
            .unwrap_err();
        assert!(error.contains("revision"), "got: {error}");
        assert_eq!(app.source.revision(), u64::MAX - 1, "no revision consumed");
        assert_eq!(app.source.stats(), stats, "no cell changed");
        // A no-op stamp needs no revision and still reports zero changes.
        assert_eq!(app.stamp_region([2, 0, 0], 0).unwrap(), 0);

        // Exactly eight units of headroom commit the whole stamp to MAX...
        write_revision_snapshot(&app.save_path, u64::MAX - 8);
        app.reload().unwrap();
        assert_eq!(
            app.stamp_region([0, LAB_TOP_Y - 2, 0], BUILD_MATERIAL)
                .unwrap(),
            EDIT_EDGE.pow(3) as usize
        );
        assert_eq!(app.source.revision(), u64::MAX);
        // ...and the next changing stamp is rejected with nothing applied.
        let error = app
            .stamp_region([2, LAB_TOP_Y - 2, 0], BUILD_MATERIAL)
            .unwrap_err();
        assert!(error.contains("revision"), "got: {error}");
        assert_eq!(app.source.revision(), u64::MAX);
        assert_eq!(app.source.get([2, LAB_TOP_Y - 2, 0]), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
