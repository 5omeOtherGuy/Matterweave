//! Far-terrain sample: the authoritative streaming window plus three nested
//! distance rings derived from the same generator, out to six kilometres.
//!
//! The 7x7-chunk window around the player is the real world: authoritative one
//! metre voxels, streamed and meshed exactly as the other samples stream them.
//! Everything beyond it is derived on demand from
//! [`matterweave_core::landscape`] at 4 m, 16 m and 64 m cells by
//! [`matterweave_core::landscape::ring_plan`], which guarantees the rings and
//! the window cover every surface cell exactly once. Nothing outside the window
//! is authoritative, editable or collidable, and no ring tile is ever written to
//! a save.
//!
//! There is no physics here: walk mode follows
//! [`matterweave_core::landscape::height_at`] directly. That is deliberate for
//! this slice - the point is the distance rings, not another collision path.
//!
//! # Budgets
//!
//! Tile meshes are cheap (33x33 = 1089 column samples each) but not free, so a
//! frame builds at most [`MAX_TILE_UPLOADS`] of them and stops as soon as
//! [`MAX_TILE_MS`] of main-thread time has gone into copying them to the GPU.
//! Generation runs on [`landscape_tiles::TileWorker`], a bounded background
//! queue, and every generated mesh is kept in [`landscape_tiles::TileCache`], so
//! a tile the plan drops and wants again is re-uploaded instead of regenerated.
//! Residency is bounded by [`matterweave_render::MAX_TERRAIN_TILES`] and every
//! tile the plan drops is evicted the same frame, so the drawing set cannot grow
//! with the path the camera took.
//!
//! # Frame shape
//!
//! One submission is in flight at a time: the renderer's upload and retain calls
//! wait on that submission's fence, so *when* this sample touches the renderer
//! decides how much of the frame overlaps the previous one. A frame therefore
//! runs in two steps - [`LandscapeSample::prepare_geometry`], which is CPU work
//! only (streaming, background mesh requests, tile planning, water derivation),
//! then [`LandscapeSample::upload_geometry`], which takes the frame's single
//! fence wait explicitly and does every upload and retain. Everything that is
//! only generation therefore happens while the previous frame is still on the
//! GPU, instead of serializing behind the fence.
use crate::capture::Capture;
use crate::controls::{Camera, Controls};
use crate::landscape_flora::{LandscapeFlora, PUSH_RADIUS_M, WIND_DIRECTION_XZ, WIND_STRENGTH_M};
use crate::landscape_tiles::{self, TileCache, TileJob, TileWorker, DEFAULT_TILE_CACHE_BYTES};
use crate::metrics;
use glam::{Mat4, Vec2, Vec3};
use matterweave_core::landscape::{
    self, Clip, RingTile, TileFilter, LANDSCAPE_RINGS, LOD_TILE_CELLS,
};
use matterweave_core::{AsyncWorld, Mesh, World};
use matterweave_render::{
    Atmosphere, FrameResult, Hud, LightingSettings, PlayerPush, Renderer, Sun, TerrainTileKey,
    Wind, MAX_TERRAIN_TILES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

/// Opt-in marker beside the save. NativeActivity passes no command line, so a
/// device run selects this sample by creating this file:
/// `adb shell run-as dev.matterweave.explorer touch files/landscape.txt`.
pub const MARKER_FILE: &str = "landscape.txt";
/// Accepted marker size. A larger file is reported and ignored, never truncated
/// and never parsed.
pub const MAX_MARKER_BYTES: u64 = 128;

/// Generator seed of the sample world. The generator is a versioned function of
/// this seed, so the same build always shows the same landscape.
pub const SEED: u64 = 20260913;

/// Tile meshes uploaded in one frame, and the most tiles one frame may request
/// from the background generator. Both are bounded so a saturated frame drops
/// work instead of queuing it.
pub const MAX_TILE_UPLOADS: usize = 8;
/// Main-thread milliseconds one frame may spend declaring and uploading tiles.
/// The first upload's fence wait is taken explicitly before this phase (see
/// [`LandscapeSample::upload_geometry`]), so this bounds tile work rather than
/// the GPU wait the frame cannot avoid.
/// Checked after each tile, so one tile can overrun it; the next cannot start.
pub const MAX_TILE_MS: f64 = 2.0;
/// Completed tile meshes one frame takes from the background worker. Bounded by
/// the worker's own result bound as well.
pub const MAX_TILE_POLLS: usize = 4;

/// Chunk meshes one frame takes from the background preparation pool and
/// uploads, and chunk mesh requests one frame may queue. Both are the bounds the
/// sandbox uses for the same pool.
pub const MAX_CHUNK_UPLOADS: usize = 4;
pub const MAX_CHUNK_REQUESTS: usize = 4;
/// Main-thread milliseconds one frame may spend taking completed chunk meshes
/// and requesting new ones. Uploading them is bounded by [`MAX_CHUNK_UPLOADS`].
pub const MAX_CHUNK_MS: f64 = 2.0;

/// Derived water meshes built and uploaded in one frame. Only the sea-level
/// chunk layer carries water, so this is at most one mesh per column stack.
pub const MAX_WATER_UPLOADS: usize = 8;
/// Main-thread milliseconds one frame may spend deriving water meshes.
pub const MAX_WATER_MS: f64 = 2.0;

/// Eye height above the surface in walk mode.
const EYE_HEIGHT: f32 = 1.7;
/// Fly speed in metres per second.
const FLY_SPEED: f32 = 90.0;
/// Camera limits. The horizontal clamp keeps the eye inside the simulation
/// domain, which is where the ring planner's coverage contract holds and where
/// the streaming window is a full 7x7 square.
/// The eye stays a window's width inside the landscape domain, so the streaming
/// window and the ring planner never see a clamped square.
const MAX_EYE_XZ: f32 = (matterweave_core::LANDSCAPE_WORLD_LIMIT - 64) as f32;
const MIN_EYE_Y: f32 = -30.0;
const MAX_EYE_Y: f32 = 420.0;

/// Far plane. The outermost ring reaches its half-extent from the eye, plus the diagonal
/// of its own square, so the frustum has to hold about 9 km.
const FAR_PLANE: f32 = 9500.0;

/// Aerial perspective for this sample.
///
/// `1 - exp(-d * 0.0006)` leaves about 9% of a surface's own colour at 4 km, so
/// ridge and valley are still told apart there, and about 2.5% at the 6144 m
/// outer edge, where the ring ends against a background cleared to the same
/// sky. Host judgement on llvmpipe at 1280x768, not a device measurement, and
/// the one number to turn if the horizon reads wrong on the phone.
const FOG_DENSITY: f32 = 0.0006;
const SKY: [f32; 3] = [0.60, 0.71, 0.82];

/// Whether the opt-in marker is present beside the save.
///
/// The file's content is reserved and ignored: any bytes within the bound select
/// the sample. A marker that cannot be read, or that exceeds the bound, is
/// reported and treated as absent - a broken marker must not stop the app from
/// starting, and this file is never a game save.
/// Marker selecting the scripted exercise path (a 200 m circle flown at 60 m):
/// the device entry for streaming and per-frame-budget evidence, because a
/// phone run has no command line.
pub const EXERCISE_MARKER_FILE: &str = "landscape-exercise.txt";

/// True when the exercise marker is present and readable, with the same
/// tolerance as [`marker_present`]: an unreadable marker is reported and
/// treated as absent rather than failing the run.
pub fn exercise_marker_present(directory: &Path) -> bool {
    use std::io::Read;
    let path = directory.join(EXERCISE_MARKER_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            log::warn!("Exercise marker {}: {error}", path.display());
            return false;
        }
    };
    let mut text = String::new();
    if let Err(error) = file.take(MAX_MARKER_BYTES + 1).read_to_string(&mut text) {
        log::warn!("Exercise marker {}: {error}", path.display());
        return false;
    }
    if text.len() as u64 > MAX_MARKER_BYTES {
        log::warn!(
            "Exercise marker {}: larger than {MAX_MARKER_BYTES} bytes",
            path.display()
        );
        return false;
    }
    let text = text.trim();
    text.is_empty() || !text.eq_ignore_ascii_case("off")
}

pub fn marker_present(directory: &Path) -> bool {
    use std::io::Read;
    let path = directory.join(MARKER_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(error) => {
            log::warn!("Landscape marker {}: {error}", path.display());
            return false;
        }
    };
    let mut text = String::new();
    match file.take(MAX_MARKER_BYTES + 1).read_to_string(&mut text) {
        Ok(_) if text.len() as u64 > MAX_MARKER_BYTES => {
            log::warn!(
                "Landscape marker {} exceeds {MAX_MARKER_BYTES} bytes; ignored",
                path.display()
            );
            false
        }
        // Content is reserved: whatever it says, the marker selects the sample.
        Ok(_) => true,
        Err(error) => {
            log::warn!("Landscape marker {}: {error}", path.display());
            false
        }
    }
}

// -- Tile work planning ------------------------------------------------------

/// Per-frame limits for [`plan_tile_work`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileBudget {
    /// Tiles that may be built and uploaded this frame.
    pub uploads: usize,
    /// Hard bound on resident tiles, including the ones built this frame.
    pub resident: usize,
}

impl Default for TileBudget {
    fn default() -> Self {
        Self {
            uploads: MAX_TILE_UPLOADS,
            resident: MAX_TERRAIN_TILES,
        }
    }
}

/// What one frame should do to the tile cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileWork {
    /// Tiles to build and upload, in plan order: nearest ring first.
    pub build: Vec<RingTile>,
    /// Resident tiles the plan no longer wants.
    pub evict: Vec<TerrainTileKey>,
    /// Tiles the plan still needs after this frame's budget is spent. Zero
    /// means the rings are fully resident for this eye position.
    pub outstanding: usize,
}

/// The filter that actually affects one tile, with parts that cannot change its
/// mesh removed.
///
/// A ring's hole is the whole previous ring's square and its bound is the whole
/// ring square; most tiles are cut by neither. Normalizing lets the cache
/// compare filters to decide what to rebuild: without it, every tile of the
/// innermost ring would look different each time the streaming window moves one
/// chunk, and all 64 would be rebuilt for a 16 m step.
pub fn normalized_filter(tile: &RingTile) -> TileFilter {
    let span = LOD_TILE_CELLS * landscape::lod_cell_m(tile.level);
    let square = Clip {
        min: [tile.key[0] * span, tile.key[1] * span],
        max: [tile.key[0] * span + span, tile.key[1] * span + span],
    };
    let overlap = |clip: Clip| -> Option<Clip> {
        let min = [
            clip.min[0].max(square.min[0]),
            clip.min[1].max(square.min[1]),
        ];
        let max = [
            clip.max[0].min(square.max[0]),
            clip.max[1].min(square.max[1]),
        ];
        (min[0] < max[0] && min[1] < max[1]).then_some(Clip { min, max })
    };
    let contains = |clip: Clip| {
        clip.min[0] <= square.min[0]
            && clip.min[1] <= square.min[1]
            && clip.max[0] >= square.max[0]
            && clip.max[1] >= square.max[1]
    };
    TileFilter {
        // A hole outside the tile removes nothing.
        hole: tile.filter.hole.and_then(overlap),
        // A bound containing the tile removes nothing.
        bound: tile.filter.bound.filter(|bound| !contains(*bound)),
    }
}

/// Diff a plan against the resident tiles and apply this frame's budget.
///
/// A tile is rebuilt when it is absent or when its normalized filter changed,
/// which is what happens to the few innermost tiles the streaming window cuts
/// into as the player walks. Eviction is not budgeted: a tile the plan dropped
/// is off-screen, and keeping it **resident** would draw its square a second
/// time under the coarser tile that replaced it, which is the coverage
/// guarantee the ring plan exists to provide. What survives a drop is the CPU
/// mesh in [`landscape_tiles::TileCache`], so a tile the eye returns to is a
/// re-upload instead of a regeneration.
pub fn plan_tile_work(
    plan: &[RingTile],
    resident: &BTreeMap<TerrainTileKey, TileFilter>,
    budget: TileBudget,
) -> TileWork {
    let wanted: BTreeSet<TerrainTileKey> = plan.iter().map(|tile| (tile.level, tile.key)).collect();
    let evict: Vec<TerrainTileKey> = resident
        .keys()
        .filter(|key| !wanted.contains(*key))
        .copied()
        .collect();
    let kept = resident.len() - evict.len();
    let mut build = Vec::new();
    let mut outstanding = 0;
    let mut new_keys = 0;
    for tile in plan {
        let key = (tile.level, tile.key);
        let current = resident.get(&key);
        if current == Some(&normalized_filter(tile)) {
            continue;
        }
        // Only growth is bounded: rebuilding a tile that is already resident
        // replaces its buffers and cannot push the cache over its limit.
        let grows = current.is_none();
        let blocked = grows && kept + new_keys + 1 > budget.resident;
        if build.len() >= budget.uploads || blocked {
            outstanding += 1;
            continue;
        }
        new_keys += usize::from(grows);
        build.push(*tile);
    }
    TileWork {
        build,
        evict,
        outstanding,
    }
}

// -- Sample ------------------------------------------------------------------

/// Open on a shore: the first 64 m grid point (scanned from the origin outward,
/// so the same build always opens in the same place) whose own ground is low
/// land and which has open water 64 m away. Standing there puts the near water
/// pass and the vegetation field in the same frame, with the mountains and the
/// distance rings beyond them.
fn spawn_camera() -> Camera {
    for gz in (-24..24).rev() {
        for gx in -24..24 {
            let (x, z) = (gx * 64, gz * 64);
            let ground = landscape::height_at(SEED, x, z);
            if !(2..=14).contains(&ground) {
                continue;
            }
            for (dx, dz) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
            ] {
                if landscape::height_at(SEED, x + dx * 64, z + dz * 64) < -2 {
                    return Camera {
                        position: Vec3::new(x as f32, ground as f32 + 8., z as f32),
                        yaw: (dx as f32).atan2(dz as f32),
                        pitch: -0.05,
                    };
                }
            }
        }
    }
    // No coast found in the probed square: keep the old behaviour rather than
    // failing to start.
    let height = landscape::height_at(SEED, 0, 0) as f32;
    Camera {
        position: Vec3::new(0., height.max(0.) + 40., 0.),
        yaw: 0.6,
        pitch: -0.12,
    }
}

/// Fly camera. The shared [`Camera::update`] is tuned for the 512 m sandbox
/// (10 m/s, 70 m ceiling, a 240 m far plane); a six kilometre view needs its own
/// speed, ceiling and projection, so only the yaw/pitch convention is shared.
fn fly(camera: &mut Camera, motion: Vec3, look: Vec2, dt: f32, speed: f32) {
    camera.yaw -= look.x * 0.004;
    camera.pitch = (camera.pitch - look.y * 0.004).clamp(-1.50, 1.50);
    let forward = Vec3::new(camera.yaw.sin(), 0., camera.yaw.cos());
    let right = Vec3::new(-camera.yaw.cos(), 0., camera.yaw.sin());
    camera.position +=
        (right * motion.x + Vec3::Y * motion.y + forward * motion.z) * speed * dt.clamp(0., 0.05);
    clamp_camera(camera);
}

/// Keep the eye inside the simulation domain. Outside it the streaming window is
/// clamped against the world edge and stops being nested inside the innermost
/// ring, which is exactly the case `ring_plan` documents as unsupported.
fn clamp_camera(camera: &mut Camera) {
    camera.position = camera.position.clamp(
        Vec3::new(-MAX_EYE_XZ, MIN_EYE_Y, -MAX_EYE_XZ),
        Vec3::new(MAX_EYE_XZ, MAX_EYE_Y, MAX_EYE_XZ),
    );
}

fn view_projection(camera: &Camera, aspect: f32) -> [[f32; 4]; 4] {
    (Mat4::perspective_rh(65_f32.to_radians(), aspect.max(0.01), 0.2, FAR_PLANE)
        * Mat4::look_at_rh(camera.position, camera.position + camera.forward(), Vec3::Y))
    .to_cols_array_2d()
}

/// Whether two derived meshes describe the same geometry, ignoring the revision
/// they carry.
///
/// A derived mesh is a pure function of its identity, so equal inputs produce
/// bit-identical floats and equality is exact. The revision is deliberately not
/// compared: a water mesh carries its column stack's chunk revision, which moves
/// whenever the stack identity does, and that is exactly the case this check
/// exists for. Keeping the older buffer cannot hide a later change either, since
/// chunk revisions only increase and the renderer ignores an upload older than
/// the one it holds.
fn same_geometry(a: &Mesh, b: &Mesh) -> bool {
    a.vertices.len() == b.vertices.len()
        && a.indices == b.indices
        && a.vertices
            .iter()
            .zip(&b.vertices)
            .all(|(a, b)| a.position == b.position && a.normal == b.normal && a.color == b.color)
}

/// Counters the HUD shows and the smoke line prints. Every one of them is a
/// count this sample actually performed, not a target.
///
/// The phases are separated because they cost different things: the plan
/// timings are CPU work that overlaps the previous frame's submission, the
/// upload timings are copy work that follows the frame's fence wait, and
/// `fence_ms` is that wait itself.
#[derive(Clone, Copy, Debug, Default)]
struct SyncCounters {
    /// Whole mesh sync: the CPU phase plus the upload phase, per frame.
    fence_ms: f64,
    /// Chunk phase. The plan half is split because "list the window" and "take
    /// and request meshes" are different costs with different fixes.
    chunk_plan_ms: f64,
    chunk_keys_ms: f64,
    chunk_poll_ms: f64,
    chunk_upload_ms: f64,
    chunk_uploads: u64,
    chunk_frame_uploads: u32,
    /// Tile phase.
    plan_ms: f64,
    declare_ms: f64,
    upload_ms: f64,
    /// The part of `upload_ms` spent inside `upload_terrain_tile` proper.
    copy_ms: f64,
    uploaded: u64,
    tile_frame_uploads: u32,
    evicted: u64,
    outstanding: usize,
    /// Tiles the plan wanted while their mesh was still cached, i.e. an upload
    /// instead of a regeneration.
    reused: u64,
    /// Water phase.
    water_plan_ms: f64,
    water_upload_ms: f64,
    water_uploads: u64,
    water_frame_uploads: u32,
    /// Water meshes that were re-derived to the same geometry already resident.
    water_unchanged: u64,
    /// Run totals of each per-frame phase time, for the means the smoke line
    /// prints. Every one is a sum of measured spans, never a modelled value.
    fence_total_ms: f64,
    chunk_plan_total_ms: f64,
    chunk_keys_total_ms: f64,
    chunk_poll_total_ms: f64,
    chunk_upload_total_ms: f64,
    declare_total_ms: f64,
    plan_total_ms: f64,
    upload_total_ms: f64,
    copy_total_ms: f64,
    water_plan_total_ms: f64,
    water_upload_total_ms: f64,
    /// The single worst frame seen so far, chosen by upload time and kept as one
    /// record. Independent maxima taken from different frames would describe a
    /// frame that never happened.
    worst: WorstFrame,
}

/// The most expensive tile frame of a run.
#[derive(Clone, Copy, Debug, Default)]
struct WorstFrame {
    tiles: usize,
    plan_ms: f64,
    upload_ms: f64,
    copy_ms: f64,
}

pub struct LandscapeSample {
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    world: World,
    preparation: AsyncWorld,
    camera: Camera,
    controls: Controls,
    lighting: LightingSettings,
    walking: bool,
    /// Planned tiles for this eye, reused every frame so the plan allocates
    /// nothing after the first frame.
    plan: Vec<RingTile>,
    /// Declared keys handed to the renderer, reused for the same reason.
    declared: Vec<TerrainTileKey>,
    /// Resident tiles and the normalized filter each was built with.
    resident: BTreeMap<TerrainTileKey, TileFilter>,
    /// Background tile generation and the CPU-side mesh cache that makes a tile
    /// the eye returns to a re-upload instead of a regeneration.
    tile_worker: TileWorker,
    tile_cache: TileCache,
    /// Tiles selected for upload this frame, filled by `prepare_geometry` and
    /// drained by `upload_geometry`. Reused so a frame allocates nothing.
    tile_uploads: Vec<RingTile>,
    /// Chunk keys of the streaming window, and the completed chunk meshes this
    /// frame will upload. Reused for the same reason.
    chunk_keys: Vec<[i32; 3]>,
    chunk_uploads: Vec<([i32; 3], Mesh)>,
    tiles: SyncCounters,
    /// Dense vegetation: the pooled prototypes plus the field for the eye's
    /// cell. `None` until the renderer exists, because the first
    /// [`LandscapeFlora::sync`] installs the scene rather than updating one.
    flora: Option<LandscapeFlora>,
    /// Wind animation clock in seconds, advanced by frame time. The renderer
    /// folds it into its animation period, so a long session cannot lose sine
    /// precision here.
    wind_time: f32,
    /// Water meshes uploaded over the run, this frame's derivation and upload
    /// times, and the keys this frame will retain.
    water_uploaded: u64,
    water_keys: Vec<[i32; 3]>,
    water_uploads: Vec<([i32; 3], Mesh)>,
    /// CPU copies of the resident water meshes, so a re-derivation that returns
    /// the same geometry does not replace a GPU buffer for nothing. Tiny: a
    /// water mesh is a few quads, and this holds one per resident sea-level
    /// chunk, bounded by the streaming window.
    water_resident: BTreeMap<[i32; 3], Mesh>,
    /// Vertical chunk layers a water mesh can be derived from: the world's whole
    /// band, because `column_is_flooded` scans a column from its top down to the
    /// band's floor, so an edit in any layer can add or remove a surface.
    water_stack: Vec<i32>,
    /// Reusable scratch for the stack revisions of the key being examined.
    water_stack_now: Vec<Option<u64>>,
    /// Identity each resident water key was derived at: the revision of every
    /// chunk in its column stack.
    ///
    /// The world revision is deliberately *not* the key. A streaming publish
    /// advances it without changing a single column, so keying on it re-derived
    /// every candidate mesh after every chunk step: 437 uploads in 60 flying
    /// frames on the host, saturated against the per-frame cap the whole time. A
    /// stack revision changes exactly when an edit touches a chunk that the
    /// derivation reads.
    water_stamps: BTreeMap<[i32; 3], Vec<Option<u64>>>,
    /// Last sync's window size and sea-level candidate count, for the report.
    water_window: usize,
    water_candidates: usize,
    capture: Capture,
    status: String,
    frames: u64,
    frame_limit: Option<u64>,
    last_frame: Instant,
    frame_ms: f64,
    focused: bool,
    /// Deterministic camera path for smoke runs. Off by default: it exists to
    /// exercise streaming, eviction and the per-frame budget without input.
    pub exercise: bool,
    /// Requested by BACK; the owning experience switches menus.
    pub return_to_menu: bool,
    pub failed: bool,
}

impl LandscapeSample {
    /// `save_path` names where a save would live. This slice makes no edits, so
    /// nothing is ever written there; only its directory is used, for the
    /// capture request and the opt-in marker.
    pub fn new(save_path: PathBuf, frame_limit: Option<u64>) -> Self {
        let directory = crate::data_directory(&save_path).to_path_buf();
        // The capture request is consumed here, at construction, so a device run
        // that selected this sample with the marker records from its first
        // frame. A sample entered from the menu finds the request already taken
        // by the chooser, which is why the marker is the scriptable path.
        let capture = Capture::new("Landscape", &directory);
        if let Some(path) = capture.path() {
            log::info!("Landscape frame capture: {}", path.display());
            eprintln!("Landscape frame capture: {}", path.display());
        }
        let mut world = World::landscape(SEED);
        let mut camera = spawn_camera();
        // The vertical chunk layers a water mesh can be derived from, fixed for
        // the life of this sample's world band.
        let water_stack: Vec<i32> = world.stream_y_chunks().collect();
        let water_stack_now: Vec<Option<u64>> = vec![None; water_stack.len()];
        clamp_camera(&mut camera);
        world.stream_around(camera.position.to_array());
        log::info!(
            "Landscape sample: seed {SEED}, rings {:?}, fog {FOG_DENSITY}/m",
            LANDSCAPE_RINGS.map(|ring| (ring.level, ring.half_extent))
        );
        Self {
            renderer: None,
            window: None,
            world,
            preparation: AsyncWorld::new(),
            camera,
            controls: Controls::default(),
            lighting: LightingSettings {
                sun: Sun {
                    direction_to_sun: [0.35, 0.82, 0.45],
                    intensity: 0.85,
                },
                shadows: true,
                shadow_map_size: 1024,
                atmosphere: Atmosphere {
                    fog_density: FOG_DENSITY,
                    sky: SKY,
                },
                // Real wind and player values, not still air. The position is
                // refreshed to the eye every frame; the constant bearing and
                // strength are the sample's own weather.
                wind: Wind {
                    direction_xz: WIND_DIRECTION_XZ,
                    strength_m: WIND_STRENGTH_M,
                    time_s: 0.0,
                },
                player: PlayerPush {
                    position: [0.0; 3],
                    radius_m: PUSH_RADIUS_M,
                },
            },
            walking: false,
            plan: Vec::new(),
            declared: Vec::new(),
            resident: BTreeMap::new(),
            tile_worker: TileWorker::new(),
            tile_cache: TileCache::new(DEFAULT_TILE_CACHE_BYTES),
            tile_uploads: Vec::new(),
            chunk_keys: Vec::new(),
            chunk_uploads: Vec::new(),
            tiles: SyncCounters::default(),
            flora: None,
            wind_time: 0.,
            water_uploaded: 0,
            water_keys: Vec::new(),
            water_uploads: Vec::new(),
            water_resident: BTreeMap::new(),
            water_stack,
            water_stack_now,
            water_stamps: BTreeMap::new(),
            water_window: 0,
            water_candidates: 0,
            capture,
            status: format!(
                "Fly the landscape. Rings reach {} km.",
                landscape::LANDSCAPE_RINGS
                    .last()
                    .map_or(0, |ring| ring.half_extent)
                    / 1000
            ),
            frames: 0,
            frame_limit,
            last_frame: Instant::now(),
            frame_ms: 0.,
            focused: true,
            exercise: false,
            return_to_menu: false,
            failed: false,
        }
    }

    /// One step of the scripted smoke path: a 200 m circle flown at altitude.
    ///
    /// The radius crosses chunk boundaries constantly and crosses whole 128 m
    /// tiles of the innermost ring, so the plan really does drop tiles and the
    /// upload budget really is reached. The path is a function of the frame
    /// index, not of elapsed time, so a slow host walks the same path.
    fn exercise_step(&mut self) {
        let angle = self.frames as f32 * 0.02;
        let radius = 200.0;
        self.camera.position.x = radius * angle.sin();
        self.camera.position.z = radius * angle.cos();
        let ground = landscape::height_at(
            SEED,
            self.camera.position.x.floor() as i32,
            self.camera.position.z.floor() as i32,
        ) as f32;
        self.camera.position.y = ground + 60.0;
        self.camera.yaw = angle + std::f32::consts::FRAC_PI_2;
        clamp_camera(&mut self.camera);
    }

    fn wants_frames(&self) -> bool {
        self.focused || self.frame_limit.is_some()
    }

    /// Stream the authoritative window and publish its chunk meshes, exactly as
    /// the sandbox does: bounded uploads per frame, background preparation when
    /// it is available and a synchronous fallback when it is not.
    /// Take completed chunk meshes and request new ones, exactly as the sandbox
    /// does: bounded per frame, background preparation when it is available and
    /// a synchronous mesh when it is not. No renderer call, so it runs before
    /// the frame's fence wait.
    fn prepare_chunks(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_ref() else {
            return Ok(());
        };
        let begin = Instant::now();
        self.chunk_keys = self.world.chunk_keys();
        self.tiles.chunk_keys_ms = begin.elapsed().as_secs_f64() * 1000.;
        let request_begin = Instant::now();
        if self.preparation.available() {
            while self.chunk_uploads.len() < MAX_CHUNK_UPLOADS {
                let Some((key, mesh)) = self.preparation.poll_mesh(&self.world) else {
                    break;
                };
                // A mesh the renderer already holds at this revision is not worth
                // an upload; a newer one replaces it.
                if renderer.chunk_revision(key) == Some(mesh.revision) {
                    continue;
                }
                self.chunk_uploads.push((key, mesh));
                if begin.elapsed().as_secs_f64() * 1000. >= MAX_CHUNK_MS {
                    break;
                }
            }
            // Nearest first: the eye's own column is what a viewer notices first.
            // The sort is part of the request half below, not the key listing.
            let eye = self.camera.position;
            self.chunk_keys.sort_by_key(|key| {
                let dx = key[0] * 16 + 8 - eye.x as i32;
                let dz = key[2] * 16 + 8 - eye.z as i32;
                dx * dx + dz * dz
            });
            let mut requested = 0;
            for key in &self.chunk_keys {
                if renderer.chunk_revision(*key) != self.world.chunk_revision(*key)
                    && self.preparation.request_mesh(&self.world, *key)
                {
                    requested += 1;
                    if requested >= MAX_CHUNK_REQUESTS {
                        break;
                    }
                }
            }
        } else {
            for key in &self.chunk_keys {
                if renderer.chunk_revision(*key) != self.world.chunk_revision(*key) {
                    self.chunk_uploads.push((*key, self.world.mesh_chunk(*key)));
                    if self.chunk_uploads.len() >= MAX_CHUNK_UPLOADS {
                        break;
                    }
                }
            }
        }
        self.tiles.chunk_poll_ms = request_begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.chunk_plan_ms += begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.chunk_plan_total_ms += self.tiles.chunk_plan_ms;
        self.tiles.chunk_keys_total_ms += self.tiles.chunk_keys_ms;
        self.tiles.chunk_poll_total_ms += self.tiles.chunk_poll_ms;
        Ok(())
    }

    /// Plan the rings for this eye, keep the cache's recency honest, request what
    /// the background worker does not have, take its completed meshes and select
    /// this frame's uploads. No renderer call: it runs before the fence wait.
    fn prepare_tiles(&mut self) {
        let begin = Instant::now();
        self.tile_cache.begin_frame();
        // Take completed meshes first, so the plan below sees them and a tile that
        // arrives this frame can still be uploaded this frame.
        for _ in 0..MAX_TILE_POLLS {
            let Some((id, mesh)) = self.tile_worker.poll() else {
                break;
            };
            self.tile_cache.insert(id, mesh);
        }
        let eye = self.camera.position.to_array();
        let fine = landscape::fine_clip(eye);
        landscape::ring_plan_into(eye, fine, &LANDSCAPE_RINGS, &mut self.plan);
        self.declared.clear();
        self.declared
            .extend(self.plan.iter().map(|tile| (tile.level, tile.key)));
        // The plan is the only thing that decides what is drawn: a tile it drops
        // is evicted this frame, in the render phase, so the drawing set stays
        // exactly the plan. The cache below keeps its mesh, which is what makes a
        // tile the eye returns to a re-upload rather than a regeneration.
        let work = plan_tile_work(&self.plan, &self.resident, TileBudget::default());
        for key in &work.evict {
            self.resident.remove(key);
        }
        self.tiles.evicted += work.evict.len() as u64;
        // This frame's selection, rebuilt from scratch: a tile left over from the
        // previous frame is not a decision this frame made.
        self.tile_uploads.clear();
        // Without a worker (thread startup failed) the sample generates what it
        // can on this thread instead of queueing work nobody runs. Bounded by the
        // same per-frame count and time budget as the uploads.
        let worker_available = self.tile_worker.available();
        let mut requests = 0;
        let mut uploads = 0;
        let mut uncached = 0;
        for tile in &work.build {
            let id = landscape_tiles::tile_id(tile);
            let mut cached = self.tile_cache.want(id);
            if cached {
                // Wanted again while still cached: exactly what the byte budget is
                // for, and what an eviction-only cache would pay for twice.
                self.tiles.reused += 1;
            } else if !worker_available {
                let filter = normalized_filter(tile);
                self.tile_cache.insert(
                    id,
                    landscape::lod_tile_mesh(SEED, tile.level, tile.key, filter),
                );
                cached = true;
                if begin.elapsed().as_secs_f64() * 1000. >= MAX_TILE_MS {
                    break;
                }
            } else if !self.tile_worker.claimed(id) && requests < MAX_TILE_UPLOADS {
                let job = TileJob::new(
                    landscape_tiles::generator_id(),
                    SEED,
                    tile,
                    normalized_filter(tile),
                );
                requests += usize::from(self.tile_worker.request(job));
            }
            // Select the upload now and copy it during the render phase: the
            // selection is CPU work, the copy waits on the frame fence.
            if self.resident.get(&landscape_tiles::renderer_key(tile))
                != Some(&normalized_filter(tile))
            {
                if cached && uploads < MAX_TILE_UPLOADS {
                    self.tile_uploads.push(*tile);
                    uploads += 1;
                } else if !cached {
                    uncached += 1;
                }
            }
        }
        self.tiles.outstanding = work.outstanding + uncached;
        self.tiles.plan_ms = begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.plan_total_ms += self.tiles.plan_ms;
    }

    /// Derive the water surface of every resident sea-level chunk whose column
    /// stack changed.
    ///
    /// Only the chunk layer that contains sea level can hold water, so a window
    /// change adds at most one water mesh per column stack. The derivation reads
    /// the authoritative world, which is CPU work, so it happens before the
    /// frame's fence wait and the upload happens after it.
    fn prepare_water(&mut self) {
        let begin = Instant::now();
        // Residency, not `chunk_keys`: an open-water column stores no voxels at
        // all, so its chunk is absent and a surface over it still has to be
        // derived. Residency is exactly the set of chunks the window published.
        self.water_keys = self
            .world
            .stream_resident_chunks()
            .unwrap_or_else(|| self.world.chunk_keys());
        let level = matterweave_core::water::sea_level_chunk_y();
        self.water_window = self.water_keys.len();
        self.water_candidates = self.water_keys.iter().filter(|key| key[1] == level).count();
        self.water_uploads.clear();
        for index in 0..self.water_keys.len() {
            let key = self.water_keys[index];
            if key[1] != level {
                continue;
            }
            // The stamp is the revision of every chunk in the column stack: an
            // edit invalidates the surface exactly when the derivation could read
            // it. A streaming publish re-stamps the columns at the window edge
            // and their neighbours for face coupling, which cannot change a water
            // surface, so a re-derivation there usually returns the mesh already
            // resident. Comparing costs a few hundred bytes of memory traffic and
            // saves a GPU buffer replacement, so the identical result refreshes
            // the stamp and is not uploaded again.
            if self.water_stack_changed(key) {
                let mesh = self.world.water_mesh_chunk(key);
                if self
                    .water_resident
                    .get(&key)
                    .is_some_and(|old| same_geometry(old, &mesh))
                {
                    self.tiles.water_unchanged += 1;
                } else {
                    self.water_uploads.push((key, mesh));
                }
            }
            if self.water_uploads.len() >= MAX_WATER_UPLOADS
                || begin.elapsed().as_secs_f64() * 1000. >= MAX_WATER_MS
            {
                break;
            }
        }
        self.tiles.water_plan_ms = begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.water_plan_total_ms += self.tiles.water_plan_ms;
    }

    /// Whether the column stack `key` was derived from still matches what the
    /// resident water mesh was built from, updating the recorded revisions.
    ///
    /// Called only for the sea-level layer, whose stack is every chunk layer the
    /// derivation can read.
    fn water_stack_changed(&mut self, key: [i32; 3]) -> bool {
        for (index, y) in self.water_stack.iter().enumerate() {
            self.water_stack_now[index] = self.world.chunk_revision([key[0], *y, key[2]]);
        }
        match self.water_stamps.get(&key) {
            Some(previous) if previous.as_slice() == self.water_stack_now.as_slice() => false,
            _ => {
                self.water_stamps.insert(key, self.water_stack_now.clone());
                true
            }
        }
    }

    /// The frame's single fence wait, then every retain and upload.
    ///
    /// One submission is in flight at a time, so the first call that touches the
    /// GPU waits for the previous frame to complete. Doing it here - after all
    /// generation, before any upload - both makes that wait visible in the
    /// capture's fence columns and lets everything above overlap the previous
    /// frame instead of serializing behind it.
    fn upload_geometry(&mut self) -> Result<(), String> {
        let begin = Instant::now();
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        renderer.wait_for_frame()?;
        self.tiles.fence_ms = begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.fence_total_ms += self.tiles.fence_ms;

        // Chunks: after the frame fence. A window change also drops the chunks
        // that left it, which is why the retain comes before the uploads.
        let chunk_begin = Instant::now();
        renderer.retain_chunks(&self.chunk_keys)?;
        let uploads = std::mem::take(&mut self.chunk_uploads);
        let uploaded = uploads.len();
        for (key, mesh) in &uploads {
            renderer.upload_chunk(*key, mesh)?;
        }
        self.tiles.chunk_upload_ms = chunk_begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.chunk_upload_total_ms += self.tiles.chunk_upload_ms;
        self.tiles.chunk_uploads += uploaded as u64;
        self.tiles.chunk_frame_uploads = u32::try_from(uploaded).unwrap_or(u32::MAX);

        // Tiles: declare the plan first, so the renderer drops what the plan no
        // longer wants before this frame adds anything.
        let declare_begin = Instant::now();
        renderer.retain_terrain_tiles(&self.declared)?;
        self.tiles.declare_ms = declare_begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.declare_total_ms += self.tiles.declare_ms;
        let tile_begin = Instant::now();
        let mut tile_uploads = 0u64;
        let mut skipped = 0usize;
        let mut copy_ms = 0.0;
        for (index, tile) in self.tile_uploads.iter().enumerate() {
            if tile_begin.elapsed().as_secs_f64() * 1000. >= MAX_TILE_MS {
                skipped = self.tile_uploads.len() - index;
                break;
            }
            let filter = normalized_filter(tile);
            let id = landscape_tiles::tile_id(tile);
            let Some(mesh) = self.tile_cache.get(id) else {
                // The entry can only disappear if the identity changed under us;
                // skip it rather than upload geometry for a different surface.
                skipped += 1;
                continue;
            };
            // A tile entirely inside the hole meshes to nothing. It stays a
            // planned, resident-as-empty tile so it is not rebuilt every frame.
            if !mesh.indices.is_empty() {
                let copy_begin = Instant::now();
                renderer.upload_terrain_tile(tile.level, tile.key, mesh)?;
                copy_ms += copy_begin.elapsed().as_secs_f64() * 1000.;
                tile_uploads += 1;
            }
            self.resident
                .insert(landscape_tiles::renderer_key(tile), filter);
        }
        self.tiles.uploaded += tile_uploads;
        self.tiles.tile_frame_uploads = u32::try_from(tile_uploads).unwrap_or(u32::MAX);
        self.tiles.outstanding += skipped;
        self.tiles.upload_ms = tile_begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.copy_ms = copy_ms;
        self.tiles.copy_total_ms += copy_ms;
        self.tiles.upload_total_ms += self.tiles.upload_ms;
        if self.tiles.upload_ms > self.tiles.worst.upload_ms {
            self.tiles.worst = WorstFrame {
                tiles: self.tile_uploads.len() - skipped,
                plan_ms: self.tiles.plan_ms,
                upload_ms: self.tiles.upload_ms,
                copy_ms,
            };
        }

        // Water: keep only the keys the window published, then upload the meshes
        // derived above. `retain_water_chunks` drops a mesh whose chunk left the
        // window, which is a fence wait of its own unless nothing was dropped.
        let water_begin = Instant::now();
        renderer.retain_water_chunks(&self.water_keys)?;
        let water_uploads = std::mem::take(&mut self.water_uploads);
        let uploaded = water_uploads.len();
        for (key, mesh) in &water_uploads {
            renderer.upload_water_chunk(*key, mesh)?;
        }
        for (key, mesh) in water_uploads {
            self.water_resident.insert(key, mesh);
        }
        // Forget stamps whose key left the window. Guarded, because the scan is
        // quadratic in the window and it only has work to do when a key the
        // window dropped is still stamped.
        if self.water_stamps.len() > self.water_candidates {
            let live: BTreeSet<[i32; 3]> = self
                .water_keys
                .iter()
                .copied()
                .filter(|key| key[1] == matterweave_core::water::sea_level_chunk_y())
                .collect();
            self.water_stamps.retain(|key, _| live.contains(key));
            self.water_resident.retain(|key, _| live.contains(key));
        }
        self.water_uploaded += uploaded as u64;
        self.tiles.water_upload_ms = water_begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.water_upload_total_ms += self.tiles.water_upload_ms;
        self.tiles.water_uploads += uploaded as u64;
        self.tiles.water_frame_uploads = u32::try_from(uploaded).unwrap_or(u32::MAX);
        Ok(())
    }

    /// Derive and upload the water surface of every resident sea-level chunk.
    ///
    /// Only the chunk layer that contains sea level can hold water, so a window
    /// change adds at most one water mesh per column stack. The derivation reads
    /// the authoritative world, so an edit that raises land above sea level
    /// removes its surface on the next sync, exactly like a chunk mesh.
    fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.95, 0.97, 0.99, 1.];
        let muted = [0.70, 0.79, 0.86, 1.];
        let accent = [0.62, 0.94, 0.76, 1.];
        let panel = [0.03, 0.06, 0.09, 0.80];
        hud.rect([16., 16., 700., 150.], panel);
        hud.text(30., 28., "MATTERWEAVE / LANDSCAPE RINGS", 2., white);
        let (chunks_visible, chunks_resident, tiles, water) = match &self.renderer {
            Some(renderer) => (
                renderer.visible_chunks,
                renderer.resident_chunks,
                renderer.terrain_tile_stats(),
                renderer.water_stats(),
            ),
            None => (0, 0, Default::default(), Default::default()),
        };
        hud.text(
            30.,
            54.,
            &format!(
                "CHUNKS {}/{} | TILES {}/{} VIS {} | WATER {}/{} | UP {} EV {} | LEFT {}",
                chunks_visible,
                chunks_resident,
                tiles.resident,
                tiles.declared,
                tiles.visible,
                water.resident,
                water.visible,
                self.tiles.uploaded,
                self.tiles.evicted,
                self.tiles.outstanding
            ),
            1.15,
            accent,
        );
        hud.text(
            30.,
            74.,
            &format!(
                "TILE PLAN {:.2} UP {:.2} | CHUNK {:.2} | FRAME {:.1} MS | TILE MEM {} KIB | {}",
                self.tiles.plan_ms,
                self.tiles.upload_ms,
                self.tiles.chunk_plan_ms + self.tiles.chunk_upload_ms,
                self.frame_ms,
                tiles.bytes / 1024,
                if self.walking { "WALK" } else { "FLY" }
            ),
            1.15,
            white,
        );
        hud.text(
            30.,
            94.,
            &format!(
                "EYE {:.0} {:.0} {:.0} | GROUND {} M | SEED {}",
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                landscape::height_at(
                    SEED,
                    self.camera.position.x as i32,
                    self.camera.position.z as i32
                ),
                SEED
            ),
            1.,
            muted,
        );
        let flora = self
            .flora
            .as_ref()
            .map(LandscapeFlora::counters)
            .unwrap_or_default();
        hud.text(
            30.,
            111.,
            &format!(
                "FLORA SITES {} TREES {} DROP {} | DRAWN {} BATCH {} | INST {} KIB | PLAN {:.2} + UP {:.2} MS",
                flora.planned_sites,
                flora.planned_trees,
                flora.dropped,
                flora.drawn,
                flora.batches,
                flora.instance_bytes / 1024,
                flora.plan_ms,
                flora.upload_ms,
            ),
            1.,
            accent,
        );
        hud.text(
            30.,
            129.,
            &self.status.chars().take(72).collect::<String>(),
            1.,
            muted,
        );
        let zone = self.controls.move_zone();
        hud.rect(zone, [0.04, 0.10, 0.13, 0.52]);
        hud.text(zone[0] + 18., zone[1] + 18., "MOVE", 1.5, white);
        hud.text(760., 350., "DRAG TO LOOK", 1.25, white);
        #[cfg(not(target_os = "android"))]
        hud.text(
            240.,
            586.,
            "WASD FLY | SPACE/SHIFT UP DOWN | RMB LOOK | G WALK OR FLY | ESC BACK",
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
        let capturing = self.capture.enabled();
        // Wall clock first, then the thread CPU clock: the busy interval stays
        // inside the wall interval. The two readings are adjacent, not
        // simultaneous.
        let cpu_busy = metrics::CpuBusySpan::begin(capturing);
        if let Some(renderer) = &mut self.renderer {
            renderer.begin_frame_diagnostics();
        }
        self.tiles.chunk_plan_ms = 0.;
        self.tiles.chunk_keys_ms = 0.;
        self.tiles.chunk_poll_ms = 0.;
        self.tiles.chunk_upload_ms = 0.;
        self.tiles.chunk_frame_uploads = 0;
        self.tiles.declare_ms = 0.;
        self.tiles.upload_ms = 0.;
        self.tiles.copy_ms = 0.;
        self.tiles.tile_frame_uploads = 0;
        self.tiles.water_plan_ms = 0.;
        self.tiles.water_upload_ms = 0.;
        self.tiles.water_frame_uploads = 0;
        let (motion, look) = self.controls.consume();
        fly(&mut self.camera, motion, look, dt, FLY_SPEED);
        if self.exercise {
            self.exercise_step();
        }
        if self.walking {
            // No physics in this slice: the eye follows the generator surface
            // directly, which is the same function the rings are derived from.
            let ground = landscape::height_at(
                SEED,
                self.camera.position.x.floor() as i32,
                self.camera.position.z.floor() as i32,
            ) as f32;
            self.camera.position.y = ground + 1.0 + EYE_HEIGHT;
        }
        let stream_begin = Instant::now();
        if self.preparation.available() {
            self.preparation
                .request_stream(&self.world, self.camera.position.to_array());
            // A published window replaces the world and advances its revision
            // without changing a column; nothing here depends on which of the
            // two happened, because water is stamped by column-stack revision.
            self.preparation.poll_stream(&mut self.world);
        } else {
            self.world.stream_around(self.camera.position.to_array());
        }
        let stream_ms = stream_begin.elapsed().as_secs_f64() * 1000.;
        // -- CPU phase: generation only, overlapping the previous submission. --
        let mesh_begin = Instant::now();
        self.prepare_water();
        self.prepare_tiles();
        if let Err(error) = self.prepare_chunks() {
            log::error!("Landscape geometry failed: {error}");
            eprintln!("Landscape geometry failed: {error}");
            self.failed = true;
            event_loop.exit();
            return;
        }
        // -- GPU phase: one explicit fence wait, then every upload. --
        if let Err(error) = self.upload_geometry() {
            log::error!("Landscape geometry failed: {error}");
            eprintln!("Landscape geometry failed: {error}");
            self.failed = true;
            event_loop.exit();
            return;
        }
        let mesh_ms = mesh_begin.elapsed().as_secs_f64() * 1000.;
        // Flora is rebuilt only when the eye leaves its rebuild cell, which is
        // what keeps a moving camera from re-running the planner every frame.
        let eye = self.camera.position.to_array();
        if let Some(flora) = self.flora.as_mut() {
            let renderer = self.renderer.as_mut().unwrap();
            if let Err(error) = flora.sync(renderer, eye) {
                log::error!("Landscape flora failed: {error}");
                eprintln!("Landscape flora failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        }
        // The wind clock advances every frame even when the field does not, so
        // the resident plants keep moving. Player push follows the eye.
        self.wind_time += dt.clamp(0.0, 0.1);
        self.lighting.wind = Wind {
            direction_xz: WIND_DIRECTION_XZ,
            strength_m: WIND_STRENGTH_M,
            time_s: self.wind_time,
        };
        self.lighting.player = PlayerPush {
            position: eye,
            radius_m: PUSH_RADIUS_M,
        };
        let hud = self.hud();
        let matrix = view_projection(&self.camera, size.width as f32 / size.height as f32);
        let render_begin = Instant::now();
        let outcome =
            self.renderer
                .as_mut()
                .unwrap()
                .render_with_lighting(matrix, eye, &hud, &self.lighting);
        let render_ms = render_begin.elapsed().as_secs_f64() * 1000.;
        match &outcome {
            FrameResult::Presented => {
                self.frames += 1;
                if self.frames.is_multiple_of(30) {
                    let tiles = self.renderer.as_ref().unwrap().terrain_tile_stats();
                    let water = self.renderer.as_ref().unwrap().water_stats();
                    let worker = self.tile_worker.stats();
                    let cache = self.tile_cache.stats();
                    log::info!(
                        "LANDSCAPE frame {} frame_ms {:.2} fence_ms {:.2} \
                         chunk_plan_ms {:.2} chunk_upload_ms {:.2} chunk_up {} \
                         tile_plan_ms {:.2} tile_declare_ms {:.2} tile_upload_ms {:.2} tile_copy_ms {:.2} \
                         water_plan_ms {:.2} water_upload_ms {:.2} water {}/{} up {} \
                         tiles {}/{} visible {} up {} ev {} reused {} left {} tile_kib {} \
                         tile_worker q {} r {} {} KiB gen {} {:.1} ms in bounds {} \
                         tile_cache {} {}/{} KiB hit {} miss {} evict {} \
                         eye {:?}",
                        self.frames,
                        self.frame_ms,
                        self.tiles.fence_ms,
                        self.tiles.chunk_plan_ms,
                        self.tiles.chunk_upload_ms,
                        self.tiles.chunk_frame_uploads,
                        self.tiles.plan_ms,
                        self.tiles.declare_ms,
                        self.tiles.upload_ms,
                        self.tiles.copy_ms,
                        self.tiles.water_plan_ms,
                        self.tiles.water_upload_ms,
                        water.resident,
                        water.visible,
                        self.tiles.water_frame_uploads,
                        tiles.resident,
                        tiles.declared,
                        tiles.visible,
                        self.tiles.uploaded,
                        self.tiles.evicted,
                        self.tiles.reused,
                        self.tiles.outstanding,
                        tiles.bytes / 1024,
                        worker.pending,
                        worker.results,
                        worker.result_bytes / 1024,
                        worker.generated,
                        worker.generate_ms,
                        worker.within_bounds(),
                        cache.entries,
                        cache.bytes / 1024,
                        cache.budget_bytes / 1024,
                        cache.hits,
                        cache.misses,
                        cache.evictions,
                        eye
                    );
                }
            }
            FrameResult::Retry => {}
            FrameResult::OutOfMemory => {
                log::error!("Landscape GPU out of memory");
                eprintln!("Landscape GPU out of memory");
                self.failed = true;
                event_loop.exit();
                return;
            }
            FrameResult::Fatal(error) => {
                log::error!("Landscape render failed: {error}");
                eprintln!("Landscape render failed: {error}");
                self.failed = true;
                self.renderer = None;
                event_loop.exit();
                return;
            }
        }
        if capturing {
            self.record(&outcome, dt, stream_ms, mesh_ms, render_ms, now, cpu_busy);
        }
        if !self.failed && self.frame_limit.is_some_and(|limit| self.frames >= limit) {
            let tiles = self.renderer.as_ref().unwrap().terrain_tile_stats();
            let flora = self
                .flora
                .as_ref()
                .map(LandscapeFlora::counters)
                .unwrap_or_default();
            let water = self.renderer.as_ref().unwrap().water_stats();
            let worker = self.tile_worker.stats();
            let cache = self.tile_cache.stats();
            let frames = self.frames.max(1) as f64;
            eprintln!(
                "LANDSCAPE SMOKE PASS: {} presented frames; {}",
                self.frames,
                self.renderer.as_ref().unwrap().capabilities
            );
            // Per-phase means over the run: the plan timings are CPU work that
            // overlaps the previous submission, the upload timings are copies
            // after the frame fence, and the fence wait is the frame's own
            // serialization point. The schema has one span column, so the split
            // lives here.
            eprintln!(
                "LANDSCAPE MESH SYNC (mean per frame, ms): fence {:.2} + chunk list {:.2} + chunk \
                 meshes {:.2} + tile plan {:.2} + water plan {:.2} + tile declare {:.2} + chunk \
                 upload {:.2} + tile upload {:.2} (copy {:.2}) + water upload {:.2}; budget {} tiles \
                 / {:.1} ms",
                self.tiles.fence_total_ms / frames,
                self.tiles.chunk_keys_total_ms / frames,
                self.tiles.chunk_poll_total_ms / frames,
                self.tiles.plan_total_ms / frames,
                self.tiles.water_plan_total_ms / frames,
                self.tiles.declare_total_ms / frames,
                self.tiles.chunk_upload_total_ms / frames,
                self.tiles.upload_total_ms / frames,
                self.tiles.copy_total_ms / frames,
                self.tiles.water_upload_total_ms / frames,
                MAX_TILE_UPLOADS,
                MAX_TILE_MS
            );
            eprintln!(
                "LANDSCAPE COUNTERS: chunks {}/{} | tiles resident {} declared {} visible {} \
                 | uploaded {} evicted {} reused {} outstanding {} | tile mem {} KiB",
                self.renderer.as_ref().unwrap().visible_chunks,
                self.renderer.as_ref().unwrap().resident_chunks,
                tiles.resident,
                tiles.declared,
                tiles.visible,
                self.tiles.uploaded,
                self.tiles.evicted,
                self.tiles.reused,
                self.tiles.outstanding,
                tiles.bytes / 1024,
            );
            eprintln!(
                "LANDSCAPE WATER: resident {} visible {} uploaded {} | {} KiB | plan {:.2} ms \
                 upload {:.2} ms this frame | window {} candidates {}",
                water.resident,
                water.visible,
                self.water_uploaded,
                water.bytes / 1024,
                self.tiles.water_plan_ms,
                self.tiles.water_upload_ms,
                self.water_window,
                self.water_candidates,
            );
            eprintln!("LANDSCAPE FRAME: {:.1} ms", self.frame_ms);
            eprintln!(
                "LANDSCAPE WORST TILE FRAME: {} tiles in {:.2} ms = {:.2} ms planning + {:.2} ms \
                 uploading (of which {:.2} ms inside the upload calls); budget {} tiles / {:.1} ms",
                self.tiles.worst.tiles,
                self.tiles.worst.plan_ms + self.tiles.worst.upload_ms,
                self.tiles.worst.plan_ms,
                self.tiles.worst.upload_ms,
                self.tiles.worst.copy_ms,
                MAX_TILE_UPLOADS,
                MAX_TILE_MS,
            );
            eprintln!(
                "LANDSCAPE TILE WORK: {} meshes generated by the background worker in {:.1} ms \
                 total = {:.2} ms each | worker queue {} pending {} results {} KiB",
                worker.generated,
                worker.generate_ms,
                worker.generate_ms / worker.generated.max(1) as f64,
                worker.pending,
                worker.results,
                worker.result_bytes / 1024,
            );
            eprintln!(
                "LANDSCAPE TILE CACHE: {} entries {}/{} KiB | hit {} miss {} evict {}",
                cache.entries,
                cache.bytes / 1024,
                cache.budget_bytes / 1024,
                cache.hits,
                cache.misses,
                cache.evictions,
            );
            eprintln!(
                "LANDSCAPE FLORA: sites {} trees {} dropped {} | drawn {} batches {} | instance \
                 bytes {} | plan {:.2} ms (worst {:.2}, budget {:.1}, over {}) upload {:.2} ms \
                 | rebuilds {}",
                flora.planned_sites,
                flora.planned_trees,
                flora.dropped,
                flora.drawn,
                flora.batches,
                flora.instance_bytes,
                flora.plan_ms,
                flora.worst_plan_ms,
                crate::landscape_flora::MAX_REBUILD_MS,
                flora.over_budget,
                flora.upload_ms,
                flora.rebuilds,
            );
            event_loop.exit();
        }
    }

    /// One capture row. Only fields this sample actually measures are filled;
    /// systems it does not run (physics, dynamic meshes, saves) stay empty rather
    /// than borrowing another sample's number, and `chunk_mesh_uploads` counts
    /// chunk meshes rather than the tile meshes a previous version put there.
    ///
    /// The capture wrapper owns attempt, epoch, presented and completion
    /// identity; this function supplies the per-frame measurements.
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        result: &FrameResult,
        dt: f32,
        stream_ms: f64,
        mesh_ms: f64,
        render_ms: f64,
        frame_begin: Instant,
        cpu_busy: metrics::CpuBusySpan,
    ) {
        let row = metrics::FrameRow {
            draw_interval_wall_ms: Some(f64::from(dt) * 1000.),
            render_wall_ms: Some(render_ms),
            stream_request_elapsed_ms: Some(stream_ms),
            mesh_sync_wall_ms: Some(mesh_ms),
            chunk_mesh_uploads: Some(self.tiles.chunk_frame_uploads),
            ..Default::default()
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        self.capture
            .record(renderer, result, row, frame_begin, cpu_busy);
    }

    fn point(&self, x: f64, y: f64) -> Vec2 {
        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(winit::dpi::PhysicalSize::new(1000, 600));
        Vec2::new(
            x as f32 / size.width.max(1) as f32 * 1000.,
            y as f32 / size.height.max(1) as f32 * 600.,
        )
    }

    fn toggle_walk(&mut self) {
        self.walking = !self.walking;
        self.status = if self.walking {
            "Walking the generator surface"
        } else {
            "Flying"
        }
        .into();
    }
}

impl ApplicationHandler for LandscapeSample {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        log::info!("Landscape lifecycle resumed");
        self.controls.clear();
        self.last_frame = Instant::now();
        self.focused = true;
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Matterweave | Landscape")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 768)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("Landscape window creation failed: {error}");
                eprintln!("Landscape window creation failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(mut renderer) => {
                log::info!("Landscape graphics: {}", renderer.capabilities);
                eprintln!("Landscape graphics: {}", renderer.capabilities);
                // One renderer, one epoch: the capture's epoch advances here so a
                // recreated renderer's restarted submission ids stay distinct.
                self.capture.renderer_created(&mut renderer);
                self.renderer = Some(renderer);
                self.window = Some(window);
                // Prototype pooling is CPU work that does not need the
                // renderer, and it is built once per process: a resume after a
                // suspend keeps the pool and only re-installs the field.
                if self.flora.is_none() {
                    match LandscapeFlora::new() {
                        Ok(flora) => self.flora = Some(flora),
                        Err(error) => {
                            log::error!("Landscape flora build failed: {error}");
                            eprintln!("Landscape flora build failed: {error}");
                            self.failed = true;
                            event_loop.exit();
                        }
                    }
                }
            }
            Err(error) => {
                log::error!("Landscape renderer initialization failed: {error}");
                eprintln!("Landscape renderer initialization failed: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        log::info!("Landscape lifecycle suspended");
        // GPU objects must be gone before the Android suspend callback returns.
        // Every resident tile dies with the renderer, so the cache bookkeeping
        // is cleared too and the next resume rebuilds inside the same budget.
        // The flora field's buffers die the same way; the plan and pooled
        // geometry stay, so the field is marked uninstalled and re-uploaded on
        // the next frame rather than uploading into an absent scene.
        if let Some(flora) = self.flora.as_mut() {
            flora.forget_renderer();
        }
        self.renderer = None;
        self.window = None;
        // Every resident tile and water mesh dies with the renderer. The CPU
        // tile cache survives, because its meshes are pure functions of the
        // generator identity and nothing about them belongs to the renderer; the
        // water stamps do not, because they describe buffers the renderer still
        // holds. Clearing them re-derives and re-uploads the surfaces on resume.
        self.resident.clear();
        self.water_stamps.clear();
        self.water_resident.clear();
        self.controls.clear();
        self.focused = false;
        self.capture.flush();
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
                if let PhysicalKey::Code(code) = event.physical_key {
                    if pressed && !event.repeat {
                        match code {
                            KeyCode::Escape => self.return_to_menu = true,
                            KeyCode::KeyG => self.toggle_walk(),
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
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Right {
                    self.controls.mouse_look = state == ElementState::Pressed;
                }
            }
            WindowEvent::Touch(touch) => {
                let point = self.point(touch.location.x, touch.location.y);
                match touch.phase {
                    TouchPhase::Started => {
                        self.controls.start(touch.id, point);
                    }
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
        self.capture.flush();
        self.renderer = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_core::Vertex;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "matterweave-landscape-{}-{}-{name}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn plan_at(eye: [f32; 3]) -> Vec<RingTile> {
        landscape::ring_plan(eye, landscape::fine_clip(eye), &LANDSCAPE_RINGS)
    }

    fn resident_from(plan: &[RingTile]) -> BTreeMap<TerrainTileKey, TileFilter> {
        plan.iter()
            .map(|tile| ((tile.level, tile.key), normalized_filter(tile)))
            .collect()
    }

    #[test]
    fn a_missing_or_broken_marker_is_not_an_opt_in() {
        let dir = temp_dir("marker");
        assert!(!marker_present(&dir));
        std::fs::write(dir.join(MARKER_FILE), "").unwrap();
        assert!(marker_present(&dir), "an empty marker is still an opt-in");
        // Content is reserved and ignored rather than parsed or rejected.
        std::fs::write(dir.join(MARKER_FILE), "anything at all\n").unwrap();
        assert!(marker_present(&dir));
        // An oversized marker is ignored, not truncated and not fatal.
        std::fs::write(
            dir.join(MARKER_FILE),
            "x".repeat(MAX_MARKER_BYTES as usize + 1),
        )
        .unwrap();
        assert!(!marker_present(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_exercise_marker_is_a_separate_opt_in_with_an_off_switch() {
        let dir = temp_dir("exercise-marker");
        assert!(
            !exercise_marker_present(&dir),
            "a plain landscape run must not fly itself"
        );
        // The two markers are independent: selecting the sample does not
        // select the scripted path.
        std::fs::write(dir.join(MARKER_FILE), "").unwrap();
        assert!(!exercise_marker_present(&dir));
        std::fs::write(dir.join(EXERCISE_MARKER_FILE), "").unwrap();
        assert!(exercise_marker_present(&dir), "an empty marker selects it");
        std::fs::write(dir.join(EXERCISE_MARKER_FILE), "on\n").unwrap();
        assert!(exercise_marker_present(&dir));
        // `off` is the documented way to keep the file but stop the flight, so
        // a captured run can be replayed without deleting the marker.
        std::fs::write(dir.join(EXERCISE_MARKER_FILE), "OFF\n").unwrap();
        assert!(!exercise_marker_present(&dir));
        std::fs::write(
            dir.join(EXERCISE_MARKER_FILE),
            "x".repeat(MAX_MARKER_BYTES as usize + 1),
        )
        .unwrap();
        assert!(!exercise_marker_present(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn construction_records_a_capture_and_writes_no_world() {
        let dir = temp_dir("capture");
        std::fs::write(dir.join("profile-frames.txt"), "4").unwrap();
        let save = dir.join("world.json");
        let sample = LandscapeSample::new(save.clone(), Some(1));
        // The capture request is consumed at construction, before the first
        // frame, so a marker-selected device run records from frame one.
        assert!(
            sample.capture.enabled(),
            "the capture request was not taken"
        );
        assert!(!dir.join("profile-frames.txt").exists());
        // A viewer must not fabricate a save.
        assert!(!save.exists(), "the landscape sample wrote a world save");
        drop(sample);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Mirrors what `upload_geometry` does with a water upload, so a test can
    /// drive the derivation pipeline without a renderer.
    fn take_water_uploads(sample: &mut LandscapeSample) -> Vec<[i32; 3]> {
        let uploads = std::mem::take(&mut sample.water_uploads);
        let keys = uploads.iter().map(|(key, _)| *key).collect();
        for (key, mesh) in uploads {
            sample.water_resident.insert(key, mesh);
        }
        keys
    }

    /// Deep copy of a derived mesh, for comparing geometry across a change.
    fn mesh_copy(mesh: &Mesh) -> Mesh {
        Mesh {
            vertices: mesh.vertices.clone(),
            indices: mesh.indices.clone(),
            revision: mesh.revision,
        }
    }

    /// Runs frames until the water phase has no work left for the current window,
    /// returning the keys whose mesh actually had to be uploaded. A call that
    /// changes nothing and uploads nothing repeats itself exactly, so the loop
    /// stops there; the bound is the candidates plus the frames the real budget
    /// needs to visit them.
    fn drain_water(sample: &mut LandscapeSample) -> Vec<[i32; 3]> {
        let mut uploaded = Vec::new();
        for _ in 0..(sample.water_candidates + 2).max(4) + 8 {
            let unchanged = sample.tiles.water_unchanged;
            sample.prepare_water();
            let pushed = sample.water_uploads.len() as u64;
            uploaded.extend(take_water_uploads(sample));
            if pushed == 0 && sample.tiles.water_unchanged == unchanged {
                break;
            }
        }
        uploaded
    }

    /// A window publish advances the world revision and re-stamps the chunk
    /// revisions of the columns that entered or left plus their neighbours, for
    /// face coupling that cannot change a water surface. Keying water on the world
    /// revision re-derived every candidate on every chunk step; keying on the
    /// column stack, then comparing the derived geometry, re-derives only that
    /// handful and uploads only what actually differs.
    #[test]
    fn water_is_re_derived_for_the_window_edge_but_uploaded_only_when_it_changed() {
        let dir = temp_dir("water-stamps");
        let mut sample = LandscapeSample::new(dir.join("world.json"), None);
        let level = matterweave_core::water::sea_level_chunk_y();
        let first = drain_water(&mut sample);
        assert!(!first.is_empty(), "the first frame derived nothing");
        let flooded = sample
            .water_resident
            .iter()
            .any(|(key, mesh)| key[1] == level && !mesh.indices.is_empty());
        assert!(
            flooded,
            "the shore spawn should hold flooded sea-level chunks"
        );
        let candidates = sample.water_candidates;
        let quiet = sample.tiles.water_unchanged;

        // A publish moves the window by one chunk column: nothing to upload for
        // the columns whose combined stack revision did not move.
        let eye = sample.camera.position;
        sample
            .world
            .stream_around([eye.x + 16.0, eye.y, eye.z]);
        let rederived = drain_water(&mut sample);
        assert!(
            rederived.len() < candidates,
            "a publish re-derived every candidate: {rederived:?}"
        );
        assert!(
            sample.tiles.water_unchanged > quiet,
            "the columns re-stamped for face coupling were re-uploaded"
        );
        assert!(
            !rederived.is_empty(),
            "the columns that entered the window were never uploaded"
        );

        // An edit changes the column it is in and nothing else. Raising the
        // surface to sea level removes that column's water, so the mesh differs
        // and is uploaded.
        // A flooded column that is still in the window (the test keeps the
        // resident copies a real frame would have pruned).
        let edited = sample
            .water_resident
            .iter()
            .find(|(key, mesh)| {
                key[1] == level && !mesh.indices.is_empty() && sample.water_keys.contains(key)
            })
            .map(|(key, _)| *key)
            .expect("a flooded column in the window");
        let before = mesh_copy(
            sample
                .water_resident
                .get(&edited)
                .expect("a resident water mesh"),
        );
        assert!(!before.indices.is_empty(), "the column should be flooded");
        assert!(
            sample.world.set([edited[0] * 16 + 8, level, edited[2] * 16 + 8], 3),
            "the edit of {edited:?} was refused"
        );
        let rederived = drain_water(&mut sample);
        assert_eq!(
            rederived,
            vec![edited],
            "an edit must upload its own column and no other"
        );
        assert!(
            sample
                .water_resident
                .get(&edited)
                .is_some_and(|mesh| !same_geometry(&before, mesh)),
            "the edited column's surface must change"
        );
        drop(sample);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mesh_equality_is_exact_and_not_shared_identity() {
        let vertex = Vertex {
            position: [1., 2., 3.],
            normal: [0., 1., 0.],
            color: [0.5; 3],
        };
        let a = Mesh {
            vertices: vec![vertex],
            indices: vec![0, 1, 2],
            revision: 7,
        };
        let mut b = mesh_copy(&a);
        assert!(same_geometry(&a, &b), "equal meshes must compare equal");
        b.vertices[0].position[0] = 1.000_001;
        assert!(!same_geometry(&a, &b), "a moved vertex is a different mesh");
        b.vertices[0].normal[1] = 0.5;
        assert!(!same_geometry(&a, &b), "a different normal is different");
        b = mesh_copy(&a);
        b.indices.push(3);
        assert!(!same_geometry(&a, &b), "a longer index list is different");
        b = mesh_copy(&a);
        b.revision += 1;
        assert!(
            same_geometry(&a, &b),
            "a revision that moved with no geometry change is the same surface"
        );
        b.vertices.clear();
        assert!(!same_geometry(&a, &b), "a shorter vertex list is different");
    }

    /// Tile meshes are generated on the worker, not in the frame, and a tile that
    /// is already cached is selected for upload instead of being regenerated.
    #[test]
    fn tiles_are_generated_off_the_frame_and_served_from_the_cache() {
        let dir = temp_dir("tile-worker");
        let mut sample = LandscapeSample::new(dir.join("world.json"), None);
        sample.prepare_tiles();
        assert!(
            sample.tile_uploads.is_empty(),
            "nothing can be uploaded before the worker has generated it"
        );
        let queued = sample.tile_worker.stats();
        assert!(
            queued.pending + queued.inflight + queued.results > 0,
            "the frame requested no generation: {queued:?}"
        );
        // Wait the way a frame does: bounded polls, never a busy spin. A frame
        // moves the worker's completed meshes into the cache itself.
        let mut waited = 0;
        while sample.tile_cache.stats().entries == 0 && waited < 5_000 {
            sample.prepare_tiles();
            if sample.tile_cache.stats().entries > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
            waited += 1;
        }
        assert!(
            sample.tile_cache.stats().entries > 0,
            "the worker never produced a mesh"
        );
        assert!(sample.tile_worker.available());
        // The next frame over the same eye uploads what the worker finished and
        // regenerates none of it.
        sample.prepare_tiles();
        assert!(!sample.tile_uploads.is_empty(), "no tile was selected");
        assert!(
            sample.tiles.reused > 0,
            "no cached tile was reported as reused"
        );
        for tile in &sample.tile_uploads {
            let id = crate::landscape_tiles::tile_id(tile);
            assert!(
                sample.tile_cache.get(id).is_some(),
                "a selected tile was not cached"
            );
        }
        assert!(sample.tile_worker.stats().within_bounds());
        drop(sample);
        std::fs::remove_dir_all(&dir).ok();
    }


    #[test]
    fn normalizing_a_filter_cannot_change_the_mesh() {
        let plan = plan_at([137.5, 60.0, -201.25]);
        // Tiles from every ring, including ones the hole and the bound cut.
        for tile in plan.iter().step_by(11) {
            let original = landscape::lod_tile_mesh(SEED, tile.level, tile.key, tile.filter);
            let normalized =
                landscape::lod_tile_mesh(SEED, tile.level, tile.key, normalized_filter(tile));
            assert_eq!(
                original.vertices.len(),
                normalized.vertices.len(),
                "tile {:?} changed shape when its filter was normalized",
                (tile.level, tile.key)
            );
            assert_eq!(original.indices, normalized.indices);
            for (a, b) in original.vertices.iter().zip(&normalized.vertices) {
                assert_eq!(a.position, b.position);
            }
        }
    }

    #[test]
    fn normalizing_drops_filters_that_cannot_bite() {
        let plan = plan_at([0.0, 40.0, 0.0]);
        let outer = plan.last().unwrap();
        // A corner tile of the coarsest ring is inside its own ring square and
        // nowhere near the streaming window, so neither part of the filter can
        // remove one of its cells.
        assert_eq!(normalized_filter(outer), TileFilter::default());
        // The innermost ring has tiles the streaming window cuts into; those
        // must keep a hole.
        assert!(
            plan.iter()
                .any(|tile| normalized_filter(tile).hole.is_some()),
            "the streaming window must cut at least one planned tile"
        );
    }

    #[test]
    fn a_full_plan_is_built_over_several_frames_within_the_budget() {
        let plan = plan_at([0.0, 40.0, 0.0]);
        let mut resident = BTreeMap::new();
        let budget = TileBudget::default();
        let mut frames = 0;
        loop {
            let work = plan_tile_work(&plan, &resident, budget);
            if work.build.is_empty() {
                assert_eq!(work.outstanding, 0, "nothing buildable but work remaining");
                break;
            }
            assert!(work.build.len() <= budget.uploads);
            assert!(work.evict.is_empty(), "a stable plan evicts nothing");
            for tile in &work.build {
                resident.insert((tile.level, tile.key), normalized_filter(tile));
            }
            frames += 1;
            assert!(frames < 1000, "tile work never converged");
        }
        assert_eq!(resident.len(), plan.len());
        assert_eq!(frames, plan.len().div_ceil(budget.uploads));
        // A fully resident plan asks for nothing at all.
        assert_eq!(
            plan_tile_work(&plan, &resident, budget),
            TileWork::default()
        );
    }

    #[test]
    fn moving_evicts_dropped_tiles_and_rebuilds_only_what_changed() {
        let here = plan_at([0.0, 40.0, 0.0]);
        let resident = resident_from(&here);
        // One chunk of movement keeps every tile key, but moves the streaming
        // window, so only the tiles the window cuts may be rebuilt.
        let nudged = plan_at([16.0, 40.0, 0.0]);
        assert_eq!(
            nudged.iter().map(|t| (t.level, t.key)).collect::<Vec<_>>(),
            here.iter().map(|t| (t.level, t.key)).collect::<Vec<_>>()
        );
        let work = plan_tile_work(&nudged, &resident, TileBudget::default());
        assert!(work.evict.is_empty(), "the tile set did not change");
        let rebuilt = work.build.len() + work.outstanding;
        assert!(
            (1..=16).contains(&rebuilt),
            "one chunk of movement rebuilt {rebuilt} tiles"
        );
        assert!(work
            .build
            .iter()
            .all(|tile| tile.level == LANDSCAPE_RINGS[0].level));

        // A long jump drops most tiles from the plan. Everything the new plan
        // does not want goes in the same frame, with no budget: an off-screen
        // tile held back would let the cache grow along the camera's path.
        let far = plan_at([4096.0, 40.0, 4096.0]);
        let wanted: BTreeSet<TerrainTileKey> =
            far.iter().map(|tile| (tile.level, tile.key)).collect();
        let work = plan_tile_work(&far, &resident, TileBudget::default());
        let expected: Vec<TerrainTileKey> = resident
            .keys()
            .filter(|key| !wanted.contains(*key))
            .copied()
            .collect();
        assert_eq!(work.evict, expected);
        assert!(
            work.evict.len() > resident.len() / 2,
            "a jump of two rings should drop most of the cache"
        );
        assert_eq!(work.build.len(), MAX_TILE_UPLOADS);
        // Everything the new plan needs is either built now or still owed.
        let needed = far
            .iter()
            .filter(|tile| resident.get(&(tile.level, tile.key)) != Some(&normalized_filter(tile)))
            .count();
        assert_eq!(work.build.len() + work.outstanding, needed);
    }

    #[test]
    fn the_residency_bound_refuses_growth_instead_of_exceeding_it() {
        let plan = plan_at([0.0, 40.0, 0.0]);
        let budget = TileBudget {
            uploads: MAX_TILE_UPLOADS,
            resident: 5,
        };
        let work = plan_tile_work(&plan, &BTreeMap::new(), budget);
        assert_eq!(work.build.len(), 5);
        assert_eq!(work.build.len() + work.outstanding, plan.len());
        // Rebuilding a resident tile does not grow the cache, so it is allowed
        // even with the bound already reached.
        let resident = resident_from(&plan);
        let moved = plan_at([16.0, 40.0, 0.0]);
        let work = plan_tile_work(
            &moved,
            &resident,
            TileBudget {
                uploads: 8,
                resident: 1,
            },
        );
        assert!(
            !work.build.is_empty(),
            "a filter change on a resident tile must still be rebuilt"
        );
    }

    #[test]
    fn the_camera_stays_inside_the_simulation_domain() {
        let mut camera = spawn_camera();
        for _ in 0..200 {
            fly(
                &mut camera,
                Vec3::new(1., 1., 1.),
                Vec2::ZERO,
                0.05,
                FLY_SPEED,
            );
        }
        assert!(camera.position.x <= MAX_EYE_XZ && camera.position.z <= MAX_EYE_XZ);
        assert!(camera.position.y <= MAX_EYE_Y);
        for _ in 0..400 {
            fly(
                &mut camera,
                Vec3::new(-1., -1., -1.),
                Vec2::ZERO,
                0.05,
                FLY_SPEED,
            );
        }
        assert!(camera.position.x >= -MAX_EYE_XZ && camera.position.z >= -MAX_EYE_XZ);
        assert!(camera.position.y >= MIN_EYE_Y);
        // The clamp is what keeps the planner's coverage contract valid.
        let fine = landscape::fine_clip(camera.position.to_array());
        let plan = landscape::ring_plan(camera.position.to_array(), fine, &LANDSCAPE_RINGS);
        let inner = plan[0].filter.bound.unwrap();
        assert!(inner.min[0] <= fine.min[0] && inner.max[0] >= fine.max[0]);
        assert!(inner.min[1] <= fine.min[1] && inner.max[1] >= fine.max[1]);
    }
}
