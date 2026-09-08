//! Shared configuration, scenes and statistics for the renderer benchmark mode.
//!
//! This module owns only CPU-side, GPU-free data: it validates a benchmark
//! profile, builds bounded voxel scenes with meaningful unit voxels, and
//! summarizes repeated wall-time samples. It selects no renderer, claims no
//! GPU timing, and performs no measurement itself; the `renderer_comparison`
//! example owns Vulkan resources and calls into here so the default 16-run
//! correctness gate and the benchmark mode share one source of scenes and
//! calibration constants.
use crate::ray_reference::MAX_CELLS;

/// Largest cubic voxel extent whose `extent + 2` cell margin still fits the
/// packed ray volume limit of `MAX_CELLS` (62 + 2 = 64, 64^3 = MAX_CELLS).
pub const MAX_SCENE_EXTENT: u32 = 62;
/// Smallest extent that still contains interior, surface and edge voxels.
pub const MIN_SCENE_EXTENT: u32 = 4;
/// Smallest benchmark image edge; the correctness gate itself runs at 128.
pub const MIN_RESOLUTION: u32 = 128;
/// Largest benchmark image edge. `2048 * 2048 * 4` readback bytes per
/// attachment is the declared host memory bound, not a device capability claim.
pub const MAX_RESOLUTION: u32 = 2048;
/// Largest total readback allocation per attachment, in bytes.
pub const MAX_READBACK_BYTES: usize = 2048 * 2048 * 4;
/// Face-edge discrepancy cap shared with the correctness gate: 0.05% of pixels.
pub const FACE_EDGE_CAP: f64 = 0.000_5;
/// Upper bound on warmup renders and measured samples per path.
pub const MAX_ITERATIONS: u32 = 512;

/// Why a requested benchmark profile was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// Argument was not `key=value`.
    Malformed(String),
    /// Key is not part of the profile surface.
    UnknownKey(String),
    /// Value did not parse as an unsigned integer.
    NotAnInteger { key: String, value: String },
    /// Value parsed but lies outside the declared bound.
    OutOfRange {
        key: String,
        value: u64,
        min: u64,
        max: u64,
    },
    /// The value is individually in range but the derived size overflows or
    /// exceeds a declared allocation bound.
    Unrepresentable(String),
    /// The same key was supplied twice with different meaning.
    Duplicate(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(a) => write!(f, "expected key=value, got `{a}`"),
            Self::UnknownKey(k) => write!(f, "unknown profile key `{k}`"),
            Self::NotAnInteger { key, value } => {
                write!(f, "`{key}` expects an unsigned integer, got `{value}`")
            }
            Self::OutOfRange {
                key,
                value,
                min,
                max,
            } => write!(f, "`{key}` = {value} is outside {min}..={max}"),
            Self::Unrepresentable(m) => write!(f, "profile is unrepresentable: {m}"),
            Self::Duplicate(k) => write!(f, "`{k}` was supplied more than once"),
        }
    }
}

/// A validated benchmark profile. Every field is bounded and every derived
/// allocation was checked for overflow before construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    resolution: u32,
    extent: u32,
    warmup: u32,
    samples: u32,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            resolution: 512,
            extent: 32,
            warmup: 3,
            samples: 30,
        }
    }
}

impl Profile {
    /// Parses `key=value` arguments over the default profile. Unknown keys,
    /// duplicates, non-integers, out-of-range values and unrepresentable
    /// derived sizes are all rejected; nothing is clamped silently.
    pub fn parse<I, S>(args: I) -> Result<Self, ProfileError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut profile = Self::default();
        let mut seen: Vec<String> = Vec::new();
        for arg in args {
            let arg = arg.as_ref();
            let (key, value) = arg
                .split_once('=')
                .ok_or_else(|| ProfileError::Malformed(arg.to_string()))?;
            let key = key.trim();
            if seen.iter().any(|k| k == key) {
                return Err(ProfileError::Duplicate(key.to_string()));
            }
            seen.push(key.to_string());
            let parsed: u64 = value.trim().parse().map_err(|_| ProfileError::NotAnInteger {
                key: key.to_string(),
                value: value.to_string(),
            })?;
            let range = match key {
                "resolution" => (u64::from(MIN_RESOLUTION), u64::from(MAX_RESOLUTION)),
                "extent" => (u64::from(MIN_SCENE_EXTENT), u64::from(MAX_SCENE_EXTENT)),
                "warmup" => (0, u64::from(MAX_ITERATIONS)),
                "samples" => (1, u64::from(MAX_ITERATIONS)),
                other => return Err(ProfileError::UnknownKey(other.to_string())),
            };
            if parsed < range.0 || parsed > range.1 {
                return Err(ProfileError::OutOfRange {
                    key: key.to_string(),
                    value: parsed,
                    min: range.0,
                    max: range.1,
                });
            }
            let parsed = parsed as u32;
            match key {
                "resolution" => profile.resolution = parsed,
                "extent" => profile.extent = parsed,
                "warmup" => profile.warmup = parsed,
                "samples" => profile.samples = parsed,
                _ => unreachable!("key range lookup already rejected unknown keys"),
            }
        }
        profile.check_derived_sizes()?;
        Ok(profile)
    }

    fn check_derived_sizes(&self) -> Result<(), ProfileError> {
        let pixels = (self.resolution as usize)
            .checked_mul(self.resolution as usize)
            .ok_or_else(|| ProfileError::Unrepresentable("pixel count overflows".into()))?;
        let readback = pixels
            .checked_mul(4)
            .ok_or_else(|| ProfileError::Unrepresentable("readback bytes overflow".into()))?;
        if readback > MAX_READBACK_BYTES {
            return Err(ProfileError::Unrepresentable(format!(
                "{readback} readback bytes exceed the {MAX_READBACK_BYTES} bound"
            )));
        }
        let margin = self.extent.checked_add(2).ok_or_else(|| {
            ProfileError::Unrepresentable("extent margin overflows".into())
        })?;
        let cells = (margin as usize)
            .checked_pow(3)
            .ok_or_else(|| ProfileError::Unrepresentable("voxel volume overflows".into()))?;
        if cells > MAX_CELLS {
            return Err(ProfileError::Unrepresentable(format!(
                "{cells} packed cells exceed the {MAX_CELLS} ray volume limit"
            )));
        }
        Ok(())
    }

    /// Square image edge in pixels.
    pub fn resolution(&self) -> u32 {
        self.resolution
    }

    /// Cubic voxel extent of every benchmark scene, in unit voxels.
    pub fn extent(&self) -> u32 {
        self.extent
    }

    /// Unmeasured renders executed before sampling starts.
    pub fn warmup(&self) -> u32 {
        self.warmup
    }

    /// Measured renders per path.
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Pixels per image; overflow was rejected at construction.
    pub fn pixels(&self) -> usize {
        (self.resolution as usize) * (self.resolution as usize)
    }
}

/// Camera description shared by every path of a scene, so the compared images
/// differ only by render path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub orthographic: bool,
    /// Orthographic half-height, or perspective vertical field of view in radians.
    pub extent: f32,
}

/// One bounded benchmark scene of meaningful unit voxels.
///
/// Every voxel is one world cell; no fine-object transform, instancing or
/// sub-voxel detail is represented here, so no fine-detail coverage is implied.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileScene {
    pub name: &'static str,
    pub cells: Vec<([i32; 3], u8)>,
    pub camera: ProfileCamera,
    /// Plane used only by the partitioned hybrid path: `x < split` renders by
    /// ray, `x > split` by raster, `x == split` is left empty so the two sets
    /// never share a face plane.
    pub split: i32,
    /// Edit applied for the edited repeat of the scene.
    pub edit: Option<([i32; 3], u8)>,
}

impl ProfileScene {
    /// Half-open cell bounds `(min, max)` over the scene including any edit.
    pub fn bounds(&self) -> ([i32; 3], [i32; 3]) {
        let mut min = [i32::MAX; 3];
        let mut max = [i32::MIN; 3];
        for (cell, _) in self.cells.iter().chain(self.edit.iter()) {
            for axis in 0..3 {
                min[axis] = min[axis].min(cell[axis]);
                max[axis] = max[axis].max(cell[axis]);
            }
        }
        (min, max)
    }

    /// Solid cells on each side of the hybrid split plane.
    pub fn split_counts(&self) -> (usize, usize) {
        let ray = self
            .cells
            .iter()
            .filter(|(c, _)| c[0] < self.split)
            .count();
        let raster = self
            .cells
            .iter()
            .filter(|(c, _)| c[0] > self.split)
            .count();
        (ray, raster)
    }
}

/// The four representative bounded scenes, all sized from `profile.extent()`.
///
/// * `dense-occlusion`: a solid cube; most voxels are hidden behind a surface.
/// * `thin-stems`: many one-voxel stems with leaf caps and wide empty space.
/// * `close-solid-edit`: a close perspective slab whose edit removes a voxel.
/// * `ortho-diagonal`: an orthographic diagonal wall with a stepped edge.
pub fn scenes(profile: &Profile) -> Vec<ProfileScene> {
    let e = profile.extent() as i32;
    let mid = e / 2;
    let far = e as f32 * 1.9;
    vec![
        dense_occlusion(e, mid, far),
        thin_stems(e, mid, far),
        close_solid_edit(e, mid),
        ortho_diagonal(e, mid),
    ]
}

fn dense_occlusion(e: i32, mid: i32, far: f32) -> ProfileScene {
    let mut cells = Vec::new();
    for x in 0..e {
        for y in 0..e {
            for z in 0..e {
                // Negative x half exercises negative chunk coordinates.
                let cell = [x - mid, y, z];
                if cell[0] == mid.min(e - mid - 1) - mid {
                    // Leave the split plane empty for the partitioned hybrid.
                }
                cells.push((cell, 3u8 + ((x + y + z) % 3) as u8));
            }
        }
    }
    cells.retain(|(c, _)| c[0] != 0);
    ProfileScene {
        name: "dense-occlusion",
        cells,
        camera: ProfileCamera {
            eye: [far, far * 0.6, far],
            target: [0.0, mid as f32, mid as f32],
            orthographic: false,
            extent: 0.9,
        },
        split: 0,
        edit: Some(([-1, e - 1, 0], 0)),
    }
}

fn thin_stems(e: i32, mid: i32, far: f32) -> ProfileScene {
    let mut cells = Vec::new();
    for x in 0..e {
        for z in 0..e {
            if x % 3 != 0 || z % 3 != 0 {
                continue;
            }
            let sx = x - mid;
            if sx == 0 {
                continue;
            }
            let height = 3 + ((x * 7 + z * 5) % (e.max(6) / 2)).max(2);
            for y in 0..height.min(e) {
                cells.push(([sx, y, z], 5u8));
            }
            let cap = height.min(e) - 1;
            for (dx, dz) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
                let leaf = [sx + dx, cap, z + dz];
                if leaf[0] != 0 {
                    cells.push((leaf, 1u8));
                }
            }
        }
    }
    ProfileScene {
        name: "thin-stems",
        cells,
        camera: ProfileCamera {
            eye: [far * 0.7, e as f32 * 0.8, far * 0.7],
            target: [0.0, e as f32 * 0.3, mid as f32],
            orthographic: false,
            extent: 1.0,
        },
        split: 0,
        edit: Some(([-3, 0, 0], 7)),
    }
}

fn close_solid_edit(e: i32, mid: i32) -> ProfileScene {
    let mut cells = Vec::new();
    let depth = 3.min(e);
    for x in 0..e {
        for y in 0..e {
            for z in 0..depth {
                let sx = x - mid;
                if sx == 0 {
                    continue;
                }
                cells.push(([sx, y, z], 4u8));
            }
        }
    }
    ProfileScene {
        name: "close-solid-edit",
        cells,
        camera: ProfileCamera {
            eye: [0.5, mid as f32, depth as f32 + 2.5],
            target: [0.5, mid as f32, 0.0],
            orthographic: false,
            extent: 1.2,
        },
        split: 0,
        edit: Some(([-2, mid, depth - 1], 0)),
    }
}

fn ortho_diagonal(e: i32, mid: i32) -> ProfileScene {
    let mut cells = Vec::new();
    for x in 0..e {
        let sx = x - mid;
        if sx == 0 {
            continue;
        }
        for y in 0..e {
            let z = (x + y / 2) % e;
            cells.push(([sx, y, z], 6u8));
            cells.push(([sx, y, (z + 1) % e], 2u8));
        }
    }
    cells.sort_unstable();
    cells.dedup_by_key(|(cell, _)| *cell);
    ProfileScene {
        name: "ortho-diagonal",
        cells,
        camera: ProfileCamera {
            eye: [e as f32 * 1.5, e as f32 * 1.2, e as f32 * 1.5],
            target: [0.0, mid as f32, mid as f32],
            orthographic: true,
            extent: e as f32 * 0.9,
        },
        split: 0,
        edit: Some(([-1, 0, 0], 3)),
    }
}

/// Realized resources of one render path, recorded from actual allocations.
///
/// `shared_bytes` covers attachments and readback buffers that every path in a
/// run needs; it is reported separately so per-path totals are not inflated by
/// storage the comparison itself requires.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathResources {
    pub voxel_cells: usize,
    pub vertices: usize,
    pub indices: usize,
    pub path_bytes: usize,
    pub shared_bytes: usize,
}

impl PathResources {
    /// Path-owned plus shared bytes.
    pub fn total_bytes(&self) -> usize {
        self.path_bytes + self.shared_bytes
    }
}

/// Wall-time samples for one measured phase. These are host wall times around a
/// synchronous submit and readback; they are NOT GPU timestamp queries.
#[derive(Debug, Clone, Default)]
pub struct Samples {
    values: Vec<f64>,
}

impl Samples {
    /// Empty collection with room for `capacity` samples.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
        }
    }

    /// Records one measured millisecond value.
    pub fn push(&mut self, ms: f64) {
        self.values.push(ms);
    }

    /// Number of measured samples; warmup renders are never recorded here.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when no sample was recorded.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Nearest-rank percentile in milliseconds, or `None` when empty.
    /// `percentile` is a fraction in `0.0..=1.0`.
    pub fn percentile(&self, percentile: f64) -> Option<f64> {
        if self.values.is_empty() || !(0.0..=1.0).contains(&percentile) {
            return None;
        }
        let mut sorted = self.values.clone();
        sorted.sort_by(f64::total_cmp);
        let rank = (percentile * sorted.len() as f64).ceil().max(1.0) as usize;
        sorted.get(rank.min(sorted.len()) - 1).copied()
    }

    /// `(p50, p95, p99)` in milliseconds, or `None` when empty.
    pub fn quantiles(&self) -> Option<(f64, f64, f64)> {
        Some((
            self.percentile(0.50)?,
            self.percentile(0.95)?,
            self.percentile(0.99)?,
        ))
    }
}

/// Whether a measured face-edge discrepancy stays within the shared cap.
///
/// `differing` counts pixels that differ between two paths of the same scene and
/// camera. The cap is the fixture threshold used by the correctness gate; it is
/// not an owner-approved image-quality guarantee.
pub fn within_face_edge_cap(differing: usize, pixels: usize) -> bool {
    if pixels == 0 {
        return false;
    }
    differing as f64 <= FACE_EDGE_CAP * pixels as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_within_every_declared_bound() {
        let profile = Profile::default();
        assert!(profile.resolution() >= MIN_RESOLUTION);
        assert!(profile.resolution() <= MAX_RESOLUTION);
        assert!(profile.extent() >= MIN_SCENE_EXTENT);
        assert!(profile.extent() <= MAX_SCENE_EXTENT);
        assert!(profile.samples() >= 1);
        assert_eq!(Profile::parse::<[&str; 0], &str>([]), Ok(profile));
    }

    #[test]
    fn parses_declared_high_resolution_and_extent() {
        let profile = Profile::parse(["resolution=1024", "extent=48", "samples=7", "warmup=2"])
            .expect("valid profile");
        assert_eq!(profile.resolution(), 1024);
        assert_eq!(profile.extent(), 48);
        assert_eq!(profile.samples(), 7);
        assert_eq!(profile.warmup(), 2);
        assert_eq!(profile.pixels(), 1024 * 1024);
    }

    #[test]
    fn rejects_malformed_unknown_duplicate_and_non_integer_arguments() {
        assert!(matches!(
            Profile::parse(["resolution"]),
            Err(ProfileError::Malformed(_))
        ));
        assert!(matches!(
            Profile::parse(["height=512"]),
            Err(ProfileError::UnknownKey(_))
        ));
        assert!(matches!(
            Profile::parse(["resolution=512", "resolution=512"]),
            Err(ProfileError::Duplicate(_))
        ));
        assert!(matches!(
            Profile::parse(["resolution=-512"]),
            Err(ProfileError::NotAnInteger { .. })
        ));
        assert!(matches!(
            Profile::parse(["extent=12.5"]),
            Err(ProfileError::NotAnInteger { .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_and_unrepresentable_sizes_without_clamping() {
        assert!(matches!(
            Profile::parse(["resolution=64"]),
            Err(ProfileError::OutOfRange { .. })
        ));
        assert!(matches!(
            Profile::parse(["resolution=4096"]),
            Err(ProfileError::OutOfRange { .. })
        ));
        assert!(matches!(
            Profile::parse([format!("extent={}", MAX_SCENE_EXTENT + 1)]),
            Err(ProfileError::OutOfRange { .. })
        ));
        assert!(matches!(
            Profile::parse(["extent=99999999999999999999"]),
            Err(ProfileError::NotAnInteger { .. })
        ));
        assert!(matches!(
            Profile::parse(["samples=0"]),
            Err(ProfileError::OutOfRange { .. })
        ));
    }

    #[test]
    fn largest_accepted_extent_still_fits_the_packed_ray_volume() {
        let profile = Profile::parse([format!("extent={MAX_SCENE_EXTENT}")]).expect("max extent");
        let margin = profile.extent() as usize + 2;
        assert!(margin.pow(3) <= MAX_CELLS);
        assert!((margin + 1).pow(3) > MAX_CELLS);
    }

    #[test]
    fn every_scene_is_bounded_split_and_editable() {
        let profile = Profile::parse(["extent=16"]).expect("valid profile");
        let scenes = scenes(&profile);
        assert_eq!(scenes.len(), 4);
        for scene in &scenes {
            assert!(!scene.cells.is_empty(), "{} has no voxels", scene.name);
            let (min, max) = scene.bounds();
            for axis in 0..3 {
                assert!(
                    max[axis] - min[axis] < profile.extent() as i32 + 2,
                    "{} exceeds the configured extent on axis {axis}",
                    scene.name
                );
            }
            let (ray, raster) = scene.split_counts();
            assert!(ray > 0 && raster > 0, "{} is not partitionable", scene.name);
            assert_eq!(ray + raster, scene.cells.len(), "{}", scene.name);
            assert!(
                !scene.cells.iter().any(|(c, _)| c[0] == scene.split),
                "{} leaves solid voxels on the split plane",
                scene.name
            );
            assert!(
                !scene.cells.iter().any(|(_, m)| *m == 0),
                "{} stores air as a solid voxel",
                scene.name
            );
            let edit = scene.edit.expect("scene defines an edit");
            assert!(
                scene.cells.iter().any(|(c, _)| *c == edit.0) || edit.1 != 0,
                "{} removes a voxel that does not exist",
                scene.name
            );
        }
        let names: Vec<_> = scenes.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "dense-occlusion",
                "thin-stems",
                "close-solid-edit",
                "ortho-diagonal"
            ]
        );
    }

    #[test]
    fn scenes_cover_dense_sparse_and_orthographic_workloads() {
        let profile = Profile::parse(["extent=16"]).expect("valid profile");
        let scenes = scenes(&profile);
        let dense = &scenes[0];
        let stems = &scenes[1];
        assert!(
            dense.cells.len() > stems.cells.len() * 4,
            "dense scene {} is not denser than the thin-stem scene {}",
            dense.cells.len(),
            stems.cells.len()
        );
        assert!(!dense.camera.orthographic);
        assert!(scenes[3].camera.orthographic);
        // The close scene must place the eye near the surface it renders.
        let close = &scenes[2];
        assert!(close.camera.eye[2] - close.bounds().1[2] as f32 <= 4.0);
    }

    #[test]
    fn scene_size_follows_the_configured_extent() {
        let small = scenes(&Profile::parse(["extent=8"]).expect("small"));
        let large = scenes(&Profile::parse(["extent=24"]).expect("large"));
        for (a, b) in small.iter().zip(large.iter()) {
            assert_eq!(a.name, b.name);
            assert!(
                b.cells.len() > a.cells.len(),
                "{} did not grow with the extent",
                a.name
            );
        }
    }

    #[test]
    fn percentiles_use_nearest_rank_and_ignore_no_sample() {
        let mut samples = Samples::with_capacity(4);
        assert!(samples.is_empty());
        assert_eq!(samples.quantiles(), None);
        for value in [4.0, 1.0, 3.0, 2.0] {
            samples.push(value);
        }
        assert_eq!(samples.len(), 4);
        let (p50, p95, p99) = samples.quantiles().expect("samples present");
        assert_eq!(p50, 2.0);
        assert_eq!(p95, 4.0);
        assert_eq!(p99, 4.0);
        assert_eq!(samples.percentile(0.0), Some(1.0));
        assert_eq!(samples.percentile(1.5), None);
    }

    #[test]
    fn quality_gate_matches_the_declared_face_edge_cap() {
        let pixels = 512 * 512;
        let allowed = (FACE_EDGE_CAP * pixels as f64) as usize;
        assert_eq!(allowed, 131);
        assert!(within_face_edge_cap(0, pixels));
        assert!(within_face_edge_cap(allowed, pixels));
        assert!(!within_face_edge_cap(allowed + 1, pixels));
        // A cap is meaningless without pixels to apply it to.
        assert!(!within_face_edge_cap(0, 0));
    }

    #[test]
    fn resource_totals_keep_shared_attachments_separate() {
        let resources = PathResources {
            voxel_cells: 100,
            vertices: 24,
            indices: 36,
            path_bytes: 1_000,
            shared_bytes: 4_000,
        };
        assert_eq!(resources.total_bytes(), 5_000);
        assert_ne!(resources.path_bytes, resources.total_bytes());
    }
}
