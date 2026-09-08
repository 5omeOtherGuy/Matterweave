//! Automatic view-dependent LOD selection and derived-mesh preparation.
//!
//! This module turns a camera plus the crate's existing `Source`/`Half`/`Quarter`
//! derived representations into a per-instance LOD selection with a screen-space
//! error *estimate*, hysteresis against boundary chatter, a thin-feature/opening
//! *bias* and a retained authoritative fallback. It never changes authoritative
//! source data: world queries and collision keep using the finest source (see
//! [`crate::DetailScene::is_collidable_world_metres`]).
//!
//! ## Honesty of the error model
//!
//! Any-occupied coarsening does not admit a cheap tight geometric bound, so this
//! module deals in *estimates*, not guarantees:
//!
//! - `error_estimate_m = scale * factor` is the coarse cell size. It is a
//!   heuristic magnitude for how much a face can move, **not** a conservative
//!   Hausdorff bound: filling a long narrow cavity deletes interior faces
//!   arbitrarily far from the remaining coarse surface, and a lone diagonal
//!   protrusion moves by up to the cell diagonal, which already exceeds one edge.
//! - `dilation_fraction = 1 - occupied / (coarse_cells * factor^3)` is the
//!   fraction of the coarse solid newly filled by expansion, aggregated over the
//!   whole prototype. It *biases* thin, perforated prototypes (fronds, stems,
//!   sheets) toward finer levels, but it is global: a small deep opening inside a
//!   large solid contributes a negligible fraction and is **not** guaranteed to be
//!   preserved. Callers needing a specific opening kept must cap the level.
//! - The pixel figure is a *working estimate* for prioritization, never a proof of
//!   temporal visual quality. Approach/retreat/zoom capture review on-device is
//!   still owed before trusting a coarse level.
//!
//! The perspective projection uses the nearest depth of the instance AABB along
//! the camera forward axis (clamped to the near plane), which is the meaningful
//! depth for screen-space size; it is not Euclidean eye distance, which
//! overstates depth for off-axis instances and would coarsen them too eagerly.

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

/// Derived-loss *estimates* for one prototype at one LOD. See the module docs:
/// these are heuristics for prioritization, not guaranteed geometric bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErrorMetrics {
    /// Coarse cell size in metres (`scale * factor`); a heuristic error magnitude,
    /// not a conservative surface-displacement bound.
    pub error_estimate_m: f32,
    /// Fraction of the coarse solid newly filled by any-occupied expansion,
    /// aggregated over the whole prototype. A thin-feature bias, not a per-opening
    /// guarantee.
    pub dilation_fraction: f32,
}

impl ErrorMetrics {
    pub(crate) const SOURCE: Self = Self {
        error_estimate_m: 0.0,
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
    /// Smaller `view_height_m` is a zoom-in; projected size is depth independent.
    Orthographic { view_height_m: f32 },
}

/// View parameters needed to project a world length to screen pixels. No graphics
/// library types cross this boundary; the adapter owns the actual matrices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Eye position in world metres.
    pub eye_m: [f32; 3],
    /// View direction in world metres (need not be unit length; must be nonzero).
    /// Perspective depth is measured along this axis.
    pub forward_m: [f32; 3],
    /// Viewport height in pixels; the vertical axis defines the pixel budget.
    pub viewport_height_px: f32,
    /// Near-plane distance in metres. Depth is clamped to this for the perspective
    /// pixels-per-metre estimate so an instance at or behind the eye resolves to
    /// the finest level rather than an infinite projected estimate.
    pub near_m: f32,
    pub projection: Projection,
}

impl Camera {
    /// f64 squared norm so finite components near `f32::MAX` (e.g. `1e30`) do
    /// not overflow the intermediate to infinity. Only the downstream f32
    /// normalization/depth sees the result, and every normalized component is
    /// `forward_m[i] / norm` in `[-1, 1]`, hence finite.
    fn forward_norm_f64(&self) -> f64 {
        let f = self.forward_m;
        ((f[0] as f64) * (f[0] as f64)
            + (f[1] as f64) * (f[1] as f64)
            + (f[2] as f64) * (f[2] as f64))
            .sqrt()
    }

    pub fn validate(&self) -> Result<()> {
        let bound =
            crate::scene::MAX_SCENE_TRANSLATION_M + (MAX_CELL_COORD + 1) as f32 * MAX_SCALE_M;
        let eye_ok = self.eye_m.iter().all(|v| v.is_finite() && v.abs() <= bound);
        let forward_finite = self.forward_m.iter().all(|v| v.is_finite());
        // The f64 norm of finite f32 components is always finite; the cutoff
        // still guarantees the normalized forward is nonzero and that f32
        // depth arithmetic stays representable.
        let forward_ok = forward_finite && self.forward_norm_f64() >= 1e-4;
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
        // Accepted cameras must give a finite positive projection scale at the
        // near plane: every projected error multiplies by this scale, and an
        // infinite scale would turn the `Source` level's zero error into
        // `0 * inf = NaN`. Subnormal FOV/view-height or huge viewports can
        // overflow even though every input field is finite.
        let near_scale = self.pixels_per_metre(self.near_m);
        let scale_ok = near_scale.is_finite() && near_scale > 0.0;
        if eye_ok && forward_ok && viewport_ok && near_ok && projection_ok && scale_ok {
            Ok(())
        } else {
            Err(DetailError::InvalidPoint)
        }
    }

    /// Screen pixels spanned by one world metre at `depth_m` along the view axis.
    /// Computed in f64 and rounded once, so a subnormal FOV/view height or a
    /// huge viewport yields infinity only when the true scale leaves `f32`.
    pub fn pixels_per_metre(&self, depth_m: f32) -> f32 {
        match self.projection {
            Projection::Perspective { vertical_fov_rad } => {
                let d = f64::from(depth_m.max(self.near_m));
                let fov_rad = f64::from(vertical_fov_rad);
                (f64::from(self.viewport_height_px) / (2.0 * d * (0.5 * fov_rad).tan())) as f32
            }
            Projection::Orthographic { view_height_m } => {
                (f64::from(self.viewport_height_px) / f64::from(view_height_m)) as f32
            }
        }
    }

    /// Projected screen-space error estimate in pixels for a world-metre error at
    /// a view-axis depth.
    pub fn projected_error_px(&self, error_m: f32, depth_m: f32) -> f32 {
        error_m * self.pixels_per_metre(depth_m)
    }

    /// Nearest depth in metres of a world-space box along the camera forward axis,
    /// clamped to the near plane. This is the conservative (smallest) depth over
    /// the box, so an instance partly in front resolves by its closest point.
    pub fn nearest_depth(&self, bounds: &Bounds) -> f32 {
        let norm = self.forward_norm_f64();
        if !norm.is_finite() || norm == 0.0 {
            // Unvalidated camera: fall back to the near-plane clamp instead of
            // collapsing every depth onto it or producing NaN.
            return self.near_m;
        }
        let inv = 1.0 / norm;
        let eye: [f64; 3] = self.eye_m.map(f64::from);
        let forward: [f64; 3] = self.forward_m.map(f64::from);
        let mut min_depth = f64::INFINITY;
        for corner in 0..8u8 {
            let p: [f64; 3] = std::array::from_fn(|axis| {
                if corner >> axis & 1 == 1 {
                    f64::from(bounds.max[axis])
                } else {
                    f64::from(bounds.min[axis])
                }
            });
            // f64 dot: raw forward components near f32::MAX against scene-bound
            // offsets would overflow an f32 product before normalization.
            let depth = (0..3)
                .map(|axis| (p[axis] - eye[axis]) * forward[axis])
                .sum::<f64>()
                * inv;
            min_depth = min_depth.min(depth);
        }
        (min_depth.max(f64::from(self.near_m))) as f32
    }
}

/// Tunable selection policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodConfig {
    /// Screen-space error budget in pixels. A coarser level is chosen only while
    /// its projected error estimate stays within this budget (with hysteresis).
    pub error_budget_px: f32,
    /// Hysteresis fraction `>= 0`. The switch-to-coarser threshold is
    /// `budget / (1 + hysteresis)` and switch-to-finer is `budget * (1 + hysteresis)`,
    /// so a level is left only after the estimate clears a dead-band.
    pub hysteresis: f32,
    /// Quality cap: the coarsest level ever selected. `Lod::Source` disables LOD.
    pub max_lod: Lod,
    /// Thin-feature/opening bias: a level whose `dilation_fraction` exceeds this is
    /// never selected (nor any coarser level). Global, not a per-opening guarantee.
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
        // The hysteresis thresholds are f32 (`budget / (1 + hysteresis)` and
        // `budget * (1 + hysteresis)`); finite inputs can still multiply to
        // infinity or underflow the low threshold to zero, leaving the
        // dead-band vacuous. Reject thresholds that do not survive f32.
        let [low, high] = self.thresholds();
        let thresholds_ok = low.is_finite() && low > 0.0 && high.is_finite() && high > 0.0;
        if ok && thresholds_ok {
            Ok(())
        } else {
            Err(DetailError::InvalidScale)
        }
    }

    fn thresholds(&self) -> [f32; 2] {
        let one_plus = 1.0 + f64::from(self.hysteresis);
        [
            (f64::from(self.error_budget_px) / one_plus) as f32,
            (f64::from(self.error_budget_px) * one_plus) as f32,
        ]
    }

    /// Disable coarsening entirely: always draw the authoritative source.
    pub fn disabled() -> Self {
        Self {
            max_lod: Lod::Source,
            ..Self::default()
        }
    }
}

/// One instance's chosen LOD with the estimates behind the choice.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceLod {
    pub instance: String,
    pub prototype: String,
    pub transform: Transform,
    pub lod: Lod,
    /// Geometric error *estimate* in metres of the selected level (0 for `Source`).
    /// Not a guaranteed bound; see module docs.
    pub error_estimate_m: f32,
    /// Projected screen-space error *estimate* in pixels of the selected level.
    pub projected_error_estimate_px: f32,
    /// Nearest instance-AABB depth in metres along the camera forward axis.
    pub depth_m: f32,
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
/// cap, thin-feature bias and hysteresis relative to the previously chosen level.
///
/// `metrics[lod.index()] == None` marks a level whose derived scale is out of the
/// supported range; the walk stops there because every coarser level is also out
/// of range. Returns the chosen level and its `(error_estimate_m, projected_px)`.
pub(crate) fn choose_lod(
    previous: Lod,
    camera: &Camera,
    config: &LodConfig,
    metrics: &[Option<ErrorMetrics>; 3],
    depth_m: f32,
) -> (Lod, f32, f32) {
    let [low, high] = config.thresholds();
    let mut chosen = Lod::Source;
    let mut chosen_error_m = 0.0;
    let mut chosen_projected = camera.projected_error_px(0.0, depth_m);
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
        let projected = camera.projected_error_px(metric.error_estimate_m, depth_m);
        // Already at least this coarse: keep it until the estimate clears `high`.
        // Currently finer: only descend when the estimate is comfortably under `low`.
        let threshold = if previous >= lod { high } else { low };
        if projected <= threshold {
            chosen = lod;
            chosen_error_m = metric.error_estimate_m;
            chosen_projected = projected;
        } else {
            break;
        }
    }
    (chosen, chosen_error_m, chosen_projected)
}
