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
//! [`MAX_TILE_MS`] of main-thread time has gone into tile work, whichever comes
//! first. Residency is bounded by [`matterweave_render::MAX_TERRAIN_TILES`] and
//! every tile the plan drops is evicted the same frame, so the cache cannot grow
//! with the path the camera took.
use crate::controls::{Camera, Controls};
use crate::metrics;
use glam::{Mat4, Vec2, Vec3};
use matterweave_core::landscape::{
    self, Clip, RingTile, TileFilter, LANDSCAPE_RINGS, LOD_TILE_CELLS,
};
use matterweave_core::{AsyncWorld, World};
use matterweave_render::{
    Atmosphere, FrameResult, Hud, LightingSettings, Renderer, Sun, TerrainTileKey,
    MAX_TERRAIN_TILES,
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

/// Tile meshes built and uploaded in one frame.
pub const MAX_TILE_UPLOADS: usize = 8;
/// Main-thread milliseconds one frame may spend planning, building and
/// uploading tiles. Checked after each tile, so one tile can overrun it; the
/// next cannot start.
pub const MAX_TILE_MS: f64 = 2.0;

/// Derived water meshes built and uploaded in one frame. Only the sea-level
/// chunk layer carries water, so this is at most one mesh per column stack.
pub const MAX_WATER_UPLOADS: usize = 8;
/// Main-thread milliseconds one frame may spend deriving and uploading water.
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
/// is off-screen and holding it would let the cache grow along the camera's
/// path.
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

/// Open on water looking at a coast: the first 64 m grid point (scanned from the
/// origin outward, so the same build always opens in the same place) that is open
/// water with land at least 20 m high about 320 m away. The camera then sits a
/// few metres above the sea with the shore and the mountains beyond it: the view
/// the near water pass, the distance rings and the haze are for.
fn spawn_camera() -> Camera {
    for gz in (-24..24).rev() {
        for gx in -24..24 {
            let (x, z) = (gx * 64, gz * 64);
            let depth = landscape::height_at(SEED, x, z);
            if depth > -8 {
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
                if landscape::height_at(SEED, x + dx * 320, z + dz * 320) > 20 {
                    return Camera {
                        position: Vec3::new(x as f32, 6., z as f32),
                        yaw: (dx as f32).atan2(dz as f32),
                        pitch: -0.04,
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

/// Counters the HUD shows and the smoke line prints. Every one of them is a
/// count this sample actually performed, not a target.
#[derive(Clone, Copy, Debug, Default)]
struct TileCounters {
    uploaded: u64,
    evicted: u64,
    outstanding: usize,
    /// Main-thread time spent building and uploading tiles. This is the
    /// quantity [`MAX_TILE_MS`] bounds.
    build_ms: f64,
    /// The part of `build_ms` spent generating tile meshes from the generator.
    /// This is the sample's own cost.
    generate_ms: f64,
    /// The part of `build_ms` spent inside `upload_terrain_tile`, which waits
    /// on the frame fence before it replaces a buffer. On a software rasterizer
    /// that wait is most of a frame, so it is reported rather than blamed on
    /// tile generation.
    upload_ms: f64,
    /// Main-thread time spent planning and declaring residency. Declaring can
    /// block on the frame fence when a tile has to be evicted, which is a GPU
    /// wait rather than tile work, so it is reported apart from the budget
    /// instead of being hidden inside it.
    declare_ms: f64,
    /// Total main-thread time spent generating tile meshes over the run, and
    /// the number of meshes that time covers. Their ratio is the only per-tile
    /// cost this sample can honestly report.
    total_generate_ms: f64,
    generated: u64,
    /// The single worst frame seen so far, chosen by build time and kept as one
    /// record. Independent maxima taken from different frames would describe a
    /// frame that never happened.
    worst: WorstFrame,
}

/// The most expensive tile frame of a run.
#[derive(Clone, Copy, Debug, Default)]
struct WorstFrame {
    tiles: usize,
    build_ms: f64,
    generate_ms: f64,
    upload_ms: f64,
    declare_ms: f64,
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
    tiles: TileCounters,
    /// Water meshes uploaded over the run, and this frame's derivation time.
    water_uploaded: u64,
    water_ms: f64,
    /// World revision each resident water key was derived at.
    water_stamps: BTreeMap<[i32; 3], u64>,
    /// Last sync's window size and sea-level candidate count, for the report.
    water_window: usize,
    water_candidates: usize,
    profile: Option<metrics::FrameLog>,
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
        let profile = match metrics::FrameLog::requested(&directory) {
            Ok(profile) => {
                if let Some(profile) = &profile {
                    log::info!("Landscape frame capture: {}", profile.path.display());
                    eprintln!("Landscape frame capture: {}", profile.path.display());
                }
                profile
            }
            Err(error) => {
                log::warn!("Landscape frame capture request failed: {error}");
                None
            }
        };
        let mut world = World::landscape(SEED);
        let mut camera = spawn_camera();
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
            },
            walking: false,
            plan: Vec::new(),
            declared: Vec::new(),
            resident: BTreeMap::new(),
            tiles: TileCounters::default(),
            water_uploaded: 0,
            water_ms: 0.,
            water_stamps: BTreeMap::new(),
            water_window: 0,
            water_candidates: 0,
            profile,
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
    fn sync_chunks(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let begin = Instant::now();
        let keys = self.world.chunk_keys();
        renderer.retain_chunks(&keys)?;
        if self.preparation.available() {
            for _ in 0..4 {
                let Some((key, mesh)) = self.preparation.poll_mesh(&self.world) else {
                    break;
                };
                if renderer.chunk_revision(key) != Some(mesh.revision) {
                    renderer.upload_chunk(key, &mesh)?;
                }
                if begin.elapsed().as_secs_f64() >= 0.002 {
                    break;
                }
            }
            let mut keys = keys;
            let eye = self.camera.position;
            keys.sort_by_key(|key| {
                let dx = key[0] * 16 + 8 - eye.x as i32;
                let dz = key[2] * 16 + 8 - eye.z as i32;
                dx * dx + dz * dz
            });
            let mut requested = 0;
            for key in keys {
                if renderer.chunk_revision(key) != self.world.chunk_revision(key)
                    && self.preparation.request_mesh(&self.world, key)
                {
                    requested += 1;
                    if requested >= 4 {
                        break;
                    }
                }
            }
        } else {
            for key in keys {
                if renderer.chunk_revision(key) != self.world.chunk_revision(key) {
                    renderer.upload_chunk(key, &self.world.mesh_chunk(key))?;
                }
            }
        }
        Ok(())
    }

    /// Plan the rings for this eye, evict what the plan dropped and build what
    /// it is missing, inside the frame budget.
    fn sync_tiles(&mut self) -> Result<(), String> {
        let declare_begin = Instant::now();
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let eye = self.camera.position.to_array();
        let fine = landscape::fine_clip(eye);
        landscape::ring_plan_into(eye, fine, &LANDSCAPE_RINGS, &mut self.plan);
        self.declared.clear();
        self.declared
            .extend(self.plan.iter().map(|tile| (tile.level, tile.key)));
        // Declare first: the renderer drops what the plan no longer wants before
        // this frame adds anything, so residency never exceeds plan + budget.
        renderer.retain_terrain_tiles(&self.declared)?;
        let work = plan_tile_work(&self.plan, &self.resident, TileBudget::default());
        self.tiles.declare_ms = declare_begin.elapsed().as_secs_f64() * 1000.;
        let begin = Instant::now();
        for key in &work.evict {
            self.resident.remove(key);
        }
        self.tiles.evicted += work.evict.len() as u64;
        let mut uploaded = 0u64;
        let mut skipped = 0usize;
        let mut generate_ms = 0.0;
        let mut upload_ms = 0.0;
        for (index, tile) in work.build.iter().enumerate() {
            if begin.elapsed().as_secs_f64() * 1000. >= MAX_TILE_MS {
                skipped = work.build.len() - index;
                break;
            }
            let filter = normalized_filter(tile);
            let generate_begin = Instant::now();
            let mesh = landscape::lod_tile_mesh(SEED, tile.level, tile.key, filter);
            generate_ms += generate_begin.elapsed().as_secs_f64() * 1000.;
            // A tile entirely inside the hole meshes to nothing. It stays a
            // planned, resident-as-empty tile so it is not rebuilt every frame.
            if !mesh.indices.is_empty() {
                let upload_begin = Instant::now();
                renderer.upload_terrain_tile(tile.level, tile.key, &mesh)?;
                upload_ms += upload_begin.elapsed().as_secs_f64() * 1000.;
                uploaded += 1;
            }
            self.resident.insert((tile.level, tile.key), filter);
        }
        self.tiles.uploaded += uploaded;
        self.tiles.outstanding = work.outstanding + skipped;
        self.tiles.build_ms = begin.elapsed().as_secs_f64() * 1000.;
        self.tiles.generate_ms = generate_ms;
        self.tiles.total_generate_ms += generate_ms;
        self.tiles.generated += (work.build.len() - skipped) as u64;
        self.tiles.upload_ms = upload_ms;
        if self.tiles.build_ms > self.tiles.worst.build_ms {
            self.tiles.worst = WorstFrame {
                tiles: work.build.len() - skipped,
                build_ms: self.tiles.build_ms,
                generate_ms,
                upload_ms,
                declare_ms: self.tiles.declare_ms,
            };
        }
        Ok(())
    }

    /// Derive and upload the water surface of every resident sea-level chunk.
    ///
    /// Only the chunk layer that contains sea level can hold water, so a window
    /// change adds at most one water mesh per column stack. The derivation reads
    /// the authoritative world, so an edit that raises land above sea level
    /// removes its surface on the next sync, exactly like a chunk mesh.
    fn sync_water(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let begin = Instant::now();
        // Residency, not `chunk_keys`: an open-water column stores no voxels at
        // all, so its chunk is absent and a surface over it still has to be
        // derived. Residency is exactly the set of chunks the window published.
        let keys = self
            .world
            .stream_resident_chunks()
            .unwrap_or_else(|| self.world.chunk_keys());
        renderer.retain_water_chunks(&keys)?;
        let level = matterweave_core::water::sea_level_chunk_y();
        let revision = self.world.revision();
        self.water_window = keys.len();
        self.water_candidates = keys.iter().filter(|key| key[1] == level).count();
        let mut uploaded = 0usize;
        for key in &keys {
            if key[1] != level {
                continue;
            }
            // The stamp is the whole-world revision: any edit can change the
            // terrain under a water surface, and re-deriving the sea-level layer
            // costs one mesh per column stack, not one per resident chunk.
            if self.water_stamps.get(key) == Some(&revision) {
                continue;
            }
            renderer.upload_water_chunk(*key, &self.world.water_mesh_chunk(*key))?;
            self.water_stamps.insert(*key, revision);
            uploaded += 1;
            if uploaded >= MAX_WATER_UPLOADS
                || begin.elapsed().as_secs_f64() * 1000. >= MAX_WATER_MS
            {
                break;
            }
        }
        self.water_stamps.retain(|key, _| keys.contains(key));
        self.water_uploaded += uploaded as u64;
        self.water_ms = begin.elapsed().as_secs_f64() * 1000.;
        Ok(())
    }

    fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.95, 0.97, 0.99, 1.];
        let muted = [0.70, 0.79, 0.86, 1.];
        let accent = [0.62, 0.94, 0.76, 1.];
        let panel = [0.03, 0.06, 0.09, 0.80];
        hud.rect([16., 16., 700., 112.], panel);
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
                "TILE {:.2}+{:.2} MS | FRAME {:.1} MS | TILE MEM {} KIB | {}",
                self.tiles.build_ms,
                self.tiles.declare_ms,
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
        hud.text(
            30.,
            111.,
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
        let capturing = self.profile.is_some();
        if let Some(renderer) = &mut self.renderer {
            renderer.begin_frame_diagnostics();
        }
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
            self.preparation.poll_stream(&mut self.world);
        } else {
            self.world.stream_around(self.camera.position.to_array());
        }
        let stream_ms = stream_begin.elapsed().as_secs_f64() * 1000.;
        let mesh_begin = Instant::now();
        if let Err(error) = self
            .sync_chunks()
            .and_then(|()| self.sync_tiles())
            .and_then(|()| self.sync_water())
        {
            log::error!("Landscape geometry failed: {error}");
            eprintln!("Landscape geometry failed: {error}");
            self.failed = true;
            event_loop.exit();
            return;
        }
        let mesh_ms = mesh_begin.elapsed().as_secs_f64() * 1000.;
        let hud = self.hud();
        let matrix = view_projection(&self.camera, size.width as f32 / size.height as f32);
        let eye = self.camera.position.to_array();
        let outcome =
            self.renderer
                .as_mut()
                .unwrap()
                .render_with_lighting(matrix, eye, &hud, &self.lighting);
        let result = match &outcome {
            FrameResult::Presented => metrics::DrawOutcome::Presented,
            FrameResult::OutOfMemory => metrics::DrawOutcome::OutOfMemory,
            FrameResult::Retry | FrameResult::Fatal(_) => metrics::DrawOutcome::Retry,
        };
        match outcome {
            FrameResult::Presented => {
                self.frames += 1;
                if self.frames.is_multiple_of(30) {
                    let tiles = self.renderer.as_ref().unwrap().terrain_tile_stats();
                    let water = self.renderer.as_ref().unwrap().water_stats();
                    log::info!(
                        "LANDSCAPE frame {} frame_ms {:.2} tile_build_ms {:.2} tile_declare_ms {:.2} \
                         tile_generate_ms {:.2} tile_upload_ms {:.2} tiles {}/{} visible {} \
                         uploaded {} evicted {} outstanding {} tile_kib {} \
                         water {}/{} up {} {:.2} ms eye {:?}",
                        self.frames,
                        self.frame_ms,
                        self.tiles.build_ms,
                        self.tiles.declare_ms,
                        self.tiles.generate_ms,
                        self.tiles.upload_ms,
                        tiles.resident,
                        tiles.declared,
                        tiles.visible,
                        self.tiles.uploaded,
                        self.tiles.evicted,
                        self.tiles.outstanding,
                        tiles.bytes / 1024,
                        water.resident,
                        water.visible,
                        self.water_uploaded,
                        self.water_ms,
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
            self.record(result, dt, stream_ms, mesh_ms, now);
        }
        if !self.failed && self.frame_limit.is_some_and(|limit| self.frames >= limit) {
            let tiles = self.renderer.as_ref().unwrap().terrain_tile_stats();
            let water = self.renderer.as_ref().unwrap().water_stats();
            eprintln!(
                "LANDSCAPE SMOKE PASS: {} presented frames; {}",
                self.frames,
                self.renderer.as_ref().unwrap().capabilities
            );
            eprintln!(
                "LANDSCAPE COUNTERS: chunks {}/{} | tiles resident {} declared {} visible {} \
                 | uploaded {} evicted {} outstanding {} | tile mem {} KiB \n\
                 LANDSCAPE WATER: resident {} visible {} uploaded {} | {} KiB | {} ms this frame | \
                 window {} candidates {}\n\
                 frame {:.1} ms\n\
                 LANDSCAPE WORST TILE FRAME: {} tiles in {:.2} ms = {:.2} ms generating + {:.2} ms \
                 uploading (the upload waits on the frame fence) + {:.2} ms planning; \
                 budget {} tiles / {:.1} ms\n\
                 LANDSCAPE TILE COST: {} meshes generated in {:.1} ms total = {:.2} ms each",
                self.renderer.as_ref().unwrap().visible_chunks,
                self.renderer.as_ref().unwrap().resident_chunks,
                tiles.resident,
                tiles.declared,
                tiles.visible,
                self.tiles.uploaded,
                self.tiles.evicted,
                self.tiles.outstanding,
                tiles.bytes / 1024,
                water.resident,
                water.visible,
                self.water_uploaded,
                water.bytes / 1024,
                self.water_ms,
                self.water_window,
                self.water_candidates,
                self.frame_ms,
                self.tiles.worst.tiles,
                self.tiles.worst.build_ms,
                self.tiles.worst.generate_ms,
                self.tiles.worst.upload_ms,
                self.tiles.worst.declare_ms,
                MAX_TILE_UPLOADS,
                MAX_TILE_MS,
                self.tiles.generated,
                self.tiles.total_generate_ms,
                self.tiles.total_generate_ms / self.tiles.generated.max(1) as f64
            );
            event_loop.exit();
        }
    }

    /// One capture row. Only fields this sample actually measures are filled;
    /// systems it does not run stay empty rather than borrowing another
    /// sample's number.
    fn record(
        &mut self,
        result: metrics::DrawOutcome,
        dt: f32,
        stream_ms: f64,
        mesh_ms: f64,
        frame_begin: Instant,
    ) {
        let diagnostics = self.renderer.as_ref().and_then(|r| r.draw_diagnostics());
        let row = metrics::FrameRow {
            draw_attempt_id: self.frames,
            presented_count: self.frames,
            result,
            submitted_gpu_frame_id: diagnostics.and_then(|d| d.submitted_frame_id),
            draw_interval_wall_ms: Some(f64::from(dt) * 1000.),
            main_wall_ms: Some(frame_begin.elapsed().as_secs_f64() * 1000.),
            stream_request_elapsed_ms: Some(stream_ms),
            mesh_sync_wall_ms: Some(mesh_ms),
            chunk_mesh_uploads: u32::try_from(self.tiles.uploaded).ok(),
            ..Default::default()
        };
        let Some(profile) = &mut self.profile else {
            return;
        };
        match profile.record(&row) {
            Ok(true) => {}
            Ok(false) => {
                log::info!("Landscape capture complete: {}", profile.path.display());
                self.profile = None;
            }
            Err(error) => {
                log::warn!("Landscape capture failed: {error}");
                self.profile = None;
            }
        }
        if self.profile.is_none() {
            if let Some(renderer) = &mut self.renderer {
                renderer.set_diagnostics_enabled(false);
            }
        }
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
                renderer.set_diagnostics_enabled(self.profile.is_some());
                self.renderer = Some(renderer);
                self.window = Some(window);
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
        self.renderer = None;
        self.window = None;
        self.resident.clear();
        self.controls.clear();
        self.focused = false;
        if let Some(profile) = &mut self.profile {
            if let Err(error) = profile.flush() {
                log::warn!("Landscape capture flush failed: {error}");
            }
        }
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
        if let Some(profile) = &mut self.profile {
            if let Err(error) = profile.flush() {
                log::warn!("Landscape capture flush failed: {error}");
            }
        }
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
    fn construction_records_a_capture_and_writes_no_world() {
        let dir = temp_dir("capture");
        std::fs::write(dir.join("profile-frames.txt"), "4").unwrap();
        let save = dir.join("world.json");
        let sample = LandscapeSample::new(save.clone(), Some(1));
        // The capture request is consumed at construction, before the first
        // frame, so a marker-selected device run records from frame one.
        assert!(
            sample.profile.is_some(),
            "the capture request was not taken"
        );
        assert!(!dir.join("profile-frames.txt").exists());
        // A viewer must not fabricate a save.
        assert!(!save.exists(), "the landscape sample wrote a world save");
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
