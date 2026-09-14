//! Far-terrain sample: the authoritative streaming window plus six nested
//! distance rings derived from the same generator, out to six kilometres.
//!
//! The 7x7-chunk window around the player is the real world: authoritative one
//! metre voxels, streamed and meshed exactly as the other samples stream them.
//! Everything beyond it is derived on demand from
//! [`matterweave_core::landscape`] at 2 m to 64 m cells by
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
//! Tile meshes are cheap (33x33 = 1089 column samples each) but not free.
//! Generation runs on one bounded background worker; a frame uploads at most
//! [`MAX_TILE_UPLOADS`] meshes and stops as soon as [`MAX_TILE_MS`] of
//! main-thread time has gone into collecting and uploading them, whichever comes
//! first. GPU residency is bounded by `MAX_TERRAIN_TILES`, and every tile the
//! plan drops leaves the renderer the same frame, so a held tile can never sit
//! under the new plan's coarser ring. The dropped mesh stays in a byte-budgeted
//! CPU cache, pinned for a bounded window when the plan has only just wanted
//! it, so an oscillation is an upload rather than a regeneration. See
//! [`crate::landscape_tiles`].
use crate::controls::{Camera, Controls};
use crate::landscape_flora::{LandscapeFlora, PUSH_RADIUS_M, WIND_DIRECTION_XZ, WIND_STRENGTH_M};
use crate::landscape_tiles::{TileStream, MAX_TILE_MS, MAX_TILE_UPLOADS};
use crate::metrics;
use glam::{Mat4, Vec2, Vec3};
use matterweave_core::landscape::{
    self, Clip, RingTile, TileFilter, LANDSCAPE_RINGS, LOD_TILE_CELLS,
};
use matterweave_core::{AsyncWorld, World};
use matterweave_render::{
    Atmosphere, Clouds, FrameResult, Hud, LightingSettings, PlayerPush, Renderer, Sun, Wind,
};
use std::collections::BTreeMap;
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

/// The opt-in marker's contents, or `None` when there is no usable marker.
///
/// A marker that cannot be read, or that exceeds the bound, is reported and
/// treated as absent - a broken marker must not stop the app from starting, and
/// this file is never a game save.
fn marker_text(directory: &Path) -> Option<String> {
    bounded_marker_text(directory, MARKER_FILE)
}

/// One bounded, non-fatal marker read, shared by every marker beside the save.
fn bounded_marker_text(directory: &Path, name: &str) -> Option<String> {
    use std::io::Read;
    let path = directory.join(name);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("Landscape marker {}: {error}", path.display());
            return None;
        }
    };
    let mut text = String::new();
    match file.take(MAX_MARKER_BYTES + 1).read_to_string(&mut text) {
        Ok(_) if text.len() as u64 > MAX_MARKER_BYTES => {
            log::warn!(
                "Landscape marker {} exceeds {MAX_MARKER_BYTES} bytes; ignored",
                path.display()
            );
            None
        }
        Ok(_) => Some(text),
        Err(error) => {
            log::warn!("Landscape marker {}: {error}", path.display());
            None
        }
    }
}

/// Marker selecting the scripted exercise path (a 200 m circle flown at 60 m):
/// the device entry for streaming and per-frame-budget evidence, because a
/// phone run has no command line.
pub const EXERCISE_MARKER_FILE: &str = "landscape-exercise.txt";

/// True when the exercise marker selects the scripted flight. It is read with
/// the same tolerance as the sample marker - unreadable or oversized is absent,
/// never fatal - and `off` keeps the file while stopping the flight, so a
/// captured run can be replayed without deleting it.
pub fn exercise_marker_present(directory: &Path) -> bool {
    match bounded_marker_text(directory, EXERCISE_MARKER_FILE) {
        Some(text) => {
            let text = text.trim();
            text.is_empty() || !text.eq_ignore_ascii_case("off")
        }
        None => false,
    }
}

/// Whether the opt-in marker is present beside the save. Any bytes within the
/// bound select the sample, whatever they say.
pub fn marker_present(directory: &Path) -> bool {
    marker_text(directory).is_some()
}

/// The cloud setting a device run asked for in the marker, since NativeActivity
/// passes no command line and a phone has no C key. A marker word of `clouds
/// low`, `clouds high` or `clouds off` selects it; anything else, including the
/// empty marker every existing device run writes, stays off.
pub fn marker_clouds(directory: &Path) -> CloudChoice {
    let Some(text) = marker_text(directory) else {
        return CloudChoice::Off;
    };
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        if word.eq_ignore_ascii_case("clouds") {
            return words
                .next()
                .and_then(CloudChoice::parse)
                .unwrap_or_default();
        }
    }
    CloudChoice::Off
}

// -- Tile work planning ------------------------------------------------------

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

/// Run-time cloud switch. Three settings so a device capture can compare the
/// same flight with clouds off, cheap and expensive; `Off` is the default, and
/// with it the frame is the one this sample rendered before clouds existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloudChoice {
    #[default]
    Off,
    Low,
    High,
}

impl CloudChoice {
    /// `--clouds` accepts either the names or 0/1/2, so a capture script can
    /// pass whichever it already has.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "off" | "0" => Some(Self::Off),
            "low" | "1" => Some(Self::Low),
            "high" | "2" => Some(Self::High),
            _ => None,
        }
    }

    fn settings(self, time_s: f32) -> Clouds {
        Clouds {
            enabled: self != Self::Off,
            quality: u8::from(self == Self::High),
            time_s,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Low => "LOW",
            Self::High => "HIGH",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::High,
            Self::High => Self::Off,
        }
    }
}

/// `Some(ms)` as `0.00`, `None` as `-`, for the periodic log line: a missing
/// diagnostics reading must not print as a zero.
fn option_ms(value: Option<f64>) -> String {
    value.map_or_else(|| "-".into(), |value| format!("{value:.2}"))
}

fn option_count(value: Option<u32>) -> String {
    value.map_or_else(|| "-".into(), |value| value.to_string())
}

/// One frame's chunk mesh sync, measured where the work happens. Generation is
/// background preparation, so `generate_ms` is nonzero only on the synchronous
/// fallback path; `upload_ms` is the part inside `upload_chunk`, which waits on
/// the frame fence before it replaces a buffer.
#[derive(Clone, Copy, Debug, Default)]
struct ChunkFrame {
    /// Chunk meshes uploaded to the renderer this frame.
    uploads: u32,
    /// Completed background meshes accepted from the pool this frame.
    polled: u32,
    /// Mesh jobs handed to the background pool this frame.
    requested: u32,
    /// Main-thread mesh generation this frame (synchronous fallback only).
    generate_ms: f64,
    /// Main-thread time inside `upload_chunk` calls this frame.
    upload_ms: f64,
    /// Wall time of the whole chunk sync this frame, including retain/declare.
    wall_ms: f64,
}

/// One draw attempt's phase measurements, taken where each span happened and
/// handed to [`LandscapeSample::record`] as one value. All wall times; each is
/// `None` when that term was not measured for this attempt.
#[derive(Clone, Copy, Debug, Default)]
struct FrameTimings {
    dt: f32,
    stream_ms: f64,
    mesh_ms: f64,
    cpu_busy_ms: Option<f64>,
    render_ms: Option<f64>,
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
    /// Background tile generation, the CPU mesh cache and residency, including
    /// the hysteresis that keeps a just-dropped tile resident.
    tiles: TileStream,
    /// This frame's chunk mesh sync, and the running upload total.
    chunks: ChunkFrame,
    chunks_uploaded: u64,
    /// Frame-capture identity: increments once per renderer creation or
    /// recreation, so per-renderer GPU submission counters cannot be joined
    /// across one.
    renderer_epoch: u64,
    /// Frame-capture identity: increments once per recorded draw attempt.
    /// [`Self::frames`] only advances on a presented attempt, so a retry after
    /// rotation would otherwise share an attempt id with the row before it.
    draw_attempts: u64,
    /// Deduplicates repeated reads of one completed GPU submission; the
    /// renderer epoch is part of the identity.
    gpu_completions: metrics::GpuCompletionTracker,
    /// Dense vegetation: the pooled prototypes plus the field for the eye's
    /// cell. `None` until the renderer exists, because the first
    /// [`LandscapeFlora::sync`] installs the scene rather than updating one.
    flora: Option<LandscapeFlora>,
    /// Wind animation clock in seconds, advanced by frame time. The renderer
    /// folds it into its animation period, so a long session cannot lose sine
    /// precision here.
    wind_time: f32,
    /// Cloud animation clock in seconds, advanced by frame time like the wind
    /// clock. The renderer folds it into its own period.
    cloud_time: f32,
    /// Which cloud setting this run is flying with. Switched by C at run time
    /// and by `--clouds` before the first frame.
    pub clouds: CloudChoice,
    /// Water meshes uploaded over the run, and this frame's phase times: the
    /// whole sync, the mesh derivation, and the upload calls (which fence).
    water_uploaded: u64,
    water_ms: f64,
    water_generate_ms: f64,
    water_upload_ms: f64,
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
        let terrain_source = world.terrain_source();
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
                    // This sample is the one that looks at the sky: rings reach
                    // six kilometres and the eye flies above the terrain.
                    sky_gradient: true,
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
                // Off unless the run asks: the phone decides what this costs,
                // and the default frame stays the one every capture compared.
                clouds: Clouds::default(),
            },
            walking: false,
            plan: Vec::new(),
            tiles: TileStream::new(terrain_source, SEED),
            chunks: ChunkFrame::default(),
            chunks_uploaded: 0,
            renderer_epoch: 0,
            draw_attempts: 0,
            gpu_completions: metrics::GpuCompletionTracker::default(),
            flora: None,
            wind_time: 0.,
            cloud_time: 0.,
            clouds: CloudChoice::default(),
            water_uploaded: 0,
            water_ms: 0.,
            water_generate_ms: 0.,
            water_upload_ms: 0.,
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
        let mut frame = ChunkFrame::default();
        let keys = self.world.chunk_keys();
        renderer.retain_chunks(&keys)?;
        if self.preparation.available() {
            for _ in 0..4 {
                let Some((key, mesh)) = self.preparation.poll_mesh(&self.world) else {
                    break;
                };
                frame.polled += 1;
                if renderer.chunk_revision(key) != Some(mesh.revision) {
                    let upload_begin = Instant::now();
                    renderer.upload_chunk(key, &mesh)?;
                    frame.upload_ms += upload_begin.elapsed().as_secs_f64() * 1000.;
                    frame.uploads += 1;
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
            for key in keys {
                if renderer.chunk_revision(key) != self.world.chunk_revision(key)
                    && self.preparation.request_mesh(&self.world, key)
                {
                    frame.requested += 1;
                    if frame.requested >= 4 {
                        break;
                    }
                }
            }
        } else {
            for key in keys {
                if renderer.chunk_revision(key) != self.world.chunk_revision(key) {
                    let generate_begin = Instant::now();
                    let mesh = self.world.mesh_chunk(key);
                    frame.generate_ms += generate_begin.elapsed().as_secs_f64() * 1000.;
                    let upload_begin = Instant::now();
                    renderer.upload_chunk(key, &mesh)?;
                    frame.upload_ms += upload_begin.elapsed().as_secs_f64() * 1000.;
                    frame.uploads += 1;
                }
            }
        }
        frame.wall_ms = begin.elapsed().as_secs_f64() * 1000.;
        self.chunks_uploaded += u64::from(frame.uploads);
        self.chunks = frame;
        Ok(())
    }

    /// Plan the rings for this eye, then let the tile stream declare residency,
    /// evict, collect generated meshes and upload what the cache can serve.
    fn sync_tiles(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        let eye = self.camera.position.to_array();
        let fine = landscape::fine_clip(eye);
        landscape::ring_plan_into(eye, fine, &LANDSCAPE_RINGS, &mut self.plan);
        self.tiles.sync(renderer, &self.plan, self.frames)
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
        let mut generate_ms = 0.0;
        let mut upload_ms = 0.0;
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
            let generate_begin = Instant::now();
            let mesh = self.world.water_mesh_chunk(*key);
            generate_ms += generate_begin.elapsed().as_secs_f64() * 1000.;
            let upload_begin = Instant::now();
            renderer.upload_water_chunk(*key, &mesh)?;
            upload_ms += upload_begin.elapsed().as_secs_f64() * 1000.;
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
        self.water_generate_ms = generate_ms;
        self.water_upload_ms = upload_ms;
        Ok(())
    }

    fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.95, 0.97, 0.99, 1.];
        let muted = [0.70, 0.79, 0.86, 1.];
        let accent = [0.62, 0.94, 0.76, 1.];
        let panel = [0.03, 0.06, 0.09, 0.80];
        hud.rect([16., 16., 700., 168.], panel);
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
        let tile_work = self.tiles.counters();
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
                tile_work.uploaded,
                tile_work.evicted,
                tile_work.outstanding
            ),
            1.15,
            accent,
        );
        hud.text(
            30.,
            74.,
            &format!(
                "TILE {:.2}+{:.2} MS | PIN {} GEN {} HIT {} | FRAME {:.1} MS | MEM {} KIB | {}",
                tile_work.build_ms,
                tile_work.declare_ms,
                tile_work.pinned,
                tile_work.generated,
                tile_work.served_from_cache,
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
        hud.text(30., 129., &self.cloud_line(), 1., accent);
        hud.text(
            30.,
            146.,
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
            "WASD FLY | SPACE/SHIFT UP DOWN | RMB LOOK | G WALK OR FLY | C CLOUDS | ESC BACK",
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
        let cpu_busy = metrics::CpuBusySpan::begin(capturing);
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
        // The cloud layer drifts on its own clock. Switching the setting off
        // frees the offscreen target on the next frame; switching it back on
        // resumes where the clock has moved to, not where it was left.
        self.cloud_time += dt.clamp(0.0, 0.1);
        self.lighting.clouds = self.clouds.settings(self.cloud_time);
        let hud = self.hud();
        let matrix = view_projection(&self.camera, size.width as f32 / size.height as f32);
        let render_begin = capturing.then(Instant::now);
        let outcome =
            self.renderer
                .as_mut()
                .unwrap()
                .render_with_lighting(matrix, eye, &hud, &self.lighting);
        let render_ms = render_begin.map(|begin| begin.elapsed().as_secs_f64() * 1000.);
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
                    let fence = self.renderer.as_ref().and_then(|r| r.draw_diagnostics());
                    let tile_work = self.tiles.counters();
                    // Built as one string so the desktop host, which has no
                    // logger installed, prints the same per-phase line to
                    // stderr that an Android run sends to logcat.
                    let message = format!(
                        "LANDSCAPE frame {} frame_ms {:.2} \
                         chunk_ms {:.2} chunk_up {} chunk_req {} chunk_poll {} \
                         chunk_generate_ms {:.2} chunk_upload_ms {:.2} \
                         tile_build_ms {:.2} tile_declare_ms {:.2} \
                         tile_generate_ms {:.2} tile_upload_ms {:.2} tile_req {} tiles {}/{} visible {} \
                         uploaded {} evicted {} outstanding {} pinned {} cache_hit {} cache_kib {} \
                         worker_pending {} tile_kib {} \
                         water {}/{} up {} {:.2} ms generate {:.2} upload {:.2} \
                         fence_ms {} fence_waits {} eye {:?}",
                        self.frames,
                        self.frame_ms,
                        self.chunks.wall_ms,
                        self.chunks.uploads,
                        self.chunks.requested,
                        self.chunks.polled,
                        self.chunks.generate_ms,
                        self.chunks.upload_ms,
                        tile_work.build_ms,
                        tile_work.declare_ms,
                        tile_work.generate_ms,
                        tile_work.upload_ms,
                        tile_work.requested,
                        tiles.resident,
                        tiles.declared,
                        tiles.visible,
                        tile_work.uploaded,
                        tile_work.evicted,
                        tile_work.outstanding,
                        tile_work.pinned,
                        tile_work.served_from_cache,
                        self.tiles.cache_bytes() / 1024,
                        tile_work.pending,
                        tiles.bytes / 1024,
                        water.resident,
                        water.visible,
                        self.water_uploaded,
                        self.water_ms,
                        self.water_generate_ms,
                        self.water_upload_ms,
                        option_ms(fence.and_then(|d| d.upload_fence_wait_ms)),
                        option_count(fence.and_then(|d| d.upload_fence_waits)),
                        eye
                    );
                    log::info!("{message}");
                    #[cfg(not(target_os = "android"))]
                    eprintln!("{message}");
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
        let cpu_busy_ms = cpu_busy.finish();
        if capturing {
            self.record(
                result,
                FrameTimings {
                    dt,
                    stream_ms,
                    mesh_ms,
                    cpu_busy_ms,
                    render_ms,
                },
                now,
            );
        }
        if !self.failed && self.frame_limit.is_some_and(|limit| self.frames >= limit) {
            let tiles = self.renderer.as_ref().unwrap().terrain_tile_stats();
            let flora = self
                .flora
                .as_ref()
                .map(LandscapeFlora::counters)
                .unwrap_or_default();
            let water = self.renderer.as_ref().unwrap().water_stats();
            let tile_work = self.tiles.counters();
            eprintln!(
                "LANDSCAPE SMOKE PASS: {} presented frames; {}",
                self.frames,
                self.renderer.as_ref().unwrap().capabilities
            );
            eprintln!(
                "LANDSCAPE COUNTERS: chunks {}/{} | tiles resident {} declared {} visible {} \
                 | uploaded {} evicted {} outstanding {} pinned {} | cache hit {} {} KiB / {} \
                 entries | worker {} pending | tile mem {} KiB \n\
                 LANDSCAPE CHUNK: {} uploaded over the run | last frame {:.2} ms (generate {:.2}, \
                 upload {:.2}) | {} requested {} polled\n\
                 LANDSCAPE WATER: resident {} visible {} uploaded {} | {} KiB | {} ms this frame \
                 (generate {:.2}, upload {:.2}) | window {} candidates {}\n\
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
                tile_work.uploaded,
                tile_work.evicted,
                tile_work.outstanding,
                tile_work.pinned,
                tile_work.served_from_cache,
                self.tiles.cache_bytes() / 1024,
                self.tiles.cache_entries(),
                tile_work.pending,
                tiles.bytes / 1024,
                self.chunks_uploaded,
                self.chunks.wall_ms,
                self.chunks.generate_ms,
                self.chunks.upload_ms,
                self.chunks.requested,
                self.chunks.polled,
                water.resident,
                water.visible,
                self.water_uploaded,
                water.bytes / 1024,
                self.water_ms,
                self.water_generate_ms,
                self.water_upload_ms,
                self.water_window,
                self.water_candidates,
                self.frame_ms,
                tile_work.worst.tiles,
                tile_work.worst.build_ms,
                tile_work.worst.generate_ms,
                tile_work.worst.upload_ms,
                tile_work.worst.declare_ms,
                MAX_TILE_UPLOADS,
                MAX_TILE_MS,
                tile_work.generated,
                tile_work.total_generate_ms,
                tile_work.total_generate_ms / tile_work.generated.max(1) as f64
            );
            eprintln!("LANDSCAPE SKY: {}", self.cloud_line());
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
    /// systems it does not run stay empty rather than borrowing another
    /// sample's number.
    fn record(
        &mut self,
        result: metrics::DrawOutcome,
        timings: FrameTimings,
        frame_begin: Instant,
    ) {
        let epoch = self.renderer_epoch;
        let diagnostics = self.renderer.as_ref().and_then(|r| r.draw_diagnostics());
        // Completions repeat until the next submission finishes; the epoch keeps
        // a recreated renderer's restarted ids from joining an old submission.
        let gpu = self
            .renderer
            .as_ref()
            .and_then(|r| r.gpu_timings())
            .filter(|timings| self.gpu_completions.accept(epoch, timings.frame_id));
        let shadow_casters = self
            .renderer
            .as_ref()
            .map(|r| r.shadow_caster_meshes())
            .and_then(|n| u32::try_from(n).ok());
        // A recorded attempt is its own attempt, whether it presented or
        // retried; `frames` only advances on a successful present API call.
        self.draw_attempts += 1;
        let row = metrics::FrameRow {
            draw_attempt_id: self.draw_attempts,
            renderer_epoch: epoch,
            presented_count: self.frames,
            result,
            submitted_gpu_frame_id: diagnostics.and_then(|d| d.submitted_frame_id),
            completed_gpu_frame_id: gpu.map(|t| t.frame_id),
            completed_gpu_renderer_epoch: gpu.map(|_| epoch),
            draw_interval_wall_ms: Some(f64::from(timings.dt) * 1000.),
            main_wall_ms: Some(frame_begin.elapsed().as_secs_f64() * 1000.),
            main_cpu_busy_ms: timings.cpu_busy_ms,
            stream_request_elapsed_ms: Some(timings.stream_ms),
            mesh_sync_wall_ms: Some(timings.mesh_ms),
            mesh_sync_fence_wait_wall_ms: diagnostics.and_then(|d| d.upload_fence_wait_ms),
            mesh_sync_fence_waits: diagnostics.and_then(|d| d.upload_fence_waits),
            render_wall_ms: timings.render_ms,
            render_fence_wait_wall_ms: diagnostics.and_then(|d| d.render_fence_wait_ms),
            acquire_wall_ms: diagnostics.and_then(|d| d.acquire_ms),
            present_wall_ms: diagnostics.and_then(|d| d.present_ms),
            gpu_prev_render_ms: gpu.map(|t| t.render_ms),
            gpu_prev_shadow_ms: gpu.and_then(|t| t.shadow_ms),
            gpu_prev_shadows: gpu.map(|t| t.shadows),
            gpu_prev_shadow_map_size: gpu.map(|t| t.shadow_map_size),
            // Actual chunk uploads this frame. Tile uploads are reported in the
            // log; the schema has no tile column, and this one is named chunks.
            chunk_mesh_uploads: Some(self.chunks.uploads),
            // Shadow work counts only for an attempt that submitted; a retry
            // must not inherit the previous pass's caster count.
            shadow_caster_meshes: metrics::shadow_casters_for_attempt(
                diagnostics.and_then(|d| d.submitted_frame_id),
                shadow_casters,
            ),
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

    /// What the sky and cloud passes are actually doing: the setting, the
    /// offscreen resolution the renderer allocated (none when clouds are off)
    /// and the measured GPU cost of the cloud pass. The cost is absent unless
    /// the device supports timestamps and a completed submission carried the
    /// pass; it is never a zero standing in for an unavailable measurement.
    fn cloud_line(&self) -> String {
        let renderer = self.renderer.as_ref();
        let target = match renderer.and_then(Renderer::cloud_target) {
            Some(((width, height), quality)) => format!("{width}x{height} Q{quality}"),
            None => "NONE".into(),
        };
        let cost = match renderer
            .and_then(Renderer::gpu_timings)
            .and_then(|timings| timings.cloud_ms)
        {
            Some(ms) => format!("{ms:.2} MS"),
            None => "UNAVAILABLE".into(),
        };
        format!(
            "SKY DOME ON | CLOUDS {} | TARGET {target} | CLOUD GPU {cost}",
            self.clouds.label()
        )
    }

    fn cycle_clouds(&mut self) {
        self.clouds = self.clouds.next();
        self.status = format!("Clouds {}", self.clouds.label().to_lowercase());
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
                // A new renderer restarts its GPU submission counter; the epoch
                // labels every capture row so the two generations cannot join.
                self.renderer_epoch += 1;
                renderer.set_diagnostics_enabled(self.profile.is_some());
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
        // Every GPU tile dies with the renderer, so residency is cleared too.
        // The CPU cache and the generation worker hold no GPU objects and
        // stay, so the next resume re-uploads instead of regenerating.
        self.tiles.forgot_renderer();
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
                            KeyCode::KeyC => self.cycle_clouds(),
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

    #[test]
    fn a_missing_or_broken_marker_is_not_an_opt_in() {
        let dir = temp_dir("marker");
        assert!(!marker_present(&dir));
        std::fs::write(dir.join(MARKER_FILE), "").unwrap();
        assert!(marker_present(&dir), "an empty marker is still an opt-in");
        // Content beyond the reserved cloud word is ignored, not rejected.
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
    fn a_marker_can_ask_a_device_run_for_clouds() {
        let dir = temp_dir("marker-clouds");
        // No marker, an empty marker and an unrelated marker all stay off, so
        // every device run that already writes one is unchanged.
        assert_eq!(marker_clouds(&dir), CloudChoice::Off);
        for text in ["", "anything at all\n", "clouds\n", "clouds sideways"] {
            std::fs::write(dir.join(MARKER_FILE), text).unwrap();
            assert_eq!(marker_clouds(&dir), CloudChoice::Off, "{text:?}");
        }
        for (text, expected) in [
            ("clouds low", CloudChoice::Low),
            ("CLOUDS High\n", CloudChoice::High),
            ("landscape\nclouds 2\n", CloudChoice::High),
            ("clouds off", CloudChoice::Off),
        ] {
            std::fs::write(dir.join(MARKER_FILE), text).unwrap();
            assert_eq!(marker_clouds(&dir), expected, "{text:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cloud_switch_cycles_and_maps_to_renderer_settings() {
        assert_eq!(CloudChoice::default(), CloudChoice::Off);
        assert!(!CloudChoice::Off.settings(12.0).enabled);
        let low = CloudChoice::Low.settings(12.0);
        assert!(low.enabled && low.quality == 0 && low.time_s == 12.0);
        let high = CloudChoice::High.settings(12.0);
        assert!(high.enabled && high.quality == 1);
        // C walks all three and returns to the setting it started from.
        let mut choice = CloudChoice::Off;
        for expected in [CloudChoice::Low, CloudChoice::High, CloudChoice::Off] {
            choice = choice.next();
            assert_eq!(choice, expected);
        }
        assert_eq!(CloudChoice::parse("nonsense"), None);
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
