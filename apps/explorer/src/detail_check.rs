//! Native Vulkan automatic-detail (LOD) selection gate, not a mobile
//! performance benchmark.
//!
//! The fixture is a disposable engine scene, not a world, physics or save path:
//! controlled dense/irregular/sheet prototypes with a protected local opening,
//! plus a representative production-flora corpus built by the real exported
//! `matterweave_detail::showcase_prototype` constructors (hollow-pit fungus,
//! thin frond, reed cluster, woody shrub, jointed spire, water-lily pad),
//! placed including negative coordinates and all four yaws.
//!
//! The gate drives the production [`DetailRuntime`] lazily, the way the wetland
//! does: it warms the authoritative `Source` meshes once and realizes coarser
//! levels only while a camera selects them, under a declared per-prepare coarse
//! build bound. A cold far phase converges stationary work with that bound and
//! proves the steady state builds nothing; eligible dense geometry realizes
//! `Half`/`Quarter` meshes while every production flora instance stays at
//! `Source` under the default quality guards -- including occlusion views where
//! a thin foreground plant overlaps a realized coarse solid. Guards are never
//! relaxed to manufacture coarsening; where a flora instance's projected coarse
//! error would pass the pixel budget, the report records that the geometry guard
//! is what retained `Source`.
//!
//! Source authority is checked independently: named world-metre collision/sample
//! probes must answer identically under every camera and LOD selection, and only
//! a declared instance move or source edit may change them. The move phase
//! re-places a shrub in a rebuilt authoritative scene (same voxel revisions), so
//! placements update without a geometry rebuild; editing the moved instance then
//! forces a revision-matched derived refresh.
//!
//! Writes a local report and a compact phase/LOD HUD; Android screenshots are
//! captured externally with adb by the device lead. This is a functional gate,
//! never a performance claim.

use crate::detail_runtime::{
    lod_histogram, yaw_quarters, DetailRuntime, FrameUpdate, RESIDENT_LODS,
};
use glam::{Mat4, Vec3};
use matterweave_detail::{
    material, showcase_prototype, Camera as LodCamera, DetailScene, DetailVolume, Lod, LodConfig,
    PreparedFrame, Projection, Scale, Transform, Yaw, FLORA_FUNGUS_SCALE_M, SCALE_FINE_M,
    SCALE_TILE_M, WETLAND_LEAF_SCALE_M,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

/// The derived levels the diagnostic treats as resident candidates.
const LODS: [Lod; 3] = RESIDENT_LODS;

/// Per-prepare cap on newly realized coarse levels, mirrored from the wetland
/// production policy (`MAX_COARSE_BUILDS_PER_PREPARE`). Kept local on purpose:
/// the diagnostic must not import private production wiring.
const MAX_COARSE_BUILDS_PER_PREPARE: usize = 2;
/// Hard bound on stationary convergence prepares, so a defective policy can
/// never hang the gate. Production stops repeating once a repeat stops shrinking
/// the deferred set; this only caps runaway behavior.
const MAX_CONVERGE_PREPARES: usize = 16;

const BOULDER: &str = "boulder";
const SHEET: &str = "sheet";
const OPENING: &str = "opening";
const DENSE_CONTROL: &str = "dense_control";
const DENSE_TILE_CONTROL: &str = "dense_tile_control";
const OPENING_INSTANCE: &str = "opening_guard";
const SHEET_INSTANCE: &str = "sheet_yaw";
const NEAR_INSTANCE: &str = "solid_near";
const OCCLUSION_SOLID_INSTANCE: &str = "occlusion_solid";
const CONTROL_INSTANCE: &str = "dense_control_far";
const TILE_CONTROL_INSTANCE: &str = "dense_tile_far";

// ---------------------------------------------------------------------------
// Representative production flora
// ---------------------------------------------------------------------------

const FLORA_FUNGUS_INSTANCE: &str = "flora_fungus";
const FLORA_FROND_INSTANCE: &str = "flora_frond";
const FLORA_REED_INSTANCE: &str = "flora_reed";
const FLORA_SHRUB_INSTANCE: &str = "flora_shrub";
const FLORA_SPIRE_INSTANCE: &str = "flora_spire";
const FLORA_LILY_INSTANCE: &str = "flora_lily";

/// `(instance, production prototype id)` for the corpus. Every prototype is
/// built by [`showcase_prototype`], the exported constructor production scatter
/// uses; no local look-alike geometry is defined here.
const FLORA_INSTANCES: [(&str, &str); 6] = [
    (FLORA_FUNGUS_INSTANCE, "funnel_mushroom"),
    (FLORA_FROND_INSTANCE, "fan_frond"),
    (FLORA_REED_INSTANCE, "reed_cluster"),
    (FLORA_SHRUB_INSTANCE, "twisted_shrub"),
    (FLORA_SPIRE_INSTANCE, "horsetail"),
    (FLORA_LILY_INSTANCE, "marsh_lily"),
];

const FLORA_FUNGUS_HOME_M: [f32; 3] = [-20.0, 0.0, 2.0];
const FLORA_FROND_HOME_M: [f32; 3] = [-24.0, 0.0, 2.0];
const FLORA_REED_HOME_M: [f32; 3] = [-24.0, 0.0, -4.0];
const FLORA_SHRUB_HOME_M: [f32; 3] = [-14.0, 0.0, 2.0];
const FLORA_SHRUB_TARGET_M: [f32; 3] = [-17.0, 0.0, 2.0];
const FLORA_SPIRE_HOME_M: [f32; 3] = [14.0, 0.0, 4.0];
const FLORA_LILY_HOME_M: [f32; 3] = [14.0, 0.0, 10.0];
const OCCLUSION_SOLID_HOME_M: [f32; 3] = [-20.0, 0.0, -2.0];

/// Builds one production flora prototype through the exported constructor.
fn production_flora(id: &str) -> DetailVolume {
    showcase_prototype(id)
        .unwrap_or_else(|error| panic!("production flora constructor {id}: {error}"))
}

// ---------------------------------------------------------------------------
// Source-authoritative probes
// ---------------------------------------------------------------------------

/// Centre of the movable shrub's local root cell `[0, 0, 0]`, where the woody
/// (collision-policy) root disc is guaranteed by the constructor.
const SHRUB_HOME_ROOT_PROBE_M: [f32; 3] = [
    FLORA_SHRUB_HOME_M[0] + WETLAND_LEAF_SCALE_M * 0.5,
    WETLAND_LEAF_SCALE_M * 0.5,
    FLORA_SHRUB_HOME_M[2] + WETLAND_LEAF_SCALE_M * 0.5,
];
const SHRUB_TARGET_ROOT_PROBE_M: [f32; 3] = [
    FLORA_SHRUB_TARGET_M[0] + WETLAND_LEAF_SCALE_M * 0.5,
    WETLAND_LEAF_SCALE_M * 0.5,
    FLORA_SHRUB_TARGET_M[2] + WETLAND_LEAF_SCALE_M * 0.5,
];
/// Centre of local cell `[12, 12, 12]`, the cell the declared boulder edit fills.
const BOULDER_EDIT_CELL_PROBE_M: [f32; 3] = [12.5 * SCALE_FINE_M; 3];
/// Collidable fungus stipe cell `[0, 2, 0]` of the occlusion foreground plant.
const FUNGUS_STEM_PROBE_M: [f32; 3] = [
    FLORA_FUNGUS_HOME_M[0],
    2.5 * FLORA_FUNGUS_SCALE_M,
    FLORA_FUNGUS_HOME_M[2],
];
/// Centre of the fungus cap pit cell `[0, 14, 0]`: source-empty by construction,
/// so a coarse fill of the hollow pit must never change this answer.
const FUNGUS_PIT_PROBE_M: [f32; 3] = [
    FLORA_FUNGUS_HOME_M[0],
    14.5 * FLORA_FUNGUS_SCALE_M,
    FLORA_FUNGUS_HOME_M[2],
];
/// Core of the dense solid placed behind the occlusion foreground plant.
const OCCLUSION_SOLID_PROBE_M: [f32; 3] = [-19.9, 0.1, -1.9];

/// Named world-metre points whose authoritative collision/sample answer the gate
/// tracks. Answers may change only through the declared move/edit operations.
const PROBE_POINTS: [(&str, [f32; 3]); 7] = [
    ("solid_core", [0.1, 0.1, 0.1]),
    ("solid_edit_cell", BOULDER_EDIT_CELL_PROBE_M),
    ("occlusion_solid_core", OCCLUSION_SOLID_PROBE_M),
    ("occlusion_fungus_stem", FUNGUS_STEM_PROBE_M),
    ("occlusion_fungus_pit", FUNGUS_PIT_PROBE_M),
    ("shrub_home_root", SHRUB_HOME_ROOT_PROBE_M),
    ("shrub_target_root", SHRUB_TARGET_ROOT_PROBE_M),
];

#[derive(Clone, Debug, PartialEq)]
struct ProbeAnswer {
    collidable: bool,
    sample: Option<(String, u8)>,
}

impl ProbeAnswer {
    fn capture(scene: &DetailScene, point: [f32; 3]) -> Result<Self, String> {
        Ok(Self {
            collidable: scene
                .is_collidable_world_metres(point)
                .map_err(|error| error.to_string())?,
            sample: scene
                .sample_world_metres(point)
                .map_err(|error| error.to_string())?,
        })
    }
}

type ProbeSnapshot = BTreeMap<&'static str, ProbeAnswer>;

fn capture_probes(scene: &DetailScene) -> Result<ProbeSnapshot, String> {
    PROBE_POINTS
        .iter()
        .map(|(name, point)| Ok((*name, ProbeAnswer::capture(scene, *point)?)))
        .collect()
}

/// Names whose authoritative answer differs between two snapshots.
fn probe_delta(before: &ProbeSnapshot, after: &ProbeSnapshot) -> Vec<&'static str> {
    PROBE_POINTS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| before.get(name) != after.get(name))
        .collect()
}

// ---------------------------------------------------------------------------
// Fixture prototypes and scene
// ---------------------------------------------------------------------------

/// An irregular solid: an 8-cell cube with a carved corner notch and a small
/// external protrusion, so it is neither a plain box nor thin.
fn irregular_solid(id: &str) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).expect("fine scale"));
    for x in 0..8 {
        for y in 0..8 {
            for z in 0..8 {
                // Carve a corner notch to break box symmetry.
                if x >= 6 && y >= 6 && z >= 6 {
                    continue;
                }
                v.set([x, y, z], material::BANK_STONE).expect("solid cell");
            }
        }
    }
    // A short protrusion beyond the cube face.
    for y in 0..3 {
        v.set([8, y, 0], material::BANK_STONE)
            .expect("protrusion cell");
    }
    v
}

/// Dense control with no local voids at the fine scale: safe to coarsen under
/// the default guard, and the separate positive control the flora guard never
/// weakens.
fn dense_solid(id: &str) -> DetailVolume {
    dense_solid_at(id, SCALE_FINE_M)
}

fn dense_solid_at(id: &str, cell_m: f32) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(cell_m).expect("dense scale"));
    for x in 0..8 {
        for y in 0..8 {
            for z in 0..8 {
                v.set([x, y, z], material::BANK_STONE).expect("dense cell");
            }
        }
    }
    v
}

/// Retains the old irregular fixture and adds a through-opening that coarse
/// any-occupied cells would fill. Its source must stay selected at every phase.
fn opening_solid() -> DetailVolume {
    let mut v = irregular_solid(OPENING);
    for x in 0..8 {
        v.set([x, 4, 4], material::AIR).expect("opening cell");
    }
    v
}

/// A one-cell-thick sheet. Any-occupied coarsening fills mostly air, driving the
/// dilation fraction high, so the thin-feature guard holds it at `Source`.
fn thin_sheet(id: &str) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).expect("fine scale"));
    for x in 0..16 {
        for y in 0..32 {
            v.set([x, y, 0], material::BANK_STONE).expect("sheet cell");
        }
    }
    v
}

/// The disposable engine fixture. The shrub placement is a parameter because the
/// move phase rebuilds the authoritative scene with the same prototypes at a new
/// authoritative placement.
fn fixture_scene_with_shrub(shrub_m: [f32; 3]) -> DetailScene {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(dense_solid(BOULDER))
        .expect("add boulder");
    scene.add_prototype(thin_sheet(SHEET)).expect("add sheet");
    scene.add_prototype(opening_solid()).expect("add opening");
    scene
        .add_prototype(dense_solid(DENSE_CONTROL))
        .expect("add dense control");
    scene
        .add_prototype(dense_solid_at(DENSE_TILE_CONTROL, SCALE_TILE_M))
        .expect("add tile-scale dense control");
    for (_, species) in FLORA_INSTANCES {
        scene
            .add_prototype(production_flora(species))
            .unwrap_or_else(|_| panic!("add production flora {species}"));
    }
    scene
        .place(
            OPENING_INSTANCE,
            OPENING,
            Transform::new([1.2, 0.0, 0.0], Yaw::Deg90).expect("opening transform"),
        )
        .expect("place opening");
    scene
        .place(NEAR_INSTANCE, BOULDER, Transform::identity())
        .expect("place near boulder");
    scene
        .place(
            "solid_yaw_neg",
            BOULDER,
            Transform::new([-9.0, 0.0, -2.0], Yaw::Deg90).expect("neg transform"),
        )
        .expect("place negative yawed boulder");
    scene
        .place(
            "solid_far",
            BOULDER,
            Transform::new([0.0, 0.0, -24.0], Yaw::Deg180).expect("far transform"),
        )
        .expect("place far boulder");
    scene
        .place(
            "solid_mid",
            BOULDER,
            Transform::new([0.0, 0.0, 130.0], Yaw::Deg0).expect("mid transform"),
        )
        .expect("place mid boulder");
    scene
        .place(
            SHEET_INSTANCE,
            SHEET,
            Transform::new([5.0, 0.0, -1.0], Yaw::Deg270).expect("sheet transform"),
        )
        .expect("place sheet");
    scene
        .place(
            CONTROL_INSTANCE,
            DENSE_CONTROL,
            Transform::new([0.0, 0.0, -30.0], Yaw::Deg180).expect("control transform"),
        )
        .expect("place dense control");
    scene
        .place(
            TILE_CONTROL_INSTANCE,
            DENSE_TILE_CONTROL,
            Transform::new([10.0, 0.0, -30.0], Yaw::Deg0).expect("tile control transform"),
        )
        .expect("place tile control");
    scene
        .place(
            OCCLUSION_SOLID_INSTANCE,
            BOULDER,
            Transform::new(OCCLUSION_SOLID_HOME_M, Yaw::Deg90).expect("occlusion solid transform"),
        )
        .expect("place occlusion solid");
    scene
        .place(
            FLORA_FUNGUS_INSTANCE,
            "funnel_mushroom",
            Transform::new(FLORA_FUNGUS_HOME_M, Yaw::Deg0).expect("fungus transform"),
        )
        .expect("place fungus");
    scene
        .place(
            FLORA_FROND_INSTANCE,
            "fan_frond",
            Transform::new(FLORA_FROND_HOME_M, Yaw::Deg180).expect("frond transform"),
        )
        .expect("place frond");
    scene
        .place(
            FLORA_REED_INSTANCE,
            "reed_cluster",
            Transform::new(FLORA_REED_HOME_M, Yaw::Deg270).expect("reed transform"),
        )
        .expect("place reed");
    scene
        .place(
            FLORA_SHRUB_INSTANCE,
            "twisted_shrub",
            Transform::new(shrub_m, Yaw::Deg0).expect("shrub transform"),
        )
        .expect("place shrub");
    scene
        .place(
            FLORA_SPIRE_INSTANCE,
            "horsetail",
            Transform::new(FLORA_SPIRE_HOME_M, Yaw::Deg90).expect("spire transform"),
        )
        .expect("place spire");
    scene
        .place(
            FLORA_LILY_INSTANCE,
            "marsh_lily",
            Transform::new(FLORA_LILY_HOME_M, Yaw::Deg0).expect("lily transform"),
        )
        .expect("place lily");
    scene
}

/// The canonical fixture, with the movable shrub at its home placement.
pub fn fixture_scene() -> DetailScene {
    fixture_scene_with_shrub(FLORA_SHRUB_HOME_M)
}

fn histogram_label(counts: &BTreeMap<Lod, usize>) -> String {
    LODS.iter()
        .map(|lod| format!("{lod:?}={}", counts.get(lod).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn deferred_count(frame: &PreparedFrame) -> usize {
    frame.selected.iter().filter(|item| item.fallback).count()
}

/// True when `Half` of a prototype at `prototype_scale_m` would pass the
/// descending (fresh-selection) hysteresis threshold at `depth_m`. Used to show
/// that a retained `Source` is the geometry guard's decision, not distance.
fn coarse_would_pass_pixels(
    camera: &LodCamera,
    prototype_scale_m: f32,
    depth_m: f32,
    config: &LodConfig,
) -> bool {
    let low = config.error_budget_px / (1.0 + config.hysteresis);
    camera.projected_error_px(prototype_scale_m * 2.0, depth_m) <= low
}

/// A camera phase: eye, look target, projection and selection policy.
#[derive(Clone, Copy)]
pub struct PhasePlan {
    pub name: &'static str,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub projection: Projection,
    pub config: LodConfig,
    /// Stationary convergence phase: keep preparing the identical view until
    /// the deferred set reaches zero.
    pub converge: bool,
    /// Rebuilds the authoritative scene with the shrub at its moved placement.
    pub move_flora: bool,
    /// Declared source edit of the moved shrub's root cell.
    pub edit_moved_flora: bool,
    /// Declared source edit of the boulder prototype.
    pub edit: bool,
    /// Occlusion pair assertions for this frame.
    pub occlusion: bool,
    /// True for the phase that drops and recreates the renderer.
    pub recreate: bool,
}

fn plan(
    name: &'static str,
    eye: [f32; 3],
    target: [f32; 3],
    projection: Projection,
    config: LodConfig,
) -> PhasePlan {
    PhasePlan {
        name,
        eye,
        target,
        projection,
        config,
        converge: false,
        move_flora: false,
        edit_moved_flora: false,
        edit: false,
        occlusion: false,
        recreate: false,
    }
}

const FOV: f32 = std::f32::consts::PI / 3.0;

/// The ordered phases. The first phase exercises cold bounded lazy realization
/// and stationary convergence; the middle phases cover approach/retreat,
/// perspective FOV zoom, orthographic zoom, a production-flora occlusion view
/// and the negative/yawed flora cluster; the last phases exercise a declared
/// instance move, two declared source edits, a resident zero-build budget and a
/// renderer lifecycle recreation.
pub fn phases() -> Vec<PhasePlan> {
    let origin = [0.2, 0.2, 0.0];
    let default = LodConfig::default();
    let perspective = Projection::Perspective {
        vertical_fov_rad: FOV,
    };
    vec![
        // Cold resident pool (Source only) plus the production coarse build
        // cap: the first prepare realizes at most the declared bound and defers
        // the rest; identical stationary prepares converge to zero deferred and
        // a zero-build steady state.
        PhasePlan {
            converge: true,
            ..plan(
                "cold-far-bounded-convergence",
                [0.2, 0.2, 220.0],
                origin,
                perspective,
                LodConfig {
                    max_coarse_builds: Some(MAX_COARSE_BUILDS_PER_PREPARE),
                    ..default
                },
            )
        },
        // Approach resolves the dense control to Source.
        plan(
            "approach-near-perspective",
            [0.2, 0.2, 2.0],
            origin,
            perspective,
            default,
        ),
        // Retreat coarsens it. This camera also makes every production flora
        // instance pixel-eligible for a coarse level while the default guards
        // retain Source.
        plan(
            "retreat-far-perspective",
            [0.2, 0.2, 220.0],
            origin,
            perspective,
            default,
        ),
        plan(
            "mid-perspective",
            [0.2, 0.2, 70.0],
            origin,
            perspective,
            default,
        ),
        plan(
            "perspective-fov-zoom-in",
            [0.2, 0.2, 70.0],
            origin,
            Projection::Perspective {
                vertical_fov_rad: 0.03,
            },
            default,
        ),
        plan(
            "orthographic-zoomed-out",
            [0.2, 0.2, 30.0],
            origin,
            Projection::Orthographic {
                view_height_m: 200.0,
            },
            default,
        ),
        plan(
            "orthographic-zoomed-in",
            [0.2, 0.2, 30.0],
            origin,
            Projection::Orthographic { view_height_m: 4.0 },
            default,
        ),
        // Foreground thin fungus over a rear dense solid along the view axis.
        PhasePlan {
            occlusion: true,
            ..plan(
                "occlusion-foreground-fungus-over-solid",
                [-20.0, 0.5, 95.0],
                [-20.0, 0.15, 0.0],
                perspective,
                default,
            )
        },
        // Negative-coordinate, all-yaw flora cluster.
        plan(
            "flora-cluster-negative-yaw",
            [-12.0, 4.0, 24.0],
            [-22.0, 0.8, 0.0],
            perspective,
            default,
        ),
        // After convergence and the camera phases every selected coarse level is
        // resident, so a zero cap is a truthful stationarity guarantee: nothing
        // is newly built and nothing falls back.
        plan(
            "resident-zero-build-budget",
            [0.2, 0.2, 220.0],
            origin,
            perspective,
            LodConfig {
                max_coarse_builds: Some(0),
                ..default
            },
        ),
        // The authoritative scene is rebuilt with the same prototypes at a new
        // shrub placement: the source version advances while voxel revisions do
        // not, so placements update without a geometry rebuild.
        PhasePlan {
            move_flora: true,
            ..plan(
                "move-flora-instance",
                [-16.6, 2.0, 9.0],
                [-17.0, 0.8, 2.0],
                perspective,
                default,
            )
        },
        // Editing the moved instance advances its revision and forces a
        // revision-matched derived refresh.
        PhasePlan {
            edit_moved_flora: true,
            ..plan(
                "move-edit-invalidation",
                [-16.6, 2.0, 9.0],
                [-17.0, 0.8, 2.0],
                perspective,
                default,
            )
        },
        // Existing prototype edit invalidation.
        PhasePlan {
            edit: true,
            ..plan(
                "edit-invalidate-rebuild",
                [0.2, 0.2, 70.0],
                origin,
                perspective,
                default,
            )
        },
        // Zero extent + renderer recreation.
        PhasePlan {
            recreate: true,
            ..plan(
                "lifecycle-recreate",
                [0.2, 0.2, 2.0],
                origin,
                perspective,
                default,
            )
        },
    ]
}

impl PhasePlan {
    fn lod_camera(&self, viewport_height_px: f32) -> LodCamera {
        LodCamera {
            eye_m: self.eye,
            forward_m: [
                self.target[0] - self.eye[0],
                self.target[1] - self.eye[1],
                self.target[2] - self.eye[2],
            ],
            viewport_height_px,
            near_m: 0.1,
            projection: self.projection,
        }
    }

    fn projection_label(&self) -> &'static str {
        match self.projection {
            Projection::Perspective { .. } => "perspective",
            Projection::Orthographic { .. } => "orthographic",
        }
    }

    fn view_projection(&self, aspect: f32) -> [[f32; 4]; 4] {
        let eye = Vec3::from_array(self.eye);
        let target = Vec3::from_array(self.target);
        let look = Mat4::look_at_rh(eye, target, Vec3::Y);
        let proj = match self.projection {
            Projection::Perspective { vertical_fov_rad } => {
                Mat4::perspective_rh(vertical_fov_rad, aspect, 0.1, 2000.0)
            }
            Projection::Orthographic { view_height_m } => {
                let half_h = view_height_m / 2.0;
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, 0.1, 2000.0)
            }
        };
        (proj * look).to_cols_array_2d()
    }
}

/// Bounded stationary convergence over one identical view. Returns the final
/// update, the number of prepares and the realized build counts.
struct ConvergenceOutcome {
    iterations: usize,
    total_builds: u64,
    max_builds: usize,
    update: crate::detail_runtime::FrameUpdate,
}

#[cfg(target_os = "android")]
const PHASE_FRAMES: u32 = 120;
#[cfg(not(target_os = "android"))]
const PHASE_FRAMES: u32 = 6;

/// Prepares identical stationary frames until the deferred set is empty, then
/// proves the steady state realizes nothing. The per-prepare coarse build count
/// may never exceed the declared cap.
fn converge_bounded(
    runtime: &mut DetailRuntime,
    scene: &mut DetailScene,
    camera: &LodCamera,
    config: &LodConfig,
    require_deferral: bool,
) -> Result<ConvergenceOutcome, String> {
    let cap = config
        .max_coarse_builds
        .ok_or("convergence requires a declared coarse build cap")?;
    let first = runtime.prepare(scene, camera, config)?;
    let mut deferred = deferred_count(&first.frame);
    if deferred == 0 {
        if !require_deferral {
            // Re-entry (a surface event changed the viewport or the pool was
            // re-warmed): the view is already fully resident, so there is
            // nothing to converge and the one-shot lazy-path evidence is not
            // required again.
            return Ok(ConvergenceOutcome {
                iterations: 1,
                total_builds: first.frame.mesh_builds_this_call,
                max_builds: first.frame.mesh_builds_this_call as usize,
                update: first,
            });
        }
        return Err(format!(
            "bounded-convergence phase started fully resident; the lazy path was not exercised (selected: {})",
            first
                .frame
                .selected
                .iter()
                .map(|s| format!("{}:{:?}:fb{}", s.instance, s.lod, s.fallback))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    let mut iterations = 1usize;
    let mut total_builds = first.frame.mesh_builds_this_call;
    let mut max_builds = total_builds as usize;
    let mut last = first;
    while deferred > 0 && iterations < MAX_CONVERGE_PREPARES {
        let next = runtime.prepare(scene, camera, config)?;
        let next_deferred = deferred_count(&next.frame);
        if next_deferred >= deferred {
            return Err(format!(
                "stationary convergence stopped making progress: deferred {deferred} -> {next_deferred}"
            ));
        }
        max_builds = max_builds.max(next.frame.mesh_builds_this_call as usize);
        total_builds += next.frame.mesh_builds_this_call;
        deferred = next_deferred;
        iterations += 1;
        last = next;
    }
    if deferred != 0 {
        return Err(format!(
            "stationary convergence left {deferred} instance(s) deferred after {iterations} prepares"
        ));
    }
    if max_builds > cap {
        return Err(format!(
            "stationary convergence built {max_builds} meshes in one prepare, cap is {cap}"
        ));
    }
    // The converged view must now be stationary: identical input, no builds and
    // no geometry change.
    let steady = runtime.prepare(scene, camera, config)?;
    if steady.geometry_changed || steady.frame.mesh_builds_this_call != 0 {
        return Err(format!(
            "converged frame still did work: geometry_changed={} builds={}",
            steady.geometry_changed, steady.frame.mesh_builds_this_call
        ));
    }
    Ok(ConvergenceOutcome {
        iterations,
        total_builds,
        max_builds,
        update: last,
    })
}

/// One-shot authoritative mutations and convergence-protocol state. A surface
/// event (resize) or lifecycle recreate re-enters the same phase against the
/// live viewport; it must re-prepare and re-upload but never replay a mutation
/// the authoritative scene already records.
#[derive(Default)]
struct PhaseState {
    /// The bounded lazy-convergence protocol has completed once.
    converged: bool,
    moved: bool,
    edit_moved: bool,
    edited: bool,
    /// A declared source edit whose derived-geometry invalidation has not yet
    /// been observed in a prepared frame.
    edit_invalidation_pending: bool,
}

/// A prepared phase plus its bounded-convergence accounting.
struct PhaseOutcome {
    update: FrameUpdate,
    iterations: usize,
    total_builds: u64,
    max_builds: usize,
    /// True when several prepares realized geometry; the renderer must
    /// re-install the pool rather than update instance data alone.
    force_replace: bool,
}

/// Applies the phase's declared authoritative mutation exactly once and checks
/// its declared source-probe delta. `probes` is updated in place so every later
/// phase can assert camera/LOD independence of the authoritative answers.
fn apply_phase_mutation(
    scene: &mut DetailScene,
    plan: &PhasePlan,
    state: &mut PhaseState,
    probes: &mut ProbeSnapshot,
) -> Result<(), String> {
    if plan.move_flora && !state.moved {
        let before = capture_probes(scene)?;
        let version = scene.source_version();
        *scene = fixture_scene_with_shrub(FLORA_SHRUB_TARGET_M);
        if scene.source_version() == version {
            return Err("flora move did not advance the authoritative scene version".into());
        }
        let after = capture_probes(scene)?;
        let delta = probe_delta(&before, &after);
        if delta != ["shrub_home_root", "shrub_target_root"] {
            return Err(format!(
                "instance move changed unexpected source probes: {delta:?}"
            ));
        }
        let target = after
            .get("shrub_target_root")
            .ok_or("moved shrub probe missing")?;
        if after["shrub_home_root"].collidable
            || !target.collidable
            || target.sample.as_ref().map(|(id, _)| id.as_str()) != Some(FLORA_SHRUB_INSTANCE)
        {
            return Err("instance move did not move the authoritative source footprint".into());
        }
        state.moved = true;
        *probes = after;
    }

    if plan.edit_moved_flora && !state.edit_moved {
        let before = capture_probes(scene)?;
        if !scene
            .edit_instance(FLORA_SHRUB_INSTANCE, [0, 0, 0], material::AIR)
            .map_err(|e| format!("edit_instance: {e}"))?
        {
            return Err("edit of the moved flora changed no source cell".into());
        }
        let after = capture_probes(scene)?;
        let delta = probe_delta(&before, &after);
        if delta != ["shrub_target_root"] {
            return Err(format!(
                "moved-flora edit changed unexpected source probes: {delta:?}"
            ));
        }
        state.edit_moved = true;
        state.edit_invalidation_pending = true;
        *probes = after;
    }

    if plan.edit && !state.edited {
        let before = capture_probes(scene)?;
        let version = scene.source_version();
        scene
            .edit_prototype(BOULDER, [12, 12, 12], material::BANK_STONE)
            .map_err(|e| format!("edit_prototype: {e}"))?;
        if scene.source_version() == version {
            return Err("edit did not advance the source version".into());
        }
        let after = capture_probes(scene)?;
        let delta = probe_delta(&before, &after);
        if delta != ["solid_edit_cell"] {
            return Err(format!(
                "boulder edit changed unexpected source probes: {delta:?}"
            ));
        }
        state.edited = true;
        state.edit_invalidation_pending = true;
        *probes = after;
    }
    Ok(())
}

/// Prepares the phase's live selection for `viewport_height_px`. Idempotent
/// across re-entry: a convergence phase re-runs the bounded protocol for the
/// current viewport without re-requiring lazy-path evidence once it has been
/// recorded, and a source edit whose invalidation was already observed does not
/// demand `geometry_changed` again. The packed instances handed to the renderer
/// are validated against the drawable part of the selection here.
fn prepare_phase(
    scene: &mut DetailScene,
    runtime: &mut DetailRuntime,
    plan: &PhasePlan,
    viewport_height_px: f32,
    probes: &ProbeSnapshot,
    state: &mut PhaseState,
) -> Result<PhaseOutcome, String> {
    let camera = plan.lod_camera(viewport_height_px);
    let (update, iterations, total_builds, max_builds, force_replace) = if plan.converge {
        let outcome = converge_bounded(runtime, scene, &camera, &plan.config, !state.converged)?;
        state.converged = true;
        (
            outcome.update,
            outcome.iterations,
            outcome.total_builds,
            outcome.max_builds,
            true,
        )
    } else {
        let update = runtime.prepare(scene, &camera, &plan.config)?;
        let builds = update.frame.mesh_builds_this_call;
        (update, 1usize, builds, builds as usize, false)
    };
    if update.frame.source_version != scene.source_version() {
        return Err("prepared frame source version disagrees with the scene".into());
    }
    validate_packed_instances(scene, runtime, &update)?;

    if plan.move_flora {
        if update.geometry_changed {
            return Err(
                "instance move rebuilt resident geometry; only placements should change".into(),
            );
        }
        let moved = update
            .frame
            .selected
            .iter()
            .find(|s| s.instance == FLORA_SHRUB_INSTANCE)
            .ok_or("moved flora instance missing from the prepared frame")?;
        if moved.transform.translation_m != FLORA_SHRUB_TARGET_M {
            return Err(format!(
                "moved instance kept translation {:?}",
                moved.transform.translation_m
            ));
        }
        let index = runtime
            .instance_index(&moved.prototype, moved.lod)
            .ok_or("moved flora selected level not resident")?;
        if !update
            .instances
            .iter()
            .any(|packed| packed.prototype == index && packed.translation == FLORA_SHRUB_TARGET_M)
        {
            return Err(
                "moved flora packed instance does not carry the target translation at the resident pool index"
                    .into(),
            );
        }
    }

    if state.edit_invalidation_pending {
        if !update.geometry_changed {
            return Err("source edit did not invalidate derived resident geometry".into());
        }
        state.edit_invalidation_pending = false;
    }

    if capture_probes(scene)? != *probes {
        return Err("authoritative probe answers changed during prepare".into());
    }
    Ok(PhaseOutcome {
        update,
        iterations,
        total_builds,
        max_builds,
        force_replace,
    })
}

/// Validates the packed instances a prepare handed to the renderer against the
/// drawable part of the frame selection. Every selected instance whose
/// prototype has occupied cells contributes exactly one packed record, in
/// order, carrying the resident mesh-pool index, world translation and
/// quarter-turn yaw of that selected `(prototype, Lod)`. Selections with no
/// occupied cells draw nothing and are omitted, matching
/// [`DetailRuntime::instances_for_frame`].
fn validate_packed_instances(
    scene: &DetailScene,
    runtime: &DetailRuntime,
    update: &FrameUpdate,
) -> Result<(), String> {
    let mut drawable = Vec::with_capacity(update.frame.selected.len());
    for selected in &update.frame.selected {
        let prototype = scene
            .prototype(&selected.prototype)
            .ok_or_else(|| format!("selected prototype {} vanished", selected.prototype))?;
        if prototype.occupied_cells() > 0 {
            drawable.push(selected);
        }
    }
    if drawable.len() != update.instances.len() {
        return Err(format!(
            "prepared {} packed instances for {} drawable selections",
            update.instances.len(),
            drawable.len()
        ));
    }
    for (selected, packed) in drawable.iter().zip(&update.instances) {
        let expected_index = runtime
            .instance_index(&selected.prototype, selected.lod)
            .ok_or_else(|| {
                format!(
                    "{} {:?} selected without resident geometry",
                    selected.prototype, selected.lod
                )
            })?;
        if packed.prototype != expected_index {
            return Err(format!(
                "{} {:?} packed mesh-pool index {} != resident index {expected_index}",
                selected.prototype, selected.lod, packed.prototype
            ));
        }
        if packed.translation != selected.transform.translation_m {
            return Err(format!(
                "{} packed translation {:?} != selected {:?}",
                selected.prototype, packed.translation, selected.transform.translation_m
            ));
        }
        let expected_yaw = yaw_quarters(selected.transform.yaw);
        if packed.yaw_quarters != expected_yaw {
            return Err(format!(
                "{} packed yaw quarters {} != selected {expected_yaw}",
                selected.prototype, packed.yaw_quarters
            ));
        }
    }
    Ok(())
}

/// Fixture capability check on a disposable source fork: every nonempty
/// prototype must realize all three derived levels eagerly, so the lazy run can
/// never confuse an unbuildable fixture level with a retained quality guard.
/// This is the only eager preload the gate performs; the resident pool it
/// renders from is realized lazily.
fn check_fixture_levels(scene: &DetailScene) -> Result<usize, String> {
    let mut fork = scene.fork_source();
    let runtime = DetailRuntime::preload(&mut fork)?;
    let expected = fork.prototype_ids().len() * LODS.len();
    if runtime.meshes().len() != expected {
        return Err(format!(
            "fixture preloaded {} of {expected} prototype levels",
            runtime.meshes().len()
        ));
    }
    Ok(expected)
}

pub(crate) struct DetailCheck {
    report_path: std::path::PathBuf,
    report: Vec<String>,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    scene: DetailScene,
    runtime: Option<DetailRuntime>,
    plans: Vec<PhasePlan>,
    lighting: LightingSettings,
    /// Eager fixture capability result: resident levels buildable on a fork.
    fixture_levels: usize,
    /// Current expected authoritative probe answers; declared move/edit
    /// operations update it, every other phase must leave it identical.
    probes: ProbeSnapshot,
    lods_seen: BTreeSet<Lod>,
    near_source_seen: bool,
    near_coarsened_seen: bool,
    /// A flora instance faced a pixel-eligible coarse choice and stayed Source,
    /// so the retention was the geometry guard's decision.
    flora_guard_decided: bool,
    /// An eligible dense instance realized a non-empty coarse level.
    coarse_realized_seen: bool,
    /// One-shot mutation and convergence state, kept separate from viewport and
    /// GPU re-preparation so surface/lifecycle events never replay a mutation.
    phase_state: PhaseState,
    recreated: bool,
    frame: u32,
    prepared: Option<u32>,
    hud_lines: Vec<String>,
    finished: bool,
}

impl DetailCheck {
    pub fn new(report_path: std::path::PathBuf) -> Self {
        let scene = fixture_scene();
        let probes = capture_probes(&scene).expect("fixture probe capture");
        let fixture_levels = check_fixture_levels(&scene).expect("fixture levels buildable");
        Self {
            report_path,
            report: Vec::new(),
            renderer: None,
            window: None,
            scene,
            runtime: None,
            plans: phases(),
            lighting: LightingSettings {
                sun: Sun {
                    direction_to_sun: [0.35, 1.0, 0.25],
                    intensity: 0.9,
                },
                ..Default::default()
            },
            fixture_levels,
            probes,
            lods_seen: BTreeSet::new(),
            near_source_seen: false,
            near_coarsened_seen: false,
            flora_guard_decided: false,
            coarse_realized_seen: false,
            phase_state: PhaseState::default(),
            recreated: false,
            frame: 0,
            prepared: None,
            hud_lines: Vec::new(),
            finished: false,
        }
    }

    fn record(&mut self, entry: String) {
        log::info!("{entry}");
        #[cfg(not(target_os = "android"))]
        eprintln!("{entry}");
        self.report.push(entry);
        std::fs::write(&self.report_path, self.report.join("\n") + "\n")
            .expect("write detail check report");
    }

    fn viewport_height(&self) -> f32 {
        self.window
            .as_ref()
            .map(|w| w.inner_size().height.max(1) as f32)
            .unwrap_or(480.0)
    }

    /// Warms the authoritative `Source` mesh of every nonempty prototype into a
    /// fresh resident pool, mirroring production `warm_source`, and installs the
    /// source-only selection. One `replace_static_scene` per renderer; later
    /// frames replace only when resident geometry changed.
    fn install_static_scene(&mut self, phase: usize) -> Result<(), String> {
        let mut runtime = DetailRuntime::new();
        let plan = self.plans[phase];
        let source_only = LodConfig {
            max_lod: Lod::Source,
            ..plan.config
        };
        let camera = plan.lod_camera(self.viewport_height());
        let update = runtime.prepare(&mut self.scene, &camera, &source_only)?;
        let renderer = self
            .renderer
            .as_mut()
            .ok_or("no renderer for static scene")?;
        renderer
            .replace_static_scene(runtime.meshes(), &update.instances)
            .map_err(|e| format!("replace_static_scene: {e}"))?;
        self.runtime = Some(runtime);
        Ok(())
    }

    /// Installs one prepared selection in the renderer, replacing the static
    /// scene when resident geometry changed or the caller did several prepares.
    fn install(
        &mut self,
        update: &crate::detail_runtime::FrameUpdate,
        force_replace: bool,
    ) -> Result<(), String> {
        let runtime = self.runtime.as_ref().ok_or("no runtime for install")?;
        let renderer = self.renderer.as_mut().ok_or("no renderer for install")?;
        if force_replace || update.geometry_changed {
            renderer
                .replace_static_scene(runtime.meshes(), &update.instances)
                .map(|_| ())
                .map_err(|e| format!("replace_static_scene: {e}"))
        } else {
            renderer
                .update_static_instances(&update.instances)
                .map(|_| ())
                .map_err(|e| format!("update_static_instances: {e}"))
        }
    }

    fn apply_phase(&mut self, phase: usize) -> Result<String, String> {
        let plan = self.plans[phase];

        // One-shot authoritative mutations and their declared probe deltas.
        // A surface event or renderer recreation re-enters this phase to
        // re-prepare the live viewport, but must not replay these.
        apply_phase_mutation(
            &mut self.scene,
            &plan,
            &mut self.phase_state,
            &mut self.probes,
        )?;

        let viewport = self.viewport_height();
        let runtime = self.runtime.as_mut().ok_or("no runtime")?;
        let outcome = prepare_phase(
            &mut self.scene,
            runtime,
            &plan,
            viewport,
            &self.probes,
            &mut self.phase_state,
        )?;
        let update = outcome.update;
        let iterations = outcome.iterations;
        let total_builds = outcome.total_builds;
        let max_builds = outcome.max_builds;
        self.install(&update, outcome.force_replace)?;

        let check = self.check_frame(&plan, &update)?;
        let histogram = lod_histogram(&update.frame);
        let (selections, runtime_meshes) = {
            let runtime = self.runtime.as_ref().ok_or("no runtime after prepare")?;
            let selections = update
                .frame
                .selected
                .iter()
                .map(|s| {
                    format!(
                        "{}:{}:{:?}:r{}",
                        s.instance,
                        s.prototype,
                        s.lod,
                        runtime.revision(&s.prototype, s.lod).unwrap_or(0)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            (selections, runtime.meshes().len())
        };
        self.hud_lines = vec![
            format!("P{phase} {}", plan.name),
            format!(
                "{} {} def{} b{} it{}",
                plan.projection_label(),
                histogram_label(&histogram),
                check.deferred,
                update.frame.mesh_builds_this_call,
                iterations
            ),
            format!(
                "flora Source {}/{} guard_px_ok {} move{} edit{}",
                FLORA_INSTANCES.len(),
                FLORA_INSTANCES.len(),
                check.flora_pixel_eligible,
                self.phase_state.moved,
                self.phase_state.edited || self.phase_state.edit_moved,
            ),
        ];
        let counts = self.scene.counts();
        Ok(format!(
            "phase={phase} {} proj={} lods[{}] instances={} fallback={} builds_this_call={} geometry_changed={} coarse_realized={} converge_iters={} max_prepare_builds={} total_converge_builds={} flora_source={}/{} flora_pixel_eligible={} sheet=Source opening=Source selected=[{selections}] resident_prototypes={} source_cells={} source_bytes={}",
            plan.name,
            plan.projection_label(),
            histogram_label(&histogram),
            update.instances.len(),
            check.deferred,
            update.frame.mesh_builds_this_call,
            update.geometry_changed,
            check.coarse_realized,
            iterations,
            max_builds,
            total_builds,
            FLORA_INSTANCES.len(),
            FLORA_INSTANCES.len(),
            check.flora_pixel_eligible,
            runtime_meshes,
            counts.unique_stored_cells,
            counts.source_bytes,
        ))
    }

    /// Per-phase contract checks over one prepared and installed frame.
    fn check_frame(
        &mut self,
        plan: &PhasePlan,
        update: &crate::detail_runtime::FrameUpdate,
    ) -> Result<PhaseCheck, String> {
        let frame = &update.frame;
        let camera = plan.lod_camera(self.viewport_height());
        let scene = &self.scene;
        let runtime = self.runtime.as_ref().ok_or("no runtime")?;

        // Trace every selected resident revision against the authoritative source.
        for item in &frame.selected {
            let source_rev = scene
                .prototype(&item.prototype)
                .ok_or("selected prototype vanished")?
                .revision();
            let resident = runtime
                .revision(&item.prototype, item.lod)
                .ok_or("selected LOD not resident")?;
            if resident != source_rev {
                return Err(format!(
                    "{} {:?} resident revision {resident} != source {source_rev}",
                    item.prototype, item.lod
                ));
            }
        }

        // Flags are collected locally and applied only after the scene/runtime
        // borrows end.
        let mut flora_guard_decided = false;
        let mut coarse_realized_seen = false;
        let seen_lods: Vec<Lod> = lod_histogram(frame).keys().copied().collect();

        // Every production flora instance is retained at the authoritative
        // Source. Where its coarse level would pass the pixel budget, the
        // retention is attributable to the default geometry guards.
        let mut flora_pixel_eligible = 0usize;
        for (instance, species) in FLORA_INSTANCES {
            let item = frame
                .selected
                .iter()
                .find(|s| s.instance == instance)
                .ok_or_else(|| format!("production flora instance {instance} missing"))?;
            if item.lod != Lod::Source {
                return Err(format!(
                    "production flora {instance} ({species}) coarsened to {:?} under the default guards",
                    item.lod
                ));
            }
            let scale_m = scene
                .prototype(species)
                .ok_or("flora prototype vanished")?
                .scale()
                .metres();
            if coarse_would_pass_pixels(&camera, scale_m, item.depth_m, &plan.config) {
                flora_pixel_eligible += 1;
            }
        }
        if flora_pixel_eligible > 0 {
            flora_guard_decided = true;
        }

        let sheet = frame
            .selected
            .iter()
            .find(|s| s.instance == SHEET_INSTANCE)
            .ok_or("sheet instance missing from selection")?;
        if sheet.lod != Lod::Source {
            return Err(format!(
                "thin sheet coarsened to {:?} under the default cap",
                sheet.lod
            ));
        }
        let opening = frame
            .selected
            .iter()
            .find(|s| s.instance == OPENING_INSTANCE)
            .ok_or("opening instance missing from selection")?;
        if opening.lod != Lod::Source {
            return Err(format!("local opening coarsened to {:?}", opening.lod));
        }

        // Track approach (near = Source) versus retreat (near coarsened).
        if let Some(near) = frame.selected.iter().find(|s| s.instance == NEAR_INSTANCE) {
            if near.lod == Lod::Source {
                self.near_source_seen = true;
            } else {
                self.near_coarsened_seen = true;
            }
        }

        // An eligible dense instance must realize a real, non-empty coarse mesh
        // whose revision matches the live source.
        let mut coarse_realized = 0usize;
        for item in &frame.selected {
            if item.lod == Lod::Source || item.fallback {
                continue;
            }
            let index = runtime
                .instance_index(&item.prototype, item.lod)
                .ok_or("selected coarse level not resident")?;
            if runtime.meshes()[index].vertices.is_empty() {
                return Err(format!(
                    "{} {:?} realized empty geometry",
                    item.prototype, item.lod
                ));
            }
            let source_rev = scene
                .prototype(&item.prototype)
                .ok_or("coarse prototype vanished")?
                .revision();
            if runtime.revision(&item.prototype, item.lod) != Some(source_rev) {
                return Err(format!(
                    "{} {:?} coarse revision is stale",
                    item.prototype, item.lod
                ));
            }
            coarse_realized += 1;
        }
        if coarse_realized > 0 {
            coarse_realized_seen = true;
        }

        // The occlusion phase must layer a guard-held thin foreground plant in
        // front of an eligible dense solid that actually coarsened: same frame,
        // same camera, per-instance quality decisions.
        if plan.occlusion {
            let fungus = frame
                .selected
                .iter()
                .find(|s| s.instance == FLORA_FUNGUS_INSTANCE)
                .ok_or("occlusion foreground flora missing")?;
            let fungus_scale = scene
                .prototype("funnel_mushroom")
                .ok_or("occlusion fungus prototype vanished")?
                .scale()
                .metres();
            if fungus.lod != Lod::Source
                || !coarse_would_pass_pixels(&camera, fungus_scale, fungus.depth_m, &plan.config)
            {
                return Err(
                    "occlusion foreground flora was not a pixel-eligible guard retention".into(),
                );
            }
            let solid = frame
                .selected
                .iter()
                .find(|s| s.instance == OCCLUSION_SOLID_INSTANCE)
                .ok_or("occlusion rear solid missing")?;
            if solid.lod == Lod::Source || solid.fallback {
                return Err(format!(
                    "occlusion rear solid did not realize a coarse level: {:?} fallback={}",
                    solid.lod, solid.fallback
                ));
            }
            if fungus.depth_m >= solid.depth_m {
                return Err("occlusion foreground flora is not in front of the rear solid".into());
            }
        }

        // Work-budget cap. After bounded convergence the zero-cap phase is a
        // truthful stationarity guarantee: the frame builds nothing.
        if plan.config.max_coarse_builds == Some(0) && frame.mesh_builds_this_call != 0 {
            return Err(format!(
                "work-budget cap still built {} meshes this frame",
                frame.mesh_builds_this_call
            ));
        }

        // Camera/LOD independence of the authoritative answers. Declared
        // move/edit operations already updated `self.probes`, so any other
        // difference is a source-authority leak.
        let now = capture_probes(scene)?;
        if now != self.probes {
            return Err(format!(
                "authoritative probe answers changed without a declared move/edit: {:?}",
                probe_delta(&self.probes, &now)
            ));
        }
        if flora_guard_decided {
            self.flora_guard_decided = true;
        }
        if coarse_realized_seen {
            self.coarse_realized_seen = true;
        }
        for lod in seen_lods {
            self.lods_seen.insert(lod);
        }

        Ok(PhaseCheck {
            deferred: deferred_count(frame),
            flora_pixel_eligible,
            coarse_realized,
        })
    }

    fn finalize(&mut self) -> Result<(), String> {
        if !self.near_source_seen {
            return Err("near instance never resolved to Source on approach".into());
        }
        if !self.near_coarsened_seen {
            return Err("near instance never coarsened on retreat".into());
        }
        if self.lods_seen.len() < 2 {
            return Err(format!(
                "fewer than two LODs were ever chosen: {:?}",
                self.lods_seen
            ));
        }
        if !self.phase_state.edited {
            return Err("prototype edit invalidation phase never ran".into());
        }
        if !self.phase_state.moved {
            return Err("instance move phase never ran".into());
        }
        if !self.phase_state.edit_moved {
            return Err("moved-instance edit phase never ran".into());
        }
        if !self.coarse_realized_seen {
            return Err("no eligible dense instance ever realized a coarse level".into());
        }
        if !self.flora_guard_decided {
            return Err(
                "no production flora instance ever faced a pixel-eligible coarse choice while retained at Source"
                    .into(),
            );
        }
        let now = capture_probes(&self.scene)?;
        if now != self.probes {
            return Err("authoritative probe answers diverged before finalize".into());
        }
        Ok(())
    }
}

struct PhaseCheck {
    deferred: usize,
    flora_pixel_eligible: usize,
    coarse_realized: usize,
}

impl ApplicationHandler for DetailCheck {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        let window = Arc::new(
            el.create_window(
                Window::default_attributes()
                    .with_title("Matterweave detail LOD engine gate")
                    .with_inner_size(winit::dpi::PhysicalSize::new(640, 480)),
            )
            .expect("create window"),
        );
        let mut init = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(r) = init.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
            panic!("renderer init suspended")
        };
        let r = r.expect("renderer");
        self.renderer = Some(r);
        self.window = Some(window);
        let phase = (self.frame / PHASE_FRAMES) as usize;
        let phase = phase.min(self.plans.len() - 1);
        if let Err(error) = self.install_static_scene(phase) {
            self.record(format!("FAIL detail: install static scene: {error}"));
            el.exit();
            return;
        }
        let caps = self.renderer.as_ref().unwrap().capabilities.to_string();
        self.record(format!("capabilities: {caps}"));
        self.record(format!(
            "fixture: {} prototypes, {} instances, {} derived levels buildable with the eager capability check on a disposable fork; resident pool realized lazily",
            self.scene.prototype_ids().len(),
            self.scene.instance_ids().len(),
            self.fixture_levels,
        ));
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                self.prepared = None;
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                return;
            }
            WindowEvent::CloseRequested => {
                el.exit();
                return;
            }
            WindowEvent::RedrawRequested => {}
            _ => return,
        }
        if self.renderer.is_none() || self.finished {
            return;
        }
        let phase = ((self.frame / PHASE_FRAMES) as usize).min(self.plans.len() - 1);
        let plan = self.plans[phase];

        if self.prepared != Some(phase as u32) {
            // The lifecycle-recreate phase drops and recreates the renderer,
            // which re-warms the Source pool and re-installs the static scene.
            if plan.recreate && !self.recreated {
                self.recreated = true;
                let r = self.renderer.as_mut().unwrap();
                r.resize(0, 0);
                if !matches!(
                    r.render_with_lighting(
                        plan.view_projection(1.0),
                        plan.eye,
                        &Hud::new(1.0, 1.0),
                        &self.lighting
                    ),
                    FrameResult::Retry
                ) {
                    self.record("FAIL detail: zero extent did not return Retry".into());
                    el.exit();
                    return;
                }
                self.record("zero-extent=Retry; recreating renderer".into());
                self.suspended(el);
                self.resumed(el);
                if self.renderer.is_none() {
                    self.record("FAIL detail: renderer did not recreate".into());
                    el.exit();
                    return;
                }
                self.record(format!(
                    "phase={phase} {} renderer recreated, static scene re-installed",
                    plan.name
                ));
            }
            match self.apply_phase(phase) {
                Ok(entry) => self.record(entry),
                Err(error) => {
                    self.record(format!("FAIL detail: phase {phase}: {error}"));
                    el.exit();
                    return;
                }
            }
            self.prepared = Some(phase as u32);
        }

        let Some(window) = self.window.as_ref() else {
            return;
        };
        let size = window.inner_size();
        // Zero-extent frames present nothing and are not counted.
        if size.width == 0 || size.height == 0 {
            return;
        }
        let aspect = size.width as f32 / size.height as f32;
        let matrix = plan.view_projection(aspect);
        let eye = plan.eye;
        let mut hud = Hud::new(size.width as f32, size.height as f32);
        for (line, text) in self.hud_lines.iter().enumerate() {
            hud.text(
                8.0,
                8.0 + line as f32 * 10.0,
                text,
                1.0,
                [1.0, 1.0, 1.0, 0.9],
            );
        }
        match self.renderer.as_mut().unwrap().render_with_lighting(
            matrix,
            eye,
            &hud,
            &self.lighting,
        ) {
            FrameResult::Presented => self.frame += 1,
            FrameResult::Retry => {
                return;
            }
            other => {
                self.record(format!("FAIL detail: render {other:?}"));
                el.exit();
                return;
            }
        }

        // Terminal condition: the last phase has been applied and rendered.
        if phase + 1 == self.plans.len() && self.frame >= PHASE_FRAMES * self.plans.len() as u32 {
            match self.finalize() {
                Ok(()) => {
                    let seen: Vec<String> =
                        self.lods_seen.iter().map(|l| format!("{l:?}")).collect();
                    self.record(format!(
                        "PASS detail: {} phases over a controlled fixture plus {} exported production flora species; \
                         bounded lazy convergence (cap {}) realized eligible dense geometry while every flora instance \
                         stayed at Source under the default guards (guarded despite a passing pixel budget where reported); \
                         occlusion view layered guarded thin flora in front of a realized coarse solid; instance move updated \
                         placements without a geometry rebuild, the moved-instance edit invalidated and refreshed its derived \
                         mesh; source collision/sample probes invariant across camera-only phases and updated exactly on the \
                         declared move/edit; thin sheet and local opening held at Source; LODs chosen {}; resident zero-build \
                         budget, zero extent and lifecycle recreation verified (cold-cache cap fallback: unit test only)",
                        self.plans.len(),
                        FLORA_INSTANCES.len(),
                        MAX_COARSE_BUILDS_PER_PREPARE,
                        seen.join("/"),
                    ));
                    self.finished = true;
                    self.renderer = None;
                    self.window = None;
                    el.exit();
                }
                Err(error) => {
                    self.record(format!("FAIL detail: {error}"));
                    el.exit();
                }
            }
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
        // Resident GPU geometry is renderer-owned; a fresh renderer re-warms.
        self.runtime = None;
        self.prepared = None;
        // Derived-geometry evidence belongs to the discarded pool: a fresh pool
        // must re-converge and re-realize invalidated levels rather than trust a
        // flag from the retired runtime. Authoritative mutation state is kept.
        self.phase_state.converged = false;
        self.phase_state.edit_invalidation_pending = false;
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detail_runtime::yaw_quarters;
    use matterweave_detail::{Bounds, InstanceLod};

    fn phase(name: &str) -> PhasePlan {
        phases()
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("phase {name} exists"))
    }

    fn camera(plan: &PhasePlan) -> LodCamera {
        plan.lod_camera(1080.0)
    }

    fn item<'a>(frame: &'a PreparedFrame, instance: &str) -> &'a InstanceLod {
        frame
            .selected
            .iter()
            .find(|s| s.instance == instance)
            .unwrap_or_else(|| panic!("instance {instance} selected"))
    }

    fn lod_of(selected: &[InstanceLod], instance: &str) -> Lod {
        selected
            .iter()
            .find(|s| s.instance == instance)
            .unwrap_or_else(|| panic!("instance {instance} selected"))
            .lod
    }

    fn source_only(config: &LodConfig) -> LodConfig {
        LodConfig {
            max_lod: Lod::Source,
            ..*config
        }
    }

    /// World-space AABB of one selected instance projected into normalized
    /// device coordinates: `[min_x, min_y, max_x, max_y]`.
    fn projected_rect(scene: &DetailScene, plan: &PhasePlan, item: &InstanceLod) -> [f32; 4] {
        let bounds: Bounds = scene
            .prototype(&item.prototype)
            .expect("prototype")
            .bounds_world(&item.transform)
            .expect("transform")
            .expect("nonempty prototype");
        let matrix = Mat4::from_cols_array_2d(&plan.view_projection(16.0 / 9.0));
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for corner in 0..8 {
            let point = Vec3::new(
                if corner & 1 == 0 {
                    bounds.min[0]
                } else {
                    bounds.max[0]
                },
                if corner & 2 == 0 {
                    bounds.min[1]
                } else {
                    bounds.max[1]
                },
                if corner & 4 == 0 {
                    bounds.min[2]
                } else {
                    bounds.max[2]
                },
            );
            let clip = matrix * point.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            min[0] = min[0].min(ndc.x);
            min[1] = min[1].min(ndc.y);
            max[0] = max[0].max(ndc.x);
            max[1] = max[1].max(ndc.y);
        }
        [min[0], min[1], max[0], max[1]]
    }

    #[test]
    fn fixture_realizes_every_prototype_at_every_level() {
        let scene = fixture_scene();
        assert_eq!(
            check_fixture_levels(&scene).unwrap(),
            scene.prototype_ids().len() * LODS.len()
        );
    }

    #[test]
    fn stale_runtime_is_rejected_after_source_edit() {
        let mut scene = fixture_scene();
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        scene
            .edit_prototype(BOULDER, [12, 12, 12], material::BANK_STONE)
            .unwrap();
        let plan = phase("mid-perspective");
        let frame = scene.prepare_batches(&camera(&plan), &plan.config).unwrap();
        assert!(runtime.instances_for_frame(&scene, &frame).is_err());
    }

    #[test]
    fn preload_builds_each_lod_once_with_source_revisions() {
        let mut scene = fixture_scene();
        let prototype_count = scene.prototype_ids().len();
        let before = scene.counts().mesh_builds;
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        // Every nonempty prototype x three levels.
        assert_eq!(runtime.meshes().len(), prototype_count * LODS.len());
        assert_eq!(
            scene.counts().mesh_builds,
            before + (prototype_count * 3) as u64
        );
        for id in scene.prototype_ids() {
            let source_rev = scene.prototype(&id).unwrap().revision();
            for lod in LODS {
                assert!(runtime.instance_index(&id, lod).is_some());
                assert_eq!(runtime.revision(&id, lod), Some(source_rev));
            }
        }
        // Distinct pool indices.
        let mut indices: Vec<usize> = scene
            .prototype_ids()
            .iter()
            .flat_map(|id| LODS.iter().map(move |&lod| (id, lod)))
            .map(|(id, lod)| runtime.instance_index(id, lod).unwrap())
            .collect();
        indices.sort();
        indices.dedup();
        assert_eq!(indices.len(), prototype_count * LODS.len());
    }

    #[test]
    fn native_fixture_preserves_opening_while_dense_control_coarsens() {
        let plan = phase("retreat-far-perspective");
        let mut scene = fixture_scene();
        let frame = scene.prepare_batches(&camera(&plan), &plan.config).unwrap();
        assert_eq!(item(&frame, OPENING_INSTANCE).lod, Lod::Source);
        let control = item(&frame, NEAR_INSTANCE);
        assert!(
            control.lod > Lod::Source,
            "safe dense control must still coarsen"
        );
        assert!(!control.fallback);
    }

    #[test]
    fn approach_selects_source_and_retreat_coarsens_the_same_instance() {
        let near_plan = phase("approach-near-perspective");
        let far_plan = phase("retreat-far-perspective");
        let mut scene = fixture_scene();
        let approach = scene
            .prepare_batches(&camera(&near_plan), &near_plan.config)
            .unwrap();
        assert_eq!(
            item(&approach, NEAR_INSTANCE).lod,
            Lod::Source,
            "near instance is authoritative"
        );

        let mut fresh = fixture_scene();
        let retreat = fresh
            .prepare_batches(&camera(&far_plan), &far_plan.config)
            .unwrap();
        let far_lod = item(&retreat, NEAR_INSTANCE).lod;
        assert!(far_lod > Lod::Source, "far instance coarsened: {far_lod:?}");
    }

    #[test]
    fn thin_sheet_stays_at_source_across_camera_phases() {
        for plan in phases()
            .into_iter()
            .filter(|p| !p.edit && !p.move_flora && !p.edit_moved_flora)
        {
            let mut scene = fixture_scene();
            let frame = scene.prepare_batches(&camera(&plan), &plan.config).unwrap();
            assert_eq!(
                item(&frame, SHEET_INSTANCE).lod,
                Lod::Source,
                "{}",
                plan.name
            );
        }
    }

    #[test]
    fn source_probes_are_camera_and_lod_invariant() {
        let mut scene = fixture_scene();
        let baseline = capture_probes(&scene).unwrap();
        assert!(baseline["solid_core"].collidable);
        assert!(!baseline["occlusion_fungus_pit"].collidable);
        assert!(baseline["occlusion_fungus_stem"].collidable);
        assert!(baseline["shrub_home_root"].collidable);
        assert!(!baseline["shrub_target_root"].collidable);
        assert!(!baseline["solid_edit_cell"].collidable);
        for plan in phases()
            .into_iter()
            .filter(|p| !p.edit && !p.move_flora && !p.edit_moved_flora)
        {
            scene.prepare_batches(&camera(&plan), &plan.config).unwrap();
            assert_eq!(capture_probes(&scene).unwrap(), baseline, "{}", plan.name);
        }
    }

    #[test]
    fn edit_advances_source_version_and_rebuild_matches_new_revision() {
        let mut scene = fixture_scene();
        let mut runtime = DetailRuntime::preload(&mut scene).unwrap();
        let before = scene.source_version();
        let before_rev = runtime.revision(BOULDER, Lod::Quarter).unwrap();
        scene
            .edit_prototype(BOULDER, [12, 12, 12], material::BANK_STONE)
            .unwrap();
        assert_ne!(scene.source_version(), before);
        // A rebuild realizes fresh geometry matching the advanced revision.
        runtime = DetailRuntime::preload(&mut scene).unwrap();
        let after_rev = runtime.revision(BOULDER, Lod::Quarter).unwrap();
        assert_ne!(after_rev, before_rev);
        assert_eq!(after_rev, scene.prototype(BOULDER).unwrap().revision());
    }

    #[test]
    fn budget_cap_falls_back_to_source_on_a_fresh_scene_and_maps_to_source_indices() {
        let far = phases()
            .into_iter()
            .find(|p| p.config.max_coarse_builds == Some(0))
            .expect("zero-budget phase exists");
        assert_eq!(far.name, "resident-zero-build-budget");
        // Fresh scene: no coarse mesh is resident, so the cap forces the
        // authoritative Source and flags the coarse-desiring boulders.
        let mut scene = fixture_scene();
        let frame = scene.prepare_batches(&camera(&far), &far.config).unwrap();
        assert!(frame.selected.iter().all(|s| s.lod == Lod::Source));
        assert!(frame
            .selected
            .iter()
            .filter(|s| s.prototype == BOULDER)
            .all(|s| s.fallback));
        assert!(frame.selected.iter().any(|s| s.fallback));
        // Once every LOD is resident, the same cap builds nothing more.
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        let again = scene.prepare_batches(&camera(&far), &far.config).unwrap();
        assert_eq!(again.mesh_builds_this_call, 0);
        let instances = runtime.instances_for_frame(&scene, &again).unwrap();
        for (instance, selected) in instances.iter().zip(&again.selected) {
            assert_eq!(
                instance.prototype,
                runtime
                    .instance_index(&selected.prototype, selected.lod)
                    .unwrap()
            );
        }
    }

    #[test]
    fn instances_map_selection_to_resident_indices_with_transform() {
        let mut scene = fixture_scene();
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        let plan = phase("mid-perspective");
        let frame = scene.prepare_batches(&camera(&plan), &plan.config).unwrap();
        let instances = runtime.instances_for_frame(&scene, &frame).unwrap();
        assert_eq!(instances.len(), frame.selected.len());
        for (instance, selected) in instances.iter().zip(&frame.selected) {
            assert_eq!(
                instance.prototype,
                runtime
                    .instance_index(&selected.prototype, selected.lod)
                    .unwrap()
            );
            assert_eq!(instance.translation, selected.transform.translation_m);
            assert_eq!(instance.yaw_quarters, yaw_quarters(selected.transform.yaw));
        }
        // Multiple instances including yaw-rotated negative flora placements.
        for (instance, _) in FLORA_INSTANCES {
            let selected = item(&frame, instance);
            let mapped = instances
                .iter()
                .find(|mapped| mapped.translation == selected.transform.translation_m)
                .expect("flora instance mapped");
            assert_eq!(
                mapped.prototype,
                runtime
                    .instance_index(&selected.prototype, selected.lod)
                    .unwrap()
            );
            assert_eq!(mapped.yaw_quarters, yaw_quarters(selected.transform.yaw));
        }
        assert!(frame
            .selected
            .iter()
            .any(|s| yaw_quarters(s.transform.yaw) != 0));
        assert!(instances.iter().any(|i| i.yaw_quarters != 0));
        assert!(instances.iter().any(|i| i.translation[0] < 0.0));
    }

    #[test]
    fn production_flora_corpus_is_held_at_source_where_pixels_would_allow_coarse() {
        let plan = phase("retreat-far-perspective");
        let camera = camera(&plan);
        let mut scene = fixture_scene();
        let frame = scene.prepare_batches(&camera, &plan.config).unwrap();
        let mut pixel_eligible = 0usize;
        for (instance, species) in FLORA_INSTANCES {
            let selected = item(&frame, instance);
            assert_eq!(
                selected.lod,
                Lod::Source,
                "production flora {instance} ({species}) coarsened"
            );
            let scale_m = scene.prototype(species).unwrap().scale().metres();
            if coarse_would_pass_pixels(&camera, scale_m, selected.depth_m, &plan.config) {
                pixel_eligible += 1;
            }
        }
        assert_eq!(
            pixel_eligible,
            FLORA_INSTANCES.len(),
            "every flora species must face a pixel-eligible coarse choice at the retreat camera"
        );
    }

    #[test]
    fn default_quality_guards_not_distance_hold_every_flora_species() {
        let plan = phase("retreat-far-perspective");
        let camera = camera(&plan);
        let mut scene = fixture_scene();
        let default_frame = scene.prepare_batches(&camera, &plan.config).unwrap();

        // Diagnostic attribution probe only -- never a rendered selection:
        // relaxing the two geometry guards lets every species coarsen, proving
        // the default Source retention is caused by the guards, not by scale or
        // pixel budget.
        let relaxed = LodConfig {
            max_lod: Lod::Quarter,
            max_dilation_fraction: 1.0,
            max_local_loss_fraction: 1.0,
            ..plan.config
        };
        let local_relaxed = LodConfig {
            max_lod: Lod::Half,
            max_local_loss_fraction: 1.0,
            ..plan.config
        };
        let all_relaxed = LodConfig {
            max_lod: Lod::Half,
            max_dilation_fraction: 1.0,
            max_local_loss_fraction: 1.0,
            ..plan.config
        };
        let mut relaxed_scene = fixture_scene();
        let relaxed_frame = relaxed_scene.select_lods(&camera, &relaxed).unwrap();
        let mut local_scene = fixture_scene();
        let local_frame = local_scene.select_lods(&camera, &local_relaxed).unwrap();
        let mut all_scene = fixture_scene();
        let all_frame = all_scene.select_lods(&camera, &all_relaxed).unwrap();
        for (instance, species) in FLORA_INSTANCES {
            assert_eq!(item(&default_frame, instance).lod, Lod::Source);
            let relaxed_lod = lod_of(&relaxed_frame, instance);
            assert!(
                relaxed_lod > Lod::Source,
                "{instance} ({species}) was retained by distance, not the quality guards"
            );
            let local_lod = lod_of(&local_frame, instance);
            let guard = if local_lod > Lod::Source {
                "local_loss"
            } else {
                assert!(
                    lod_of(&all_frame, instance) > Lod::Source,
                    "{instance} ({species}) relaxed guard probe did not coarsen"
                );
                "dilation"
            };
            println!(
                "flora guard attribution: {instance} ({species}) default=Source guard={guard} relaxed={relaxed_lod:?}"
            );
        }
    }

    #[test]
    fn occlusion_phase_layers_guarded_flora_in_front_of_a_realized_coarse_solid() {
        let plan = phase("occlusion-foreground-fungus-over-solid");
        let camera = camera(&plan);
        let mut scene = fixture_scene();
        let frame = scene.prepare_batches(&camera, &plan.config).unwrap();

        let fungus = item(&frame, FLORA_FUNGUS_INSTANCE);
        assert_eq!(fungus.lod, Lod::Source, "foreground thin flora is guarded");
        let fungus_scale = scene.prototype("funnel_mushroom").unwrap().scale().metres();
        assert!(
            coarse_would_pass_pixels(&camera, fungus_scale, fungus.depth_m, &plan.config),
            "foreground Source must be a guard retention, not distance"
        );

        let solid = item(&frame, OCCLUSION_SOLID_INSTANCE);
        assert!(
            solid.lod > Lod::Source && !solid.fallback,
            "rear solid must realize a coarse level: {:?}",
            solid.lod
        );
        let solid_mesh = scene
            .cached_prototype_mesh(&solid.prototype, solid.lod)
            .expect("realized coarse mesh is resident");
        assert!(!solid_mesh.vertices.is_empty());
        assert_eq!(
            solid_mesh.revision,
            scene.prototype(&solid.prototype).unwrap().revision()
        );

        assert!(
            fungus.depth_m < solid.depth_m,
            "foreground flora depth {} must be in front of solid depth {}",
            fungus.depth_m,
            solid.depth_m
        );
        let f = projected_rect(&scene, &plan, fungus);
        let s = projected_rect(&scene, &plan, solid);
        let overlaps = f[0] <= s[2] && s[0] <= f[2] && f[1] <= s[3] && s[1] <= f[3];
        assert!(
            overlaps,
            "occlusion view must project the thin flora over the rear solid: {f:?} vs {s:?}"
        );
    }

    #[test]
    fn moving_a_flora_placement_updates_instances_without_rebuilding_geometry() {
        let move_plan = phase("move-flora-instance");
        let camera = camera(&move_plan);
        let mut scene = fixture_scene();
        let mut runtime = DetailRuntime::new();
        runtime
            .prepare(&mut scene, &camera, &source_only(&move_plan.config))
            .unwrap();

        let before = capture_probes(&scene).unwrap();
        assert_eq!(
            before["shrub_home_root"].sample.as_ref().unwrap().0,
            FLORA_SHRUB_INSTANCE
        );
        assert!(before["shrub_home_root"].collidable);

        // Movement is a new authoritative placement of the same prototypes,
        // exactly how a moving detail object appears to the detail system.
        scene = fixture_scene_with_shrub(FLORA_SHRUB_TARGET_M);
        let after = capture_probes(&scene).unwrap();
        assert_eq!(
            probe_delta(&before, &after),
            ["shrub_home_root", "shrub_target_root"]
        );
        assert!(!after["shrub_home_root"].collidable);
        assert_eq!(
            after["shrub_target_root"].sample.as_ref().unwrap().0,
            FLORA_SHRUB_INSTANCE
        );

        let before_pool = runtime.meshes().len();
        let update = runtime
            .prepare(&mut scene, &camera, &move_plan.config)
            .unwrap();
        assert!(
            !update.geometry_changed,
            "a placement move must not rebuild voxel-derived geometry"
        );
        assert_eq!(runtime.meshes().len(), before_pool);
        assert_eq!(
            item(&update.frame, FLORA_SHRUB_INSTANCE)
                .transform
                .translation_m,
            FLORA_SHRUB_TARGET_M
        );
        assert_eq!(capture_probes(&scene).unwrap(), after);
    }

    #[test]
    fn editing_the_moved_instance_invalidates_and_refreshes_its_geometry() {
        let move_plan = phase("move-flora-instance");
        let camera = camera(&move_plan);
        let mut scene = fixture_scene_with_shrub(FLORA_SHRUB_TARGET_M);
        let mut runtime = DetailRuntime::new();
        runtime
            .prepare(&mut scene, &camera, &source_only(&move_plan.config))
            .unwrap();
        let before_rev = scene.prototype("twisted_shrub").unwrap().revision();

        assert!(scene
            .edit_instance(FLORA_SHRUB_INSTANCE, [0, 0, 0], material::AIR)
            .unwrap());
        let after_rev = scene.prototype("twisted_shrub").unwrap().revision();
        assert_ne!(after_rev, before_rev);
        assert!(!capture_probes(&scene).unwrap()["shrub_target_root"].collidable);

        let update = runtime
            .prepare(&mut scene, &camera, &move_plan.config)
            .unwrap();
        assert!(
            update.geometry_changed,
            "an edited instance must invalidate its resident derived mesh"
        );
        let selected = item(&update.frame, FLORA_SHRUB_INSTANCE);
        assert_eq!(
            runtime.revision(&selected.prototype, selected.lod),
            Some(after_rev)
        );
    }

    #[test]
    fn cold_lazy_convergence_is_bounded_and_reaches_a_zero_build_steady_state() {
        let plan = phase("cold-far-bounded-convergence");
        let camera = camera(&plan);
        let cap = plan
            .config
            .max_coarse_builds
            .expect("convergence declares a cap");
        assert_eq!(cap, MAX_COARSE_BUILDS_PER_PREPARE);

        let mut scene = fixture_scene();
        let mut runtime = DetailRuntime::new();
        runtime
            .prepare(&mut scene, &camera, &source_only(&plan.config))
            .unwrap();
        let outcome =
            converge_bounded(&mut runtime, &mut scene, &camera, &plan.config, true).unwrap();

        assert!(
            outcome.iterations >= 2,
            "cap {cap} must defer against multiple distinct coarse pairs"
        );
        assert!(outcome.total_builds > 0, "coarse levels were realized");
        assert!(outcome.max_builds <= cap);
        for item in &outcome.update.frame.selected {
            if item.lod == Lod::Source || item.fallback {
                continue;
            }
            let index = runtime
                .instance_index(&item.prototype, item.lod)
                .expect("coarse level resident");
            assert!(!runtime.meshes()[index].vertices.is_empty());
            assert_eq!(
                runtime.revision(&item.prototype, item.lod),
                Some(scene.prototype(&item.prototype).unwrap().revision())
            );
        }
        // Every flora instance and the guarded opening/sheet stay at Source.
        for (instance, _) in FLORA_INSTANCES {
            assert_eq!(item(&outcome.update.frame, instance).lod, Lod::Source);
        }
        assert_eq!(item(&outcome.update.frame, SHEET_INSTANCE).lod, Lod::Source);
        assert_eq!(
            item(&outcome.update.frame, OPENING_INSTANCE).lod,
            Lod::Source
        );

        // A further identical prepare is a stationary zero-build frame.
        let steady = runtime.prepare(&mut scene, &camera, &plan.config).unwrap();
        assert_eq!(steady.frame.mesh_builds_this_call, 0);
        assert!(!steady.geometry_changed);
        assert_eq!(deferred_count(&steady.frame), 0);

        // The warm-pool re-entry a surface event takes for this phase: the view
        // is fully resident, so the bounded protocol reports a zero-build
        // stationary outcome instead of re-requiring lazy-path deferral (the
        // latter is still a hard failure).
        let reentry = converge_bounded(&mut runtime, &mut scene, &camera, &plan.config, false)
            .expect("warm re-entry must not require lazy-path deferral");
        assert_eq!(reentry.total_builds, 0);
        assert_eq!(reentry.update.frame.mesh_builds_this_call, 0);
        assert!(
            converge_bounded(&mut runtime, &mut scene, &camera, &plan.config, true).is_err(),
            "a fully resident view must fail the lazy-path-deferral guard"
        );
    }

    /// A surface event (resize) or lifecycle recreate re-enters the current
    /// phase. It must re-prepare the live viewport but never replay a one-shot
    /// authoritative mutation, re-require lazy-path deferral from a warm pool,
    /// or demand `geometry_changed` for an edit whose invalidation was already
    /// observed. This drives the same control functions `apply_phase` uses.
    #[test]
    fn phase_reentry_for_a_new_viewport_does_not_replay_one_shot_evidence() {
        // Phase indices whose first application records one-shot evidence:
        // 0 cold bounded convergence, 10 move, 11 moved-flora edit, 12 edit.
        for target in [0usize, 10, 11, 12] {
            let plans = phases();
            let mut scene = fixture_scene();
            let mut runtime = DetailRuntime::new();
            let mut state = PhaseState::default();
            let mut probes = capture_probes(&scene).unwrap();

            // Mirror the renderer install: warm the authoritative Source once
            // before any selection, so the first prepare realizes coarse levels
            // lazily under the phase's declared bound.
            runtime
                .prepare(
                    &mut scene,
                    &plans[0].lod_camera(480.0),
                    &source_only(&plans[0].config),
                )
                .unwrap();

            // Run the ordered script through the target phase, as production
            // does, so a moved-instance edit sees the moved placement.
            for (index, plan) in plans.iter().enumerate().take(target + 1) {
                apply_phase_mutation(&mut scene, plan, &mut state, &mut probes).unwrap();
                prepare_phase(&mut scene, &mut runtime, plan, 480.0, &probes, &mut state)
                    .unwrap_or_else(|e| panic!("phase {index} ({}) first prepare: {e}", plan.name));
            }

            // Re-enter the target phase for the Android viewport against the
            // same warm runtime and mutated scene.
            let plan = plans[target];
            let name = plan.name;

            // A second prepare at the *same* viewport is the exact replay the
            // bug produced (warm pool, nothing changed): the converge guard and
            // the edit-invalidation requirement must not fire again.
            prepare_phase(&mut scene, &mut runtime, &plan, 480.0, &probes, &mut state)
                .unwrap_or_else(|e| panic!("{name} same-viewport re-entry must succeed: {e}"));

            let source_before = scene.source_version();
            let second =
                prepare_phase(&mut scene, &mut runtime, &plan, 1080.0, &probes, &mut state)
                    .unwrap_or_else(|e| panic!("{name} re-entry must succeed: {e}"));
            assert_eq!(
                scene.source_version(),
                source_before,
                "{name} re-entry replayed its one-shot mutation"
            );

            // The re-entry selected for the live 1080 px viewport, not a cached
            // 480 px selection.
            let direct = scene
                .prepare_batches(&plan.lod_camera(1080.0), &plan.config)
                .unwrap();
            let live: Vec<(&str, Lod)> = second
                .update
                .frame
                .selected
                .iter()
                .map(|s| (s.instance.as_str(), s.lod))
                .collect();
            let expected: Vec<(&str, Lod)> = direct
                .selected
                .iter()
                .map(|s| (s.instance.as_str(), s.lod))
                .collect();
            assert_eq!(
                live, expected,
                "{name} did not re-select for the live viewport"
            );
        }
    }

    /// The packed records handed to the renderer, not only the CPU selection,
    /// must carry the selected translation, yaw and resident mesh-pool index.
    /// Corrupted records are the negative controls.
    #[test]
    fn packed_instances_are_validated_and_corrupted_records_are_rejected() {
        let plan = phase("retreat-far-perspective");
        let mut scene = fixture_scene();
        let mut runtime = DetailRuntime::new();
        let update = runtime
            .prepare(&mut scene, &camera(&plan), &plan.config)
            .unwrap();
        validate_packed_instances(&scene, &runtime, &update).unwrap();
        assert!(
            update.instances.len() > 1,
            "negative controls need several packed records"
        );

        let mut corrupted = update.clone();
        corrupted.instances[0].translation[0] += 1.0;
        assert!(
            validate_packed_instances(&scene, &runtime, &corrupted).is_err(),
            "a wrong packed translation must be rejected"
        );

        let mut corrupted = update.clone();
        corrupted.instances[0].yaw_quarters = (corrupted.instances[0].yaw_quarters + 1) % 4;
        assert!(
            validate_packed_instances(&scene, &runtime, &corrupted).is_err(),
            "a wrong packed yaw must be rejected"
        );

        // A valid but wrong mesh-pool index is rejected. The confusion case is
        // another resident pool index, not the `frame.selected` position.
        let expected = update.instances[0].prototype;
        let other = update
            .instances
            .iter()
            .map(|packed| packed.prototype)
            .find(|&index| index != expected)
            .expect("a second distinct resident pool index");
        assert!(other < runtime.meshes().len());
        let mut corrupted = update.clone();
        corrupted.instances[0].prototype = other;
        assert!(
            validate_packed_instances(&scene, &runtime, &corrupted).is_err(),
            "a wrong mesh-pool index must be rejected"
        );

        let mut corrupted = update.clone();
        corrupted.instances.pop();
        assert!(
            validate_packed_instances(&scene, &runtime, &corrupted).is_err(),
            "a missing packed record must be rejected"
        );
    }

    /// The named dense controls are eligible positive coarsening controls: with
    /// the default quality guards untouched, both realize a non-empty coarse
    /// level at the host render window's 480 px viewport. (The tile-scale control
    /// clears the pixel budget at 480 px but not at the 1080 px focus viewport,
    /// where it stays `Source` by projected size, not by a relaxed guard.)
    #[test]
    fn named_dense_controls_coarsen_under_default_guards() {
        let plan = phase("retreat-far-perspective");
        let mut scene = fixture_scene();
        let frame = scene
            .prepare_batches(&plan.lod_camera(480.0), &plan.config)
            .unwrap();
        for instance in [CONTROL_INSTANCE, TILE_CONTROL_INSTANCE, NEAR_INSTANCE] {
            let selected = item(&frame, instance);
            assert!(
                selected.lod > Lod::Source && !selected.fallback,
                "{instance} must coarsen under the default guards, chose {:?} fallback={}",
                selected.lod,
                selected.fallback
            );
            let mesh = scene
                .cached_prototype_mesh(&selected.prototype, selected.lod)
                .expect("positive control coarse mesh is resident");
            assert!(
                !mesh.vertices.is_empty(),
                "{instance} realized empty geometry"
            );
            assert_eq!(
                mesh.revision,
                scene.prototype(&selected.prototype).unwrap().revision(),
                "{instance} coarse revision is stale"
            );
        }
    }

    /// An edit made while a coarse level is selected leaves the `Source` slot
    /// stale and unselected. Approaching `Source` must refresh that slot before
    /// handing it to the renderer, without recreating the runtime or eagerly
    /// rebuilding every level.
    #[test]
    fn edit_at_coarse_then_approach_refreshes_the_stale_source_slot_without_recreating_the_runtime()
    {
        let far_plan = phase("retreat-far-perspective");
        let near_plan = phase("approach-near-perspective");
        let mut scene = fixture_scene();
        let mut runtime = DetailRuntime::new();

        // Warm the authoritative Source, then let the far camera realize and
        // select a coarse level. Source is now resident at the pre-edit
        // revision but unselected.
        runtime
            .prepare(
                &mut scene,
                &camera(&far_plan),
                &source_only(&far_plan.config),
            )
            .unwrap();
        let far = runtime
            .prepare(&mut scene, &camera(&far_plan), &far_plan.config)
            .unwrap();
        assert!(item(&far.frame, NEAR_INSTANCE).lod > Lod::Source);
        let stale_revision = runtime.revision(BOULDER, Lod::Source).unwrap();
        assert_eq!(stale_revision, scene.prototype(BOULDER).unwrap().revision());

        scene
            .edit_prototype(BOULDER, [12, 12, 12], material::BANK_STONE)
            .unwrap();
        let fresh_revision = scene.prototype(BOULDER).unwrap().revision();
        assert_ne!(fresh_revision, stale_revision);

        // Approaching Source re-selects the stale slot; the same runtime must
        // refresh it before it is handed out.
        let near = runtime
            .prepare(&mut scene, &camera(&near_plan), &near_plan.config)
            .unwrap();
        let selected = item(&near.frame, NEAR_INSTANCE);
        assert_eq!(selected.lod, Lod::Source);
        assert!(
            near.geometry_changed,
            "the stale Source slot must be refreshed before selection"
        );
        assert_eq!(runtime.revision(BOULDER, Lod::Source), Some(fresh_revision));
        let index = runtime.instance_index(BOULDER, Lod::Source).unwrap();
        assert!(!runtime.meshes()[index].vertices.is_empty());
        assert_eq!(runtime.meshes()[index].revision, fresh_revision);
        assert!(
            near.instances
                .iter()
                .any(|packed| packed.prototype == index),
            "the packed record must reference the refreshed Source slot"
        );
        validate_packed_instances(&scene, &runtime, &near).unwrap();
    }
}
