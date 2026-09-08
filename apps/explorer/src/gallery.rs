//! Opt-in native detail gallery: an explicitly isolated fly/viewer mode over
//! `matterweave_detail::gallery_scene`.
//!
//! This mode never loads, renames, replaces or saves the player's world or
//! session: it owns no `World`, no `Physics` and no save path. It is a viewer
//! for authoritative detail source data, not a claim of native collision or
//! gameplay integration.
//!
//! Geometry path: one cached prototype `Mesh` per (prototype, LOD) from the
//! scene cache is transformed and copied into a single combined world-space
//! `Mesh` **once per view**, uploaded through the renderer's existing
//! whole-world compatibility path. This is an initial combined-mesh adapter.
//! It is not GPU instancing and not the selected full-scale architecture.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use crate::{
    controls::{Camera, Controls},
    metrics,
};
use glam::{Vec2, Vec3};
use matterweave_core::{Mesh, Vertex};
use matterweave_detail::{Bounds, DetailError, DetailScene, Lod};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    window::{Window, WindowId},
};

/// Marker file placed beside the normal world save to request the gallery.
pub const MARKER_FILE: &str = "detail-gallery.txt";
/// Host environment variable carrying the same request text.
pub const ENV_VAR: &str = "MATTERWEAVE_DETAIL_GALLERY";
/// Accepted marker/env request size. Anything larger is rejected, not truncated.
pub const MAX_REQUEST_BYTES: u64 = 128;
/// Seed of the accepted P03 gallery scene.
pub const GALLERY_SEED: u64 = 2026;
/// Combined world-space mesh budget for this adapter.
pub const MAX_COMBINED_MESH_BYTES: usize = 64 * 1024 * 1024;

const TILE_PROTOTYPE: &str = "terrain_tile_16m";
const PARASOL_PROTOTYPE: &str = "parasol_mushroom";

#[derive(Debug)]
pub enum GalleryError {
    /// The developer's request text is not a valid preset/LOD pair.
    Request(String),
    /// The marker file exists but could not be read within its bound.
    Marker {
        path: PathBuf,
        message: String,
    },
    Detail(DetailError),
    /// Combined-mesh construction refused the input.
    Geometry(String),
}

impl std::fmt::Display for GalleryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(message) => write!(f, "invalid detail gallery request: {message}"),
            Self::Marker { path, message } => {
                write!(f, "detail gallery marker {}: {message}", path.display())
            }
            Self::Detail(error) => write!(f, "detail scene error: {error}"),
            Self::Geometry(message) => write!(f, "detail gallery geometry: {message}"),
        }
    }
}

impl std::error::Error for GalleryError {}

impl From<DetailError> for GalleryError {
    fn from(error: DetailError) -> Self {
        Self::Detail(error)
    }
}

type Result<T> = std::result::Result<T, GalleryError>;

/// Recorded viewpoints. Each one is derived from actual scene bounds, never
/// from hand-guessed coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Tile,
    Flora,
    ParasolFront,
    ParasolSide,
    ParasolUnderside,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tile => "tile",
            Self::Flora => "flora",
            Self::ParasolFront => "parasol-front",
            Self::ParasolSide => "parasol-side",
            Self::ParasolUnderside => "parasol-underside",
        }
    }
    fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "tile" => Self::Tile,
            "flora" => Self::Flora,
            "parasol-front" => Self::ParasolFront,
            "parasol-side" => Self::ParasolSide,
            "parasol-underside" => Self::ParasolUnderside,
            _ => return None,
        })
    }
}

fn lod_label(lod: Lod) -> &'static str {
    match lod {
        Lod::Source => "source",
        Lod::Half => "half",
        Lod::Quarter => "quarter",
    }
}

fn parse_lod(token: &str) -> Option<Lod> {
    Some(match token {
        "source" => Lod::Source,
        "half" => Lod::Half,
        "quarter" => Lod::Quarter,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub preset: Preset,
    pub lod: Lod,
}

impl Request {
    /// Parses one bounded request line, e.g. `tile source` or `parasol-side`.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() as u64 > MAX_REQUEST_BYTES {
            return Err(GalleryError::Request(format!(
                "request exceeds {MAX_REQUEST_BYTES} bytes"
            )));
        }
        let line = text
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .find(|line| !line.is_empty())
            .ok_or_else(|| GalleryError::Request("empty request".into()))?;
        let mut tokens = line.split_whitespace();
        let preset = tokens.next().and_then(Preset::parse).ok_or_else(|| {
            GalleryError::Request(format!(
                "expected one of tile, flora, parasol-front, parasol-side, parasol-underside in {line:?}"
            ))
        })?;
        let lod = match tokens.next() {
            None => Lod::Source,
            Some(token) => parse_lod(token).ok_or_else(|| {
                GalleryError::Request(format!("expected one of source, half, quarter in {line:?}"))
            })?,
        };
        if tokens.next().is_some() {
            return Err(GalleryError::Request(format!(
                "expected `<preset> [lod]` only in {line:?}"
            )));
        }
        if preset == Preset::Flora && lod != Lod::Source {
            return Err(GalleryError::Request(
                "flora requires source LOD; coarse anatomy is not quality-accepted".into(),
            ));
        }
        Ok(Self { preset, lod })
    }

    /// Resolves an explicit opt-in. `Ok(None)` means normal application
    /// behaviour; an invalid request is an error and must never fall through to
    /// the normal world.
    pub fn resolve(marker: &Path, env: Option<&str>) -> Result<Option<Self>> {
        if let Some(text) = env {
            if !text.trim().is_empty() {
                return Self::parse(text).map(Some);
            }
        }
        let text = match read_marker(marker) {
            Ok(None) => return Ok(None),
            Ok(Some(text)) => text,
            Err(message) => {
                return Err(GalleryError::Marker {
                    path: marker.to_path_buf(),
                    message,
                })
            }
        };
        Self::parse(&text).map(Some)
    }

    pub fn label(&self) -> String {
        format!("{} {}", self.preset.label(), lod_label(self.lod))
    }
}

fn read_marker(path: &Path) -> std::result::Result<Option<String>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut text = String::new();
    file.take(MAX_REQUEST_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    if text.len() as u64 > MAX_REQUEST_BYTES {
        return Err(format!("marker exceeds {MAX_REQUEST_BYTES} bytes"));
    }
    Ok(Some(text))
}

/// Truthful counters for the HUD and the log. Systems this mode does not run
/// report zero or "not available", never a borrowed gameplay number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GalleryStats {
    pub prototypes: usize,
    pub instances: usize,
    pub unique_stored_cells: usize,
    pub expanded_occupied_cells: usize,
    pub expanded_collision_cells: usize,
    pub expanded_liquid_cells: usize,
    /// Authoritative prototype payload only.
    pub source_bytes: usize,
    /// Scene-held derived prototype mesh capacities. Separate from the combine.
    pub cached_mesh_bytes: usize,
    /// Derived prototype meshes built: one per prototype at the chosen LOD.
    pub mesh_builds: u64,
    /// Combined world-space adapter mesh capacities. Excludes allocator
    /// metadata, GPU buffers and renderer state.
    pub combined_mesh_bytes: usize,
    pub combined_vertices: usize,
    pub combined_triangles: usize,
}

/// A built gallery view: one combined mesh plus the derived viewpoint.
pub struct GalleryView {
    pub request: Request,
    pub mesh: Mesh,
    pub stats: GalleryStats,
    pub eye: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

fn mesh_bytes(mesh: &Mesh) -> usize {
    mesh.vertices.capacity() * std::mem::size_of::<Vertex>() + mesh.indices.capacity() * 4
}

/// Transforms and copies each cached prototype mesh into one combined
/// world-space mesh. Prototype geometry is fetched once per prototype, never
/// per instance.
pub fn combine(scene: &mut DetailScene, lod: Lod) -> Result<Mesh> {
    let draws = scene.draws();
    let mut prototypes: Vec<String> = draws.iter().map(|draw| draw.prototype.clone()).collect();
    prototypes.sort();
    prototypes.dedup();
    let mut combined = Mesh::default();
    for prototype in &prototypes {
        let placements: Vec<_> = draws
            .iter()
            .filter(|draw| &draw.prototype == prototype)
            .map(|draw| draw.transform)
            .collect();
        let source = scene.prototype_mesh(prototype, lod)?;
        let vertex_count = u32::try_from(source.vertices.len()).map_err(|_| {
            GalleryError::Geometry(format!("prototype {prototype} exceeds u32 vertices"))
        })?;
        if source.indices.len() % 3 != 0 {
            return Err(GalleryError::Geometry(format!(
                "prototype {prototype} index count {} is not a triangle list",
                source.indices.len()
            )));
        }
        if source.indices.iter().any(|index| *index >= vertex_count) {
            return Err(GalleryError::Geometry(format!(
                "prototype {prototype} has an index outside its {vertex_count} vertices"
            )));
        }
        for transform in placements {
            transform.validate()?;
            let base = u32::try_from(combined.vertices.len())
                .map_err(|_| GalleryError::Geometry("combined mesh exceeds u32 vertices".into()))?;
            base.checked_add(vertex_count).ok_or_else(|| {
                GalleryError::Geometry("combined mesh exceeds u32 vertices".into())
            })?;
            let projected = (combined.vertices.len() + source.vertices.len())
                * std::mem::size_of::<Vertex>()
                + (combined.indices.len() + source.indices.len()) * 4;
            if projected > MAX_COMBINED_MESH_BYTES {
                return Err(GalleryError::Geometry(format!(
                    "combined mesh exceeds {MAX_COMBINED_MESH_BYTES} bytes"
                )));
            }
            combined
                .vertices
                .extend(source.vertices.iter().map(|vertex| Vertex {
                    position: transform.point_to_world(vertex.position),
                    normal: transform.direction_to_world(vertex.normal),
                    color: vertex.color,
                }));
            combined
                .indices
                .extend(source.indices.iter().map(|index| index + base));
        }
    }
    combined.revision = 1;
    Ok(combined)
}

fn instance_bounds(scene: &DetailScene, prototype_id: &str) -> Result<Bounds> {
    let draw = scene
        .draws()
        .into_iter()
        .find(|draw| draw.prototype == prototype_id)
        .ok_or_else(|| {
            GalleryError::Geometry(format!("gallery scene has no {prototype_id} instance"))
        })?;
    let volume = scene.prototype(&draw.prototype).ok_or_else(|| {
        GalleryError::Geometry(format!("gallery scene has no {prototype_id} prototype"))
    })?;
    volume
        .bounds_world(&draw.transform)?
        .ok_or_else(|| GalleryError::Geometry(format!("{prototype_id} instance is empty")))
}

fn look_at(eye: [f32; 3], target: [f32; 3]) -> Result<(f32, f32)> {
    let delta = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if !length.is_finite() || length < 1e-4 {
        return Err(GalleryError::Geometry(
            "viewpoint coincides with its target".into(),
        ));
    }
    let unit = delta.map(|value| value / length);
    // Matches `Camera::forward`: (sin yaw cos pitch, sin pitch, cos yaw cos pitch).
    Ok((unit[0].atan2(unit[2]), unit[1].clamp(-1., 1.).asin()))
}

/// Viewpoint derived from actual scene bounds for one preset.
pub fn viewpoint(scene: &DetailScene, preset: Preset) -> Result<([f32; 3], f32, f32)> {
    let bounds = match preset {
        Preset::Tile | Preset::Flora => instance_bounds(scene, TILE_PROTOTYPE)?,
        _ => instance_bounds(scene, PARASOL_PROTOTYPE)?,
    };
    let centre = [
        (bounds.min[0] + bounds.max[0]) / 2.,
        (bounds.min[1] + bounds.max[1]) / 2.,
        (bounds.min[2] + bounds.max[2]) / 2.,
    ];
    let size = [
        bounds.max[0] - bounds.min[0],
        bounds.max[1] - bounds.min[1],
        bounds.max[2] - bounds.min[2],
    ];
    let extent = size.iter().copied().fold(0.0_f32, f32::max).max(0.5);
    let (eye, target) = match preset {
        Preset::Tile | Preset::Flora => (
            [
                centre[0] + extent * 0.55,
                bounds.max[1] + extent * 0.65,
                centre[2] + extent * 1.15,
            ],
            centre,
        ),
        Preset::ParasolFront => (
            [
                centre[0],
                centre[1] + extent * 0.15,
                centre[2] + extent * 2.2,
            ],
            centre,
        ),
        Preset::ParasolSide => (
            [
                centre[0] + extent * 2.2,
                centre[1] + extent * 0.15,
                centre[2],
            ],
            centre,
        ),
        // Just above the supporting surface, looking up into the cap.
        Preset::ParasolUnderside => (
            [
                centre[0] + extent * 0.95,
                bounds.min[1] + extent * 0.18,
                centre[2] + extent * 0.95,
            ],
            [centre[0], bounds.max[1] - size[1] * 0.2, centre[2]],
        ),
    };
    let (yaw, pitch) = look_at(eye, target)?;
    Ok((eye, yaw, pitch))
}

impl GalleryView {
    /// Builds the accepted gallery scene, one combined mesh and the viewpoint.
    /// Touches no world, session or save file.
    pub fn build(request: Request) -> Result<Self> {
        // Scene generation and prototype meshing happen once on entry, never
        // in the frame loop or on renderer recreation.
        let mut scene = match request.preset {
            Preset::Flora => {
                matterweave_detail::dense_tile(matterweave_detail::FLORA_CANONICAL_SEED)?
            }
            _ => matterweave_detail::gallery_scene(GALLERY_SEED)?,
        };
        let mesh = combine(&mut scene, request.lod)?;
        let counts = scene.counts();
        let stats = GalleryStats {
            prototypes: counts.prototypes,
            instances: counts.instances,
            unique_stored_cells: counts.unique_stored_cells,
            expanded_occupied_cells: counts.expanded_occupied_cells,
            expanded_collision_cells: counts.expanded_collision_cells,
            expanded_liquid_cells: counts.expanded_liquid_cells,
            source_bytes: counts.source_bytes,
            cached_mesh_bytes: counts.cached_mesh_bytes,
            mesh_builds: counts.mesh_builds,
            combined_mesh_bytes: mesh_bytes(&mesh),
            combined_vertices: mesh.vertices.len(),
            combined_triangles: mesh.indices.len() / 3,
        };
        let (eye, yaw, pitch) = viewpoint(&scene, request.preset)?;
        Ok(Self {
            request,
            mesh,
            stats,
            eye,
            yaw,
            pitch,
        })
    }
}

/// Isolated viewer application: no world, no physics, no save path, no session.
///
/// The combined mesh is uploaded once per renderer (creation and any
/// recreation), never per frame.
pub struct GalleryApp {
    view: GalleryView,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    camera: Camera,
    controls: Controls,
    lighting: LightingSettings,
    uploaded: bool,
    uploads: u64,
    upload_ms: Option<f64>,
    frame_upload_ms: Option<f64>,
    /// Bounded viewer lifecycle check. It never edits, saves or simulates.
    pub exercise: bool,
    exercise_stage: u8,
    resize_seen: bool,
    profile: Option<metrics::FrameLog>,
    gpu_completions: metrics::GpuCompletionTracker,
    renderer_epoch: u64,
    draw_attempts: u64,
    frames: u64,
    frame_limit: Option<u64>,
    last_frame: Instant,
    frame_ms: f64,
    cpu_ms: f64,
    status: String,
    focused: bool,
    failed: bool,
}

impl GalleryApp {
    pub fn new(view: GalleryView, data_directory: &Path, frame_limit: Option<u64>) -> Self {
        let profile = match metrics::FrameLog::requested(data_directory) {
            Ok(profile) => {
                if let Some(profile) = &profile {
                    log::info!("Gallery frame capture: {}", profile.path.display());
                }
                profile
            }
            Err(error) => {
                log::warn!("Gallery frame capture request failed: {error}");
                None
            }
        };
        let camera = Camera {
            position: Vec3::from_array(view.eye),
            yaw: view.yaw,
            pitch: view.pitch,
        };
        let status = format!("Detail gallery: {} (viewer only)", view.request.label());
        Self {
            view,
            renderer: None,
            window: None,
            camera,
            controls: Controls::default(),
            lighting: LightingSettings::default(),
            uploaded: false,
            uploads: 0,
            upload_ms: None,
            frame_upload_ms: None,
            exercise: false,
            exercise_stage: 0,
            resize_seen: false,
            profile,
            gpu_completions: metrics::GpuCompletionTracker::default(),
            renderer_epoch: 0,
            draw_attempts: 0,
            frames: 0,
            frame_limit,
            last_frame: Instant::now(),
            frame_ms: 0.,
            cpu_ms: 0.,
            status,
            focused: true,
            failed: false,
        }
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    fn wants_frames(&self) -> bool {
        self.focused || self.frame_limit.is_some()
    }

    fn flush_profile(&mut self) {
        if let Some(profile) = &mut self.profile {
            if let Err(error) = profile.flush() {
                log::warn!("Gallery frame capture flush failed: {error}");
            }
        }
    }

    /// One combined upload per renderer. Returns the measured upload time only
    /// on the frame that performed it.
    fn ensure_uploaded(&mut self) -> std::result::Result<Option<f64>, String> {
        if self.uploaded {
            return Ok(None);
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(None);
        };
        let begin = Instant::now();
        renderer.upload(&self.view.mesh)?;
        let elapsed = begin.elapsed().as_secs_f64() * 1000.;
        self.uploaded = true;
        self.uploads += 1;
        self.upload_ms = Some(elapsed);
        log::info!(
            "Gallery combined mesh uploaded once: {} triangles, {} B",
            self.view.stats.combined_triangles,
            self.view.stats.combined_mesh_bytes
        );
        Ok(Some(elapsed))
    }

    fn hud(&self) -> Hud {
        let mut hud = Hud::new(1000., 600.);
        let white = [0.89, 0.95, 0.97, 1.];
        let muted = [0.57, 0.72, 0.77, 1.];
        let accent = [0.52, 0.94, 0.72, 1.];
        let panel = [0.025, 0.055, 0.075, 0.87];
        let stats = &self.view.stats;
        hud.rect([16., 16., 700., 149.], panel);
        hud.text(
            30.,
            30.,
            "MATTERWEAVE DETAIL GALLERY / VIEWER ONLY",
            2.,
            white,
        );
        hud.text(
            30.,
            55.,
            &format!(
                "PRESET {} | LOD {}",
                self.view.request.preset.label().to_uppercase(),
                lod_label(self.view.request.lod).to_uppercase()
            ),
            1.25,
            accent,
        );
        hud.text(
            30.,
            75.,
            &format!(
                "{} PROTOTYPES | {} INSTANCES | {} MESH BUILDS",
                stats.prototypes, stats.instances, stats.mesh_builds
            ),
            1.25,
            white,
        );
        hud.text(
            30.,
            93.,
            &format!(
                "SOURCE CELLS {} | EXPANDED {} (COLLISION {} LIQUID {})",
                stats.unique_stored_cells,
                stats.expanded_occupied_cells,
                stats.expanded_collision_cells,
                stats.expanded_liquid_cells
            ),
            1.,
            muted,
        );
        hud.text(
            30.,
            108.,
            &format!(
                "SOURCE {} KIB | PROTOTYPE CACHE {} KIB | COMBINED {} KIB / {} TRIS",
                stats.source_bytes / 1024,
                stats.cached_mesh_bytes / 1024,
                stats.combined_mesh_bytes / 1024,
                stats.combined_triangles
            ),
            1.,
            muted,
        );
        hud.text(
            30.,
            126.,
            &format!(
                "FRAME {:.1} MS | MAIN {:.1} MS | ONE-TIME UPLOAD {} | WORLD/PHYSICS/SAVES 0",
                self.frame_ms,
                self.cpu_ms,
                match self.upload_ms {
                    Some(ms) => format!("{ms:.1} MS"),
                    None => "PENDING".into(),
                }
            ),
            1.,
            muted,
        );
        hud.text(
            30.,
            144.,
            &format!(
                "POS {:.1} {:.1} {:.1} | SEED {GALLERY_SEED} | COMBINED MESH, NOT GPU INSTANCING",
                self.camera.position.x, self.camera.position.y, self.camera.position.z
            ),
            1.,
            muted,
        );
        hud.rect([280., 400., 440., 42.], panel);
        hud.text(
            293.,
            410.,
            &self.status.chars().take(52).collect::<String>(),
            1.,
            accent,
        );
        hud.text(
            293.,
            426.,
            "NO WORLD LOADED / NO SAVE / NO COLLISION",
            1.,
            muted,
        );
        #[cfg(not(target_os = "android"))]
        hud.text(
            265.,
            588.,
            "WASD FLY | SPACE UP | SHIFT DOWN | RMB LOOK | ESC EXIT",
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
        self.draw_attempts += 1;
        let capturing = self.profile.is_some();
        let now = Instant::now();
        let cpu_busy = metrics::CpuBusySpan::begin(capturing);
        if let Some(renderer) = &mut self.renderer {
            renderer.begin_frame_diagnostics();
        }
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.frame_ms = if self.frames == 0 {
            0.
        } else {
            self.frame_ms * 0.9 + f64::from(dt) * 100.
        };
        let (motion, look) = self.controls.consume();
        self.camera.update(motion, look, dt);
        let mesh_begin = Instant::now();
        match self.ensure_uploaded() {
            Ok(uploaded) => self.frame_upload_ms = uploaded,
            Err(error) => {
                log::error!("Gallery mesh upload failed: {error}");
                eprintln!("Gallery mesh upload failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        }
        let mesh_work_ms = mesh_begin.elapsed().as_secs_f64() * 1000.;
        let hud = self.hud();
        let matrix = self
            .camera
            .view_projection(size.width as f32 / size.height as f32);
        let position = self.camera.position.to_array();
        let render_begin = capturing.then(Instant::now);
        let outcome = self.renderer.as_mut().unwrap().render_with_lighting(
            matrix,
            position,
            &hud,
            &self.lighting,
        );
        let render_ms = render_begin.map(|begin| begin.elapsed().as_secs_f64() * 1000.);
        let result = match &outcome {
            FrameResult::Presented => metrics::DrawOutcome::Presented,
            FrameResult::OutOfMemory => metrics::DrawOutcome::OutOfMemory,
            FrameResult::Retry | FrameResult::Fatal(_) => metrics::DrawOutcome::Retry,
        };
        match outcome {
            FrameResult::Fatal(error) => {
                log::error!("Gallery render failed: {error}");
                eprintln!("Gallery render failed: {error}");
                self.failed = true;
                self.renderer = None;
                event_loop.exit();
                return;
            }
            FrameResult::Presented => self.frames += 1,
            FrameResult::Retry => {}
            FrameResult::OutOfMemory => {
                self.failed = true;
                log::error!("Gallery GPU out of memory");
                eprintln!("Gallery GPU out of memory");
                event_loop.exit();
            }
        }
        let cpu_busy_ms = cpu_busy.finish();
        self.cpu_ms = now.elapsed().as_secs_f64() * 1000.;
        let epoch = self.renderer_epoch;
        let gpu = self
            .renderer
            .as_ref()
            .and_then(|r| r.gpu_timings())
            .filter(|t| self.gpu_completions.accept(epoch, t.frame_id));
        let diagnostics = self.renderer.as_ref().and_then(|r| r.draw_diagnostics());
        let shadow_casters = self
            .renderer
            .as_ref()
            .map(|r| r.shadow_caster_meshes())
            .and_then(|n| u32::try_from(n).ok());
        let mut finished = false;
        if let Some(profile) = &mut self.profile {
            // Systems this mode does not run report zero or absent, never a
            // borrowed gameplay measurement.
            let row = metrics::FrameRow {
                draw_attempt_id: self.draw_attempts,
                renderer_epoch: epoch,
                presented_count: self.frames,
                result,
                submitted_gpu_frame_id: diagnostics.and_then(|d| d.submitted_frame_id),
                completed_gpu_frame_id: gpu.map(|t| t.frame_id),
                completed_gpu_renderer_epoch: gpu.map(|_| epoch),
                draw_interval_wall_ms: Some(f64::from(dt) * 1000.),
                main_wall_ms: Some(self.cpu_ms),
                main_cpu_busy_ms: cpu_busy_ms,
                stream_request_elapsed_ms: None,
                physics_wall_ms: None,
                mesh_sync_wall_ms: Some(mesh_work_ms),
                mesh_sync_fence_wait_wall_ms: diagnostics.and_then(|d| d.upload_fence_wait_ms),
                dynamic_mesh_build_wall_ms: None,
                dynamic_upload_wall_ms: self.frame_upload_ms,
                render_wall_ms: render_ms,
                save_wall_ms: Some(0.),
                render_fence_wait_wall_ms: diagnostics.and_then(|d| d.render_fence_wait_ms),
                acquire_wall_ms: diagnostics.and_then(|d| d.acquire_ms),
                present_wall_ms: diagnostics.and_then(|d| d.present_ms),
                gpu_prev_render_ms: gpu.map(|t| t.render_ms),
                gpu_prev_shadow_ms: gpu.and_then(|t| t.shadow_ms),
                gpu_prev_shadows: gpu.map(|t| t.shadows),
                gpu_prev_shadow_map_size: gpu.map(|t| t.shadow_map_size),
                mesh_sync_fence_waits: diagnostics.and_then(|d| d.upload_fence_waits),
                physics_fixed_steps: Some(0),
                voxel_bodies_total: Some(0),
                voxel_bodies_active: Some(0),
                voxel_bodies_sleeping: Some(0),
                voxel_bodies_not_simulated: Some(0),
                chunk_mesh_uploads: Some(0),
                dynamic_mesh_builds: Some(0),
                dynamic_mesh_uploads: Some(u32::from(self.frame_upload_ms.is_some())),
                save_attempts: Some(0),
                save_failures: Some(0),
                shadow_caster_meshes: metrics::shadow_casters_for_attempt(
                    diagnostics.and_then(|d| d.submitted_frame_id),
                    shadow_casters,
                ),
            };
            match profile.record(&row) {
                Ok(true) => {}
                Ok(false) => {
                    log::info!("Gallery frame capture complete: {}", profile.path.display());
                    self.profile = None;
                    finished = true;
                }
                Err(error) => {
                    log::warn!("Gallery frame capture failed: {error}");
                    self.profile = None;
                    finished = true;
                }
            }
        }
        if finished {
            if let Some(renderer) = &mut self.renderer {
                renderer.set_diagnostics_enabled(false);
            }
        }
        self.frame_upload_ms = None;
        if self.exercise && !self.failed {
            self.run_exercise(event_loop);
        }
        if !self.failed
            && self.frame_limit.is_some_and(|limit| self.frames >= limit)
            && (!self.exercise || self.exercise_stage >= 3)
        {
            eprintln!(
                "GALLERY SMOKE PASS: preset {}; {} presented frames; {} instances; {} triangles; {}",
                self.view.request.label(),
                self.frames,
                self.view.stats.instances,
                self.view.stats.combined_triangles,
                self.renderer.as_ref().unwrap().capabilities
            );
            event_loop.exit();
        }
    }

    /// Bounded viewer-only lifecycle check: one upload per renderer, a real
    /// resize event and host window/renderer recreation. No world, save,
    /// physics or edit path is involved, so it cannot write user data.
    fn run_exercise(&mut self, event_loop: &ActiveEventLoop) {
        let result: std::result::Result<(), String> = match self.exercise_stage {
            0 if self.frames >= 2 => {
                if self.uploads == 1 && self.uploaded {
                    eprintln!(
                        "GALLERY EXERCISE: combined mesh uploaded once for {} presented frames",
                        self.frames
                    );
                    Ok(())
                } else {
                    Err(format!(
                        "expected exactly one combined upload, observed {}",
                        self.uploads
                    ))
                }
            }
            1 if self.frames >= 4 => {
                if let Some(window) = &self.window {
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(1100, 700));
                }
                eprintln!("GALLERY EXERCISE: resize requested");
                Ok(())
            }
            2 if self.frames >= 6 => {
                if !self.resize_seen {
                    if self.frames < 120 {
                        return;
                    }
                    self.failed = true;
                    eprintln!("GALLERY EXERCISE FAILED: no resize event observed");
                    event_loop.exit();
                    return;
                }
                let uploads = self.uploads;
                self.suspended(event_loop);
                self.resumed(event_loop);
                if self.renderer.is_none() {
                    Err("host renderer recreation failed".into())
                } else if self.uploaded || self.uploads != uploads {
                    Err("recreated renderer did not reset the one-time upload".into())
                } else {
                    eprintln!("GALLERY EXERCISE: host renderer/window recreated");
                    Ok(())
                }
            }
            _ => return,
        };
        match result {
            Ok(()) => self.exercise_stage += 1,
            Err(error) => {
                eprintln!("GALLERY EXERCISE FAILED: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler for GalleryApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        log::info!("Gallery lifecycle resumed");
        self.controls.clear();
        self.last_frame = Instant::now();
        self.focused = true;
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Matterweave | Detail Gallery (viewer)")
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 768)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                log::error!("Gallery window creation failed: {error}");
                eprintln!("Gallery window creation failed: {error}");
                self.failed = true;
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(Renderer::new(window.clone())) {
            Ok(renderer) => {
                log::info!("Gallery graphics: {}", renderer.capabilities);
                eprintln!("Gallery graphics: {}", renderer.capabilities);
                self.renderer_epoch += 1;
                self.gpu_completions = metrics::GpuCompletionTracker::default();
                self.renderer = Some(renderer);
                self.window = Some(window);
                // A recreated renderer holds no buffers: upload once more.
                self.uploaded = false;
                self.upload_ms = None;
                let enabled = self.profile.is_some();
                if let Some(renderer) = &mut self.renderer {
                    renderer.set_diagnostics_enabled(enabled);
                }
            }
            Err(error) => {
                log::error!("Gallery renderer initialization failed: {error}");
                eprintln!("Gallery renderer initialization failed: {error}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        log::info!("Gallery lifecycle suspended");
        self.controls.clear();
        self.focused = false;
        // No world, session or save exists to write here, by construction.
        self.flush_profile();
        self.renderer = None;
        self.window = None;
        self.uploaded = false;
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
                if self.exercise && self.exercise_stage == 2 {
                    self.resize_seen = true;
                    eprintln!(
                        "GALLERY EXERCISE: resize event {}x{}",
                        size.width, size.height
                    );
                }
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
                if event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::BrowserBack)
                {
                    event_loop.exit();
                    return;
                }
                if let PhysicalKey::Code(key) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        if key == KeyCode::Escape {
                            event_loop.exit();
                            return;
                        }
                        self.controls.keys.insert(key);
                    } else {
                        self.controls.keys.remove(&key);
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
                    // Viewer mode has no gameplay actions: any action a control
                    // zone would report is deliberately discarded.
                    TouchPhase::Started => {
                        let _ = self.controls.start(touch.id, point);
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
        self.flush_profile();
        self.renderer = None;
        self.window = None;
    }
}

impl GalleryApp {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_detail::gallery_scene;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("matterweave-gallery-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn requests_parse_presets_and_optional_lod() {
        assert_eq!(
            Request::parse("tile source").unwrap(),
            Request {
                preset: Preset::Tile,
                lod: Lod::Source
            }
        );
        assert_eq!(
            Request::parse("parasol-front\n").unwrap(),
            Request {
                preset: Preset::ParasolFront,
                lod: Lod::Source
            }
        );
        assert_eq!(
            Request::parse("# comment\n  parasol-side half  \n").unwrap(),
            Request {
                preset: Preset::ParasolSide,
                lod: Lod::Half
            }
        );
        assert_eq!(
            Request::parse("parasol-underside quarter").unwrap().lod,
            Lod::Quarter
        );
    }

    #[test]
    fn flora_preset_loads_reviewed_source_density_without_coarse_substitution() {
        let request = Request::parse("flora source").expect("reviewed flora preset");
        let view = GalleryView::build(request).unwrap();
        assert_eq!(view.request.preset.label(), "flora");
        assert_eq!(view.request.preset.seed(), 20260908);
        assert_eq!(Preset::Tile.seed(), 2026);
        assert!(GalleryView::build(Request {
            preset: request.preset,
            lod: Lod::Half,
        }).is_err());
        assert_eq!(view.stats.instances, 85); // 84 plants and the terrain tile.
        assert_eq!(view.stats.unique_stored_cells, 28_908);
        assert_eq!(view.stats.expanded_occupied_cells, 77_810);
        assert_eq!(
            view.stats.mesh_builds,
            u64::try_from(view.stats.prototypes).unwrap()
        );
        assert!(!view.mesh.indices.is_empty());
        assert!(view.eye.iter().all(|v| v.is_finite()));
        for request in ["flora half", "flora quarter"] {
            assert!(
                Request::parse(request).is_err(),
                "coarse flora quality not accepted"
            );
        }
    }

    #[test]
    fn invalid_requests_are_scoped_errors_not_defaults() {
        for text in [
            "",
            "   \n#only a comment\n",
            "world",
            "tile ultra",
            "tile source extra",
            "TILE",
        ] {
            let error = Request::parse(text).unwrap_err();
            assert!(
                matches!(error, GalleryError::Request(_)),
                "{text:?} produced {error:?}"
            );
        }
        let long = "tile ".repeat(64);
        assert!(matches!(
            Request::parse(&long).unwrap_err(),
            GalleryError::Request(_)
        ));
    }

    #[test]
    fn resolve_requires_explicit_opt_in_and_rejects_bad_markers() {
        let dir = temp_dir("resolve");
        let marker = dir.join(MARKER_FILE);
        assert_eq!(Request::resolve(&marker, None).unwrap(), None);
        assert_eq!(Request::resolve(&marker, Some("   ")).unwrap(), None);
        std::fs::write(&marker, "tile half\n").unwrap();
        assert_eq!(
            Request::resolve(&marker, None).unwrap().unwrap(),
            Request {
                preset: Preset::Tile,
                lod: Lod::Half
            }
        );
        // An explicit environment request wins over the marker file.
        assert_eq!(
            Request::resolve(&marker, Some("parasol-side"))
                .unwrap()
                .unwrap()
                .preset,
            Preset::ParasolSide
        );
        std::fs::write(&marker, vec![b'a'; 200]).unwrap();
        assert!(matches!(
            Request::resolve(&marker, None).unwrap_err(),
            GalleryError::Marker { .. }
        ));
        std::fs::write(&marker, "not-a-preset").unwrap();
        assert!(matches!(
            Request::resolve(&marker, None).unwrap_err(),
            GalleryError::Request(_)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn combined_mesh_matches_transformed_prototype_geometry() {
        let mut scene = gallery_scene(GALLERY_SEED).unwrap();
        let draws = scene.draws();
        let mut expected_vertices = 0;
        let mut expected_indices = 0;
        for draw in &draws {
            let source = scene.prototype_mesh(&draw.prototype, Lod::Source).unwrap();
            expected_vertices += source.vertices.len();
            expected_indices += source.indices.len();
        }
        let combined = combine(&mut scene, Lod::Source).unwrap();
        assert_eq!(combined.vertices.len(), expected_vertices);
        assert_eq!(combined.indices.len(), expected_indices);
        assert!(combined
            .indices
            .iter()
            .all(|index| (*index as usize) < combined.vertices.len()));
        assert_eq!(combined.indices.len() % 3, 0);
        // Every combined vertex is the prototype vertex under its instance transform.
        let mut offset = 0usize;
        let mut prototypes: Vec<String> = draws.iter().map(|d| d.prototype.clone()).collect();
        prototypes.sort();
        prototypes.dedup();
        for prototype in &prototypes {
            let placements: Vec<_> = draws
                .iter()
                .filter(|draw| &draw.prototype == prototype)
                .map(|draw| draw.transform)
                .collect();
            let source = scene.prototype_mesh(prototype, Lod::Source).unwrap();
            let sample: Vec<usize> = [0usize, source.vertices.len() / 2, source.vertices.len() - 1]
                .into_iter()
                .collect();
            for transform in placements {
                for index in &sample {
                    let local = source.vertices[*index];
                    let world = combined.vertices[offset + *index];
                    assert_eq!(world.position, transform.point_to_world(local.position));
                    assert_eq!(world.normal, transform.direction_to_world(local.normal));
                    assert_eq!(world.color, local.color);
                }
                // Indices are rebased by the running vertex offset, in range.
                let first = combined.indices[0] as usize;
                assert!(first < combined.vertices.len());
                offset += source.vertices.len();
            }
        }
        assert_eq!(offset, combined.vertices.len());
    }

    #[test]
    fn combine_uses_one_cached_mesh_per_prototype_and_leaves_source_intact() {
        let mut scene = gallery_scene(GALLERY_SEED).unwrap();
        let before = scene.counts();
        let combined = combine(&mut scene, Lod::Source).unwrap();
        let after = scene.counts();
        assert_eq!(after.mesh_builds, before.prototypes as u64);
        assert!(after.instances > after.prototypes);
        assert_eq!(after.source_bytes, before.source_bytes);
        assert_eq!(after.unique_stored_cells, before.unique_stored_cells);
        assert_eq!(
            after.expanded_occupied_cells,
            before.expanded_occupied_cells
        );
        // Re-combining reuses the cache: no prototype geometry is regenerated.
        let again = combine(&mut scene, Lod::Source).unwrap();
        assert_eq!(scene.counts().mesh_builds, after.mesh_builds);
        assert_eq!(again.vertices.len(), combined.vertices.len());
    }

    #[test]
    fn coarse_lods_shrink_geometry_and_keep_the_source_revision() {
        let mut scene = gallery_scene(GALLERY_SEED).unwrap();
        let source = combine(&mut scene, Lod::Source).unwrap();
        let half = combine(&mut scene, Lod::Half).unwrap();
        let quarter = combine(&mut scene, Lod::Quarter).unwrap();
        assert!(half.indices.len() < source.indices.len());
        assert!(quarter.indices.len() < half.indices.len());
        let revision = scene
            .prototype(TILE_PROTOTYPE)
            .expect("tile prototype")
            .revision();
        assert_eq!(
            scene
                .prototype_mesh(TILE_PROTOTYPE, Lod::Quarter)
                .unwrap()
                .revision,
            revision
        );
        // Three LODs across two prototypes: six derived meshes, none per instance.
        assert_eq!(scene.counts().mesh_builds, 6);
    }

    #[test]
    fn viewpoints_are_derived_from_scene_bounds_and_face_their_subject() {
        let scene = gallery_scene(GALLERY_SEED).unwrap();
        let tile = instance_bounds(&scene, TILE_PROTOTYPE).unwrap();
        let parasol = instance_bounds(&scene, PARASOL_PROTOTYPE).unwrap();
        for preset in [
            Preset::Tile,
            Preset::ParasolFront,
            Preset::ParasolSide,
            Preset::ParasolUnderside,
        ] {
            let (eye, yaw, pitch) = viewpoint(&scene, preset).unwrap();
            assert!(eye.iter().all(|value| value.is_finite()));
            assert!(pitch.abs() <= 1.5, "{preset:?} pitch {pitch}");
            let subject = if preset == Preset::Tile {
                tile
            } else {
                parasol
            };
            let centre = [
                (subject.min[0] + subject.max[0]) / 2.,
                (subject.min[1] + subject.max[1]) / 2.,
                (subject.min[2] + subject.max[2]) / 2.,
            ];
            // The recorded forward direction points at the subject.
            let forward = [
                yaw.sin() * pitch.cos(),
                pitch.sin(),
                yaw.cos() * pitch.cos(),
            ];
            let to_centre = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
            let length = (to_centre[0] * to_centre[0]
                + to_centre[1] * to_centre[1]
                + to_centre[2] * to_centre[2])
                .sqrt();
            let dot = (0..3)
                .map(|axis| forward[axis] * to_centre[axis] / length)
                .sum::<f32>();
            assert!(dot > 0.8, "{preset:?} looks away from its subject: {dot}");
        }
        // The underside viewpoint sits below the cap and looks upward.
        let (eye, _, pitch) = viewpoint(&scene, Preset::ParasolUnderside).unwrap();
        assert!(eye[1] < parasol.max[1]);
        assert!(pitch > 0.);
    }

    #[test]
    fn built_view_reports_truthful_separated_counters() {
        let view = GalleryView::build(Request {
            preset: Preset::ParasolFront,
            lod: Lod::Source,
        })
        .unwrap();
        let stats = view.stats;
        assert_eq!(stats.prototypes, 2);
        assert_eq!(stats.instances, 7);
        assert!(stats.unique_stored_cells < stats.expanded_occupied_cells);
        assert_eq!(
            stats.expanded_occupied_cells,
            stats.expanded_collision_cells + stats.expanded_liquid_cells
        );
        assert_eq!(stats.mesh_builds, 2);
        assert!(stats.source_bytes > 0 && stats.cached_mesh_bytes > 0);
        // Combined adapter bytes are reported separately from the scene cache.
        assert_ne!(stats.combined_mesh_bytes, stats.cached_mesh_bytes);
        assert_eq!(stats.combined_vertices, view.mesh.vertices.len());
        assert_eq!(stats.combined_triangles, view.mesh.indices.len() / 3);
    }

    /// The gallery must never read, rewrite, rename or replace user data, in the
    /// valid, corrupt and missing world cases alike.
    #[test]
    fn building_a_view_never_touches_user_world_data() {
        for (name, world) in [
            ("valid", Some(br#"{"version":1}"#.to_vec())),
            ("corrupt", Some(b"invalid world data".to_vec())),
            ("missing", None),
        ] {
            let dir = temp_dir(name);
            let save = dir.join("world.json");
            if let Some(bytes) = &world {
                std::fs::write(&save, bytes).unwrap();
            }
            let marker = dir.join(MARKER_FILE);
            std::fs::write(&marker, "tile quarter").unwrap();
            let request = Request::resolve(&marker, None).unwrap().unwrap();
            let view = GalleryView::build(request).unwrap();
            assert!(!view.mesh.vertices.is_empty());
            let mut entries: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            entries.sort();
            let expected: Vec<String> = if world.is_some() {
                vec![MARKER_FILE.into(), "world.json".into()]
            } else {
                vec![MARKER_FILE.into()]
            };
            assert_eq!(
                entries, expected,
                "{name}: gallery created or renamed files"
            );
            match &world {
                Some(bytes) => assert_eq!(&std::fs::read(&save).unwrap(), bytes),
                None => assert!(!save.exists(), "{name}: gallery created a world save"),
            }
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}
