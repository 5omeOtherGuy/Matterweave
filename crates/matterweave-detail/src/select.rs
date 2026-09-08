//! Automatic view-dependent LOD selection and derived-mesh preparation.
//!
//! This module turns a camera plus the crate's existing `Source`/`Half`/`Quarter`
//! derived representations into a per-instance LOD selection with a projected
//! geometric-error bound, hysteresis against boundary chatter, an explicit
//! thin-feature/opening guard and a retained authoritative fallback. It never
//! changes authoritative source data: world queries and collision keep using the
//! finest source (see [`crate::DetailScene::is_collidable_world_metres`]).
//!
//! ## Geometric error
//!
//! Coarsening replaces a `factor^3` block of source cells with one coarse cell
//! that is occupied when *any* source cell inside it is occupied. Two derived
//! quantities describe the loss, both cheap to compute once per prototype
//! revision:
//!
//! - `error_m = scale * factor`: a conservative bound on how far the coarse
//!   surface can move from the source surface (one coarse cell). This is the
//!   length projected to screen space and compared against a pixel budget.
//! - `dilation_fraction = 1 - occupied / (coarse_cells * factor^3)`: the fraction
//!   of the coarse solid that is *newly filled* by any-occupied expansion. Thin
//!   fronds, stems and narrow openings drive this high because their coarse cells
//!   are mostly air; solid blobs keep it near zero. A prototype whose fraction
//!   exceeds the quality cap is held at the finer level, so thin features and
//!   openings keep their source silhouette instead of the visually rejected
//!   coarse flora.
//!
//! The pixel budget is a *working bound*, not a proof of temporal visual quality:
//! it controls a conservative displacement estimate, and callers still owe
//! approach/retreat/zoom capture review before trusting a coarse level on-device.

use crate::{Bounds, DetailError, Lod, Result, Transform, MAX_CELL_COORD, MAX_SCALE_M};

/// Coarseness-ascending LOD order used for selection walks.
pub(crate) const LOD_ORDER: [Lod; 3] = [Lod::Source, Lod::Half, Lod::Quarter];

impl Lod {
    /// Dense index into per-LOD tables: `Source=0`, `Half=1`, `Quarter=2`.
    pub fn index(self) -> usize {
        match self {
            Lod::Source => 0,
            Lod::Half => 1,
            Lod::Quarter => 2,
        }
    }
}

/// Derived-loss description for one prototype at one LOD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorMetrics {
    /// Conservative surface-displacement bound in metres (coarse cell size).
    pub error_m: f32,
    /// Fraction of the coarse solid newly filled by any-occupied expansion.
    pub dilation_fraction: f32,
}

impl ErrorMetrics {
    pub(crate) const SOURCE: Self = Self {
        error_m: 0.0,
        dilation_fraction: 0.0,
    };
}

/// Camera projection. Zoom is expressed by the field of view (perspective) or the
/// world-metre view height (orthographic).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Symmetric perspective by vertical field of view in radians.
    Perspective { vertical_fov_rad: f32 },
    /// Orthographic: `view_height_m` world metres map to the full viewport height.
    /// Smaller `view_height_m` is a zoom-in; projected size is distance independent.
    Orthographic { view_height_m: f32 },
}

/// View parameters needed to project a world length to screen pixels. No graphics
/// library types cross this boundary; the adapter owns the actual matrices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Eye position in world metres.
    pub eye_m: [f32; 3],
    /// Viewport height in pixels; the vertical axis defines the pixel budget.
    pub viewport_height_px: f32,
    /// Near-plane distance in metres. Distances are clamped to this for the
    /// perspective pixels-per-metre estimate so an instance at or behind the eye
    /// resolves to the finest level rather than an infinite projected error.
    pub near_m: f32,
    pub projection: Projection,
}

impl Camera {
    pub fn validate(&self) -> Result<()> {
        let bound =
            crate::scene::MAX_SCENE_TRANSLATION_M + (MAX_CELL_COORD + 1) as f32 * MAX_SCALE_M;
        let eye_ok = self.eye_m.iter().all(|v| v.is_finite() && v.abs() <= bound);
        let viewport_ok = self.viewport_height_px.is_finite() && self.viewport_height_px > 0.0;
        let near_ok = self.near_m.is_finite() && self.near_m > 0.0;
        let projection_ok = match self.projection {
            Projection::Perspective { vertical_fov_rad } => {
                vertical_fov_rad.is_finite()
                    && vertical_fov_rad > 0.0
                    && vertical_fov_rad < std::f32::consts::PI
            }
            Projection::Orthographic { view_height_m } => {
                view_height_m.is_finite() && view_height_m > 0.0
            }
        };
        if eye_ok && viewport_ok && near_ok && projection_ok {
            Ok(())
        } else {
            Err(DetailError::InvalidPoint)
        }
    }

    /// Screen pixels spanned by one world metre at `distance_m`.
    pub fn pixels_per_metre(&self, distance_m: f32) -> f32 {
        match self.projection {
            Projection::Perspective { vertical_fov_rad } => {
                let d = distance_m.max(self.near_m);
                self.viewport_height_px / (2.0 * d * (0.5 * vertical_fov_rad).tan())
            }
            Projection::Orthographic { view_height_m } => self.viewport_height_px / view_height_m,
        }
    }

    /// Projected screen-space error in pixels for a world-metre error at a distance.
    pub fn projected_error_px(&self, error_m: f32, distance_m: f32) -> f32 {
        error_m * self.pixels_per_metre(distance_m)
    }

    /// Nearest distance in metres from the eye to a world-space box; zero inside.
    pub fn distance_to_bounds(&self, bounds: &Bounds) -> f32 {
        let mut sum = 0.0f32;
        for axis in 0..3 {
            let v = self.eye_m[axis];
            let d = if v < bounds.min[axis] {
                bounds.min[axis] - v
            } else if v > bounds.max[axis] {
                v - bounds.max[axis]
            } else {
                0.0
            };
            sum += d * d;
        }
        sum.sqrt()
    }
}

/// Tunable selection policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodConfig {
    /// Screen-space error budget in pixels. A coarser level is chosen only while
    /// its projected error stays within this budget (with hysteresis).
    pub error_budget_px: f32,
    /// Hysteresis fraction `>= 0`. The switch-to-coarser threshold is
    /// `budget / (1 + hysteresis)` and switch-to-finer is `budget * (1 + hysteresis)`,
    /// so a level is left only after the error clears a dead-band, not on a boundary.
    pub hysteresis: f32,
    /// Quality cap: the coarsest level ever selected. `Lod::Source` disables LOD.
    pub max_lod: Lod,
    /// Thin-feature/opening guard: a level whose `dilation_fraction` exceeds this
    /// is never selected (nor any coarser level), preserving source silhouettes.
    pub max_dilation_fraction: f32,
    /// Optional cap on *new coarse* mesh builds per prepare call. When reached,
    /// remaining instances fall back to the authoritative `Source` mesh instead of
    /// building more derived geometry. `Source` builds are always permitted.
    pub max_coarse_builds: Option<usize>,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            error_budget_px: 2.0,
            hysteresis: 0.4,
            max_lod: Lod::Quarter,
            max_dilation_fraction: 0.45,
            max_coarse_builds: None,
        }
    }
}

impl LodConfig {
    pub fn validate(&self) -> Result<()> {
        let ok = self.error_budget_px.is_finite()
            && self.error_budget_px > 0.0
            && self.hysteresis.is_finite()
            && self.hysteresis >= 0.0
            && self.max_dilation_fraction.is_finite()
            && (0.0..=1.0).contains(&self.max_dilation_fraction);
        if ok {
            Ok(())
        } else {
            Err(DetailError::InvalidScale)
        }
    }

    /// Disable coarsening entirely: always draw the authoritative source.
    pub fn disabled() -> Self {
        Self {
            max_lod: Lod::Source,
            ..Self::default()
        }
    }
}

/// One instance's chosen LOD with the evidence behind the choice.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceLod {
    pub instance: String,
    pub prototype: String,
    pub transform: Transform,
    pub lod: Lod,
    /// Geometric error in metres of the *selected* level (0 for `Source`).
    pub error_m: f32,
    /// Projected screen-space error in pixels of the selected level.
    pub projected_error_px: f32,
    /// Nearest eye-to-instance distance in metres used for the projection.
    pub distance_m: f32,
    /// Source occupied cells of this instance's prototype. Camera independent.
    pub occupied_cells: usize,
    /// True when the selected level was downgraded from the ideal level because
    /// the desired derived mesh was unavailable (build cap or cache budget).
    pub fallback: bool,
}

/// A shared derived mesh with the instances that reference it. Adapters upload one
/// mesh per batch and issue one static draw per instance transform.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshBatch {
    pub prototype: String,
    pub lod: Lod,
    /// Vertex/index capacity bytes of the cached mesh backing this batch.
    pub mesh_bytes: usize,
    pub instances: Vec<InstanceLod>,
}

/// Result of a synchronous, budgeted preparation pass.
#[derive(Clone, Debug)]
pub struct PreparedFrame {
    /// Batches sorted by `(prototype, lod)`.
    pub batches: Vec<MeshBatch>,
    /// Every instance in stable instance-id order.
    pub selected: Vec<InstanceLod>,
    /// The source identity this frame was prepared from.
    pub source_version: crate::SceneVersion,
    /// Derived meshes built during this call (cache misses that were realized).
    pub mesh_builds_this_call: u64,
    /// Derived-mesh cache capacity bytes after preparation.
    pub cached_mesh_bytes: usize,
}

/// Chooses the coarsest acceptable level for one instance, honoring the quality
/// cap, thin-feature guard and hysteresis relative to the previously chosen level.
///
/// `metrics[lod.index()] == None` marks a level whose derived scale is out of the
/// supported range; the walk stops there because every coarser level is also out
/// of range. Returns the chosen level and its `(error_m, projected_error_px)`.
pub(crate) fn choose_lod(
    previous: Lod,
    camera: &Camera,
    config: &LodConfig,
    metrics: &[Option<ErrorMetrics>; 3],
    distance_m: f32,
) -> (Lod, f32, f32) {
    let low = config.error_budget_px / (1.0 + config.hysteresis);
    let high = config.error_budget_px * (1.0 + config.hysteresis);
    let mut chosen = Lod::Source;
    let mut chosen_error_m = 0.0;
    let mut chosen_projected = camera.projected_error_px(0.0, distance_m);
    for &lod in &LOD_ORDER[1..] {
        if lod > config.max_lod {
            break;
        }
        let Some(metric) = metrics[lod.index()] else {
            break;
        };
        if metric.dilation_fraction > config.max_dilation_fraction {
            break;
        }
        let projected = camera.projected_error_px(metric.error_m, distance_m);
        // Already at least this coarse: keep it until the error clears `high`.
        // Currently finer: only descend when the error is comfortably under `low`.
        let threshold = if previous >= lod { high } else { low };
        if projected <= threshold {
            chosen = lod;
            chosen_error_m = metric.error_m;
            chosen_projected = projected;
        } else {
            break;
        }
    }
    (chosen, chosen_error_m, chosen_projected)
}
