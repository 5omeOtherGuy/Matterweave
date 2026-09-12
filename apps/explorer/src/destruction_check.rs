//! Rendered destruction diagnostic: a welded source structure, an exact
//! 64-piece fracture, a `PhysicsSave` JSON round trip, and 20 internal
//! reset/load cycles, each driving the real Vulkan renderer and
//! `DynamicMeshCache` upload path.
//!
//! The fixture is deliberately bounded: a 64x64 stone floor (16 terrain chunk
//! colliders), two welded 1 m tower cubes, one welded `[6, 2, 2]` beam and three
//! loose 1 m cubes — six bodies holding exactly 64 voxels, the engine's
//! `MAX_BODIES` cap. A full fracture therefore ends at exactly 64 unit pieces
//! with nothing refused and nothing left over.
//!
//! Owns no world save. It never reads or writes user data; the only file it
//! touches is `destruction-check-report.txt` beside the caller-provided path.
//! Android screenshots are captured externally with adb by the device lead.
//!
//! Every drawn frame carries a small diagnostic-only text overlay (phase/cycle
//! and the live body/weld/collider counts read from the physics state behind
//! that frame) built with the engine's existing immediate `Hud` API. It is drawn
//! only by this opt-in gate; the playable samples are untouched.
//!
//! Coverage is stated in the report itself. The 20 cycles are internal
//! `PhysicsSave` load cycles, not Android HOME/RESUME events; those are counted
//! separately from the real `suspended`/`resumed` callbacks and are only
//! exercised when the platform actually delivers them.

use crate::dynamic_upload::DynamicUploadState;
use glam::{Mat4, Vec3};
use matterweave_core::{Vertex, World};
use matterweave_physics::{
    BodySnapshot, DynamicMeshCache, Physics, PhysicsSave, PhysicsSnapshot, FIXED_DT, MAX_BODIES,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, Sun};
use std::{
    future::Future,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

/// The six source bodies hold exactly this many voxels, which equals
/// `MAX_BODIES`, so a full fracture is exact in both directions.
const SOURCE_BODIES: usize = 6;
const SOURCE_VOXELS: usize = 64;
/// Five 1 m cubes at 3 kg/m3 plus one 3 m3 beam.
const SOURCE_MASS_KG: f32 = 24.0;
const SOURCE_WELDS: usize = 2;
const LOOSE_BODY_START: usize = 3;
const LOOSE_BODIES: usize = 3;
/// 6 bodies - 3 loose + 3 x 8 unit pieces.
const LOOSE_FRACTURE_BODIES: usize = SOURCE_BODIES - LOOSE_BODIES + LOOSE_BODIES * 8;
/// The floor in `floor_world` covers a 4x4 chunk X/Z footprint.
const GROUND_CHUNK_COLLIDERS: usize = 16;
const BOX_VERTICES: usize = 24;
const BOX_INDICES: usize = 36;
const SIM_STEPS: usize = 30;
const RESET_CYCLES: usize = 20;
const RESET_CYCLE_FIRST: usize = 5;
const PHASE_COUNT: usize = RESET_CYCLE_FIRST + RESET_CYCLES;

const CAMERA_EYE: [f32; 3] = [6.0, 5.0, 15.0];
const CAMERA_TARGET: [f32; 3] = [4.5, 1.5, 0.5];

#[cfg(target_os = "android")]
const PHASE_FRAMES: u32 = 60;
#[cfg(not(target_os = "android"))]
const PHASE_FRAMES: u32 = 6;

const COVERAGE: &str = "coverage: Vulkan presentation of the welded source structure and of the 64-piece fractured dynamic mesh while simulated; runtime checks in every phase for exact fracture counts, unit-piece identity, finite conserved mass (24 kg), PhysicsSave JSON round trip, 20 internal reset/load cycles with body/collider/constraint counts and a declared retained-mesh-byte bound; internal renderer recreation and zero-extent Retry. Not covered: sustained performance, thermal behavior, user saves, process death, and real Android HOME/RESUME unless platform suspend/resume events are reported above zero";

/// Declared ceiling on `DynamicMeshCache::retained_bytes` after the fixture's
/// bounded 64-piece fracture. The exact payload is `MAX_BODIES` boxes of
/// `BOX_VERTICES` vertices plus `BOX_INDICES` indices; vector capacity can
/// double that while growing, and the cache keeps two 64-entry render-input
/// scratch vectors, admitted here as 8 KiB. The runtime reset cycles also
/// require the value not to grow after the first cycle.
const MESH_RETAINED_BOUND_BYTES: usize = 2
    * MAX_BODIES
    * (BOX_VERTICES * std::mem::size_of::<Vertex>() + BOX_INDICES * std::mem::size_of::<u32>())
    + 8 * 1024;

const fn colliders(bodies: usize) -> usize {
    1 + GROUND_CHUNK_COLLIDERS + bodies
}

fn voxel_count(dimensions: [u8; 3]) -> usize {
    dimensions.iter().map(|&n| n as usize).product()
}

fn floor_world() -> World {
    let mut world = World::new(5);
    for x in -32..32 {
        for z in -32..32 {
            world.set([x, 0, z], 3);
        }
    }
    world
}

/// The fixed source scene. Body order is the `PhysicsSave` index order:
/// 0/1 are the welded tower cubes, 2 is the welded beam, 3..6 are loose cubes.
fn source_bodies() -> Vec<BodySnapshot> {
    let body = |position: [f32; 3], dimensions: [u8; 3], material: u8| BodySnapshot {
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
        dimensions,
        material,
    };
    vec![
        body([0.5, 1.51, 0.5], [2, 2, 2], 8),
        body([0.5, 2.53, 0.5], [2, 2, 2], 8),
        body([1.5, 3.55, 0.5], [6, 2, 2], 8),
        body([4.5, 1.51, 0.5], [2, 2, 2], 7),
        body([7.5, 1.51, 0.5], [2, 2, 2], 7),
        body([10.5, 1.51, 0.5], [2, 2, 2], 7),
    ]
}

fn source_snapshot() -> PhysicsSnapshot {
    PhysicsSnapshot {
        version: 1,
        eye: CAMERA_EYE,
        bodies: source_bodies(),
    }
}

fn view_projection(aspect: f32) -> [[f32; 4]; 4] {
    let eye = Vec3::from_array(CAMERA_EYE);
    let target = Vec3::from_array(CAMERA_TARGET);
    (Mat4::perspective_rh(60f32.to_radians(), aspect, 0.1, 200.0)
        * Mat4::look_at_rh(eye, target, Vec3::Y))
    .to_cols_array_2d()
}

/// The diagnostic overlay text: phase/cycle identification plus the counts of
/// the state that produced the frame. Kept as one pure function so the strings
/// shown on device are covered by an ordinary unit test.
fn diagnostic_lines(
    phase_index: usize,
    bodies: usize,
    welds: usize,
    colliders: usize,
    drawn: u32,
    epoch: u64,
) -> [String; 3] {
    [
        format!(
            "phase {phase_index}/{} {}",
            PHASE_COUNT - 1,
            phase_name(phase_for(phase_index))
        ),
        format!("bodies {bodies}  welds {welds}  colliders {colliders}"),
        format!("drawn {drawn}  epoch {epoch}"),
    ]
}

/// Compares every saved body field within `tolerance`; dimensions and material
/// must match exactly because they define the piece identity.
fn snapshots_match(a: &PhysicsSnapshot, b: &PhysicsSnapshot, tolerance: f32) -> Result<(), String> {
    if a.bodies.len() != b.bodies.len() {
        return Err(format!(
            "snapshot body counts differ: {} vs {}",
            a.bodies.len(),
            b.bodies.len()
        ));
    }
    for (index, (left, right)) in a.bodies.iter().zip(&b.bodies).enumerate() {
        if left.dimensions != right.dimensions || left.material != right.material {
            return Err(format!(
                "body {index} identity differs: {:?}/{} vs {:?}/{}",
                left.dimensions, left.material, right.dimensions, right.material
            ));
        }
        for axis in 0..3 {
            if (left.position[axis] - right.position[axis]).abs() > tolerance
                || (left.velocity[axis] - right.velocity[axis]).abs() > tolerance
                || (left.angular_velocity[axis] - right.angular_velocity[axis]).abs() > tolerance
            {
                return Err(format!(
                    "body {index} axis {axis} differs beyond {tolerance}"
                ));
            }
        }
        for axis in 0..4 {
            if (left.rotation[axis] - right.rotation[axis]).abs() > tolerance {
                return Err(format!(
                    "body {index} rotation axis {axis} differs beyond {tolerance}"
                ));
            }
        }
    }
    Ok(())
}

/// CPU side of the diagnostic: world, physics state, dynamic mesh cache and the
/// persisted baseline. Everything here runs without a renderer, so the exact
/// fracture/persistence invariants are covered by ordinary unit tests.
struct Fixture {
    world: World,
    physics: Physics,
    mesh: DynamicMeshCache,
    baseline: PhysicsSave,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        let world = floor_world();
        let mut physics = Physics::new(&world);
        physics
            .restore(&source_snapshot())
            .map_err(|error| format!("restore source snapshot: {error}"))?;
        if !physics.constrain(0, 1) {
            return Err("weld between the two tower cubes was refused".into());
        }
        if !physics.constrain(1, 2) {
            return Err("weld between the upper cube and the beam was refused".into());
        }
        let baseline = physics.save();
        let mut mesh = DynamicMeshCache::default();
        mesh.update(&physics);
        let fixture = Self {
            world,
            physics,
            mesh,
            baseline,
        };
        fixture.assert_source_state("fixture construction")?;
        Ok(fixture)
    }

    fn check_counts(
        &self,
        context: &str,
        bodies: usize,
        constraints: usize,
        colliders: usize,
    ) -> Result<(), String> {
        let actual = (
            self.physics.body_count(),
            self.physics.constraint_count(),
            self.physics.collider_count(),
        );
        if actual != (bodies, constraints, colliders) {
            return Err(format!(
                "{context}: bodies/constraints/colliders {actual:?}, expected ({bodies}, {constraints}, {colliders})"
            ));
        }
        Ok(())
    }

    fn assert_source_state(&self, context: &str) -> Result<(), String> {
        self.check_counts(
            context,
            SOURCE_BODIES,
            SOURCE_WELDS,
            colliders(SOURCE_BODIES),
        )?;
        let bodies = self.physics.snapshot().bodies;
        let voxels: usize = bodies.iter().map(|body| voxel_count(body.dimensions)).sum();
        if voxels != SOURCE_VOXELS {
            return Err(format!(
                "{context}: source has {voxels} voxels, expected {SOURCE_VOXELS}"
            ));
        }
        let mut mass = 0.0;
        for index in 0..self.physics.body_count() {
            mass += self
                .physics
                .body_mass(index)
                .ok_or_else(|| format!("{context}: body {index} has no mass"))?;
        }
        if (mass - SOURCE_MASS_KG).abs() > 1e-4 {
            return Err(format!(
                "{context}: source mass {mass} kg, expected {SOURCE_MASS_KG}"
            ));
        }
        if self.physics.save().constraints != self.baseline.constraints {
            return Err(format!(
                "{context}: live weld topology differs from the baseline save"
            ));
        }
        Ok(())
    }

    /// A fully fractured state must be exactly `MAX_BODIES` unit pieces with a
    /// finite total mass equal to the source mass, so a fracture can neither
    /// lose volume nor invent it. Returns the measured total in kilograms.
    fn check_unit_pieces(&self, context: &str) -> Result<f32, String> {
        let bodies = self.physics.snapshot().bodies;
        if bodies.len() != MAX_BODIES {
            return Err(format!(
                "{context}: {} bodies, expected exactly {MAX_BODIES}",
                bodies.len()
            ));
        }
        for (index, body) in bodies.iter().enumerate() {
            if body.dimensions != [1, 1, 1] {
                return Err(format!(
                    "{context}: body {index} has dimensions {:?}, expected one unit voxel",
                    body.dimensions
                ));
            }
        }
        let mut mass = 0.0;
        for index in 0..self.physics.body_count() {
            let piece = self
                .physics
                .body_mass(index)
                .ok_or_else(|| format!("{context}: body {index} has no mass"))?;
            if !piece.is_finite() || piece <= 0.0 {
                return Err(format!("{context}: body {index} has invalid mass {piece}"));
            }
            mass += piece;
        }
        if !mass.is_finite() || (mass - SOURCE_MASS_KG).abs() > 1e-4 {
            return Err(format!(
                "{context}: fractured mass {mass} kg, expected source {SOURCE_MASS_KG} kg"
            ));
        }
        Ok(mass)
    }

    /// Runtime retained-byte bound for the CPU dynamic mesh. Growth after the
    /// first full fracture is checked separately by the caller.
    fn check_retained_bound(&self, context: &str) -> Result<usize, String> {
        let retained = self.mesh.retained_bytes();
        if retained > MESH_RETAINED_BOUND_BYTES {
            return Err(format!(
                "{context}: retained dynamic mesh bytes {retained} exceed the declared bound {MESH_RETAINED_BOUND_BYTES}"
            ));
        }
        Ok(retained)
    }

    fn check_finite(&self, context: &str) -> Result<(), String> {
        let activity = self.physics.body_activity();
        if activity.total != self.physics.body_count() {
            return Err(format!(
                "{context}: activity total {} != body count {}",
                activity.total,
                self.physics.body_count()
            ));
        }
        for (index, body) in self.physics.snapshot().bodies.iter().enumerate() {
            let finite = body.position.iter().all(|value| value.is_finite())
                && body.velocity.iter().all(|value| value.is_finite())
                && body.angular_velocity.iter().all(|value| value.is_finite())
                && body.rotation.iter().all(|value| value.is_finite());
            if !finite {
                return Err(format!("{context}: body {index} is not finite: {body:?}"));
            }
        }
        Ok(())
    }

    fn check_welds_match_baseline(&self, context: &str) -> Result<(), String> {
        if self.physics.save().constraints != self.baseline.constraints {
            return Err(format!(
                "{context}: live weld topology differs from the persisted baseline"
            ));
        }
        Ok(())
    }

    /// Actual dynamic geometry: one box per live body, exact vertex and index
    /// counts. This cannot pass with a counter that only tracks rebuilds.
    fn mesh_boxes(&self) -> Result<usize, String> {
        let vertices = self.mesh.mesh().vertices.len();
        let indices = self.mesh.mesh().indices.len();
        if !vertices.is_multiple_of(BOX_VERTICES) {
            return Err(format!(
                "dynamic mesh vertex count {vertices} is not whole boxes"
            ));
        }
        let boxes = vertices / BOX_VERTICES;
        if indices != boxes * BOX_INDICES {
            return Err(format!(
                "dynamic mesh has {indices} indices for {boxes} boxes"
            ));
        }
        let bodies = self.physics.body_count();
        if boxes != bodies {
            return Err(format!(
                "dynamic mesh has {boxes} boxes for {bodies} live bodies"
            ));
        }
        Ok(boxes)
    }

    /// Breaks the body nearest to `target` along +Z. The fixture rows every
    /// body at z = 0.5 with clear air between the rays, so the seed is exact.
    fn break_at(&mut self, target: [f32; 3]) -> bool {
        self.physics.break_body(
            &self.world,
            [target[0], target[1], target[2] - 3.0],
            [0.0, 0.0, 1.0],
            8.0,
        )
    }

    fn fracture_all(&mut self) -> Result<(), String> {
        let targets: Vec<[f32; 3]> = self
            .physics
            .snapshot()
            .bodies
            .iter()
            .map(|body| body.position)
            .collect();
        for target in targets {
            if !self.break_at(target) {
                return Err(format!(
                    "fracture refused at {target:?} with {} bodies",
                    self.physics.body_count()
                ));
            }
        }
        Ok(())
    }

    fn fracture_loose(&mut self) -> Result<(), String> {
        let targets: Vec<[f32; 3]> = self
            .physics
            .snapshot()
            .bodies
            .get(LOOSE_BODY_START..)
            .ok_or("source snapshot has no loose bodies")?
            .iter()
            .map(|body| body.position)
            .collect();
        if targets.len() != LOOSE_BODIES {
            return Err(format!(
                "fixture has {} loose bodies, expected {LOOSE_BODIES}",
                targets.len()
            ));
        }
        for target in targets {
            if !self.break_at(target) {
                return Err(format!("loose fracture refused at {target:?}"));
            }
        }
        Ok(())
    }

    fn simulate(&mut self, steps: usize) {
        for _ in 0..steps {
            self.physics.set_flying_eye(CAMERA_EYE);
            self.physics.step_objects(FIXED_DT);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Source,
    LooseFracture,
    FullFracture,
    SerializeReload,
    LifecycleRecreate,
    ResetCycle(usize),
}

fn phase_for(index: usize) -> Phase {
    match index {
        0 => Phase::Source,
        1 => Phase::LooseFracture,
        2 => Phase::FullFracture,
        3 => Phase::SerializeReload,
        4 => Phase::LifecycleRecreate,
        other => Phase::ResetCycle(other - RESET_CYCLE_FIRST),
    }
}

fn phase_name(phase: Phase) -> String {
    match phase {
        Phase::Source => "source".into(),
        Phase::LooseFracture => "loose-fracture".into(),
        Phase::FullFracture => "full-fracture-64".into(),
        Phase::SerializeReload => "serialize-reload".into(),
        Phase::LifecycleRecreate => "lifecycle-recreate".into(),
        Phase::ResetCycle(index) => format!("reset-cycle-{index:02}"),
    }
}

pub(crate) struct DestructionCheck {
    report_path: PathBuf,
    report: Vec<String>,
    fixture: Option<Fixture>,
    init_error: Option<String>,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    lighting: LightingSettings,
    upload: DynamicUploadState,
    renderer_epoch: u64,
    phase: usize,
    phase_presented: u32,
    total_presented: u32,
    phase_uploads: u32,
    total_uploads: u32,
    phase_rebuilds: u32,
    total_rebuilds: u32,
    prepared: Option<usize>,
    platform_suspends: u32,
    platform_resumes: u32,
    internal_recreations: u32,
    retained_bytes_after_first_cycle: Option<usize>,
    final_reset_done: bool,
    failed: bool,
    finished: bool,
}

impl DestructionCheck {
    pub(crate) fn new(report_path: PathBuf) -> Self {
        let (fixture, init_error) = match Fixture::new() {
            Ok(fixture) => (Some(fixture), None),
            Err(error) => (None, Some(error)),
        };
        let check = Self {
            report_path,
            report: vec![
                "Matterweave destruction check report (standalone; no user save is read or written)"
                    .into(),
                COVERAGE.into(),
            ],
            fixture,
            init_error,
            renderer: None,
            window: None,
            lighting: LightingSettings {
                sun: Sun {
                    direction_to_sun: [0.45, 1.0, 0.35],
                    intensity: 1.0,
                },
                ..Default::default()
            },
            upload: DynamicUploadState::default(),
            renderer_epoch: 0,
            phase: 0,
            phase_presented: 0,
            total_presented: 0,
            phase_uploads: 0,
            total_uploads: 0,
            phase_rebuilds: 0,
            total_rebuilds: 0,
            prepared: None,
            platform_suspends: 0,
            platform_resumes: 0,
            internal_recreations: 0,
            retained_bytes_after_first_cycle: None,
            final_reset_done: false,
            failed: false,
            finished: false,
        };
        check.write_report();
        check
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    fn write_report(&self) {
        std::fs::write(&self.report_path, self.report.join("\n") + "\n")
            .expect("write destruction check report");
    }

    fn record(&mut self, entry: String) {
        log::info!("{entry}");
        #[cfg(not(target_os = "android"))]
        eprintln!("{entry}");
        self.report.push(entry);
        self.write_report();
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.record(format!("FAIL destruction: {error}"));
        self.failed = true;
        self.finished = true;
        self.renderer = None;
        self.window = None;
        event_loop.exit();
    }

    fn create_renderer(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Matterweave destruction engine gate")
                        .with_inner_size(winit::dpi::PhysicalSize::new(720, 480)),
                )
                .map_err(|error| format!("create window: {error}"))?,
        );
        let mut init = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(renderer) = init.as_mut().poll(&mut Context::from_waker(Waker::noop()))
        else {
            return Err("renderer initialization suspended".into());
        };
        let mut renderer = renderer.map_err(|error| format!("renderer init: {error}"))?;
        let world_mesh = self
            .fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .world
            .mesh();
        renderer
            .upload(&world_mesh)
            .map_err(|error| format!("world upload: {error}"))?;
        self.renderer_epoch += 1;
        self.renderer = Some(renderer);
        self.window = Some(window);
        self.prepared = None;
        let capabilities = self
            .renderer
            .as_ref()
            .expect("renderer")
            .capabilities
            .to_string();
        self.record(format!(
            "renderer epoch {}: capabilities: {capabilities}",
            self.renderer_epoch
        ));
        Ok(())
    }

    fn release_renderer(&mut self) {
        self.renderer = None;
        self.window = None;
        self.prepared = None;
    }

    /// Rebuilds the CPU dynamic geometry if the bodies moved and uploads it when
    /// the cache or the renderer epoch demands it. Returns without upload when
    /// no renderer exists, which is a valid pre-resume state.
    fn sync_dynamic(&mut self) -> Result<(), String> {
        let Some(fixture) = self.fixture.as_mut() else {
            return Ok(());
        };
        let rebuilt = fixture.mesh.update(&fixture.physics);
        self.phase_rebuilds += u32::from(rebuilt);
        self.total_rebuilds += u32::from(rebuilt);
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        if self.upload.needs_upload(rebuilt, self.renderer_epoch) {
            renderer
                .upload_dynamic(fixture.mesh.mesh())
                .map_err(|error| format!("dynamic upload: {error}"))?;
            self.upload.uploaded(self.renderer_epoch);
            self.phase_uploads += 1;
            self.total_uploads += 1;
        }
        Ok(())
    }

    /// Presents one frame of the current state and advances physics by one fixed
    /// step. Returns whether a frame was actually presented; zero extent and
    /// swapchain Retry present nothing and are never counted.
    fn draw(&mut self) -> Result<bool, String> {
        let Some(window) = self.window.as_ref() else {
            return Ok(false);
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(false);
        }
        self.sync_dynamic()?;
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(false);
        };
        let aspect = size.width as f32 / size.height as f32;
        let mut hud = Hud::new(size.width as f32, size.height as f32);
        // Diagnostic-only overlay; see the module docs. Never drawn by the
        // playable samples, and it reads the live state instead of counters.
        let (bodies, welds, colliders) = self.fixture.as_ref().map_or((0, 0, 0), |fixture| {
            (
                fixture.physics.body_count(),
                fixture.physics.constraint_count(),
                fixture.physics.collider_count(),
            )
        });
        let lines = diagnostic_lines(
            self.phase,
            bodies,
            welds,
            colliders,
            self.total_presented,
            self.renderer_epoch,
        );
        hud.rect([8.0, 8.0, 560.0, 72.0], [0.0, 0.0, 0.0, 0.72]);
        for (line_index, line) in lines.iter().enumerate() {
            hud.text(
                16.0,
                14.0 + line_index as f32 * 22.0,
                line,
                2.0,
                [1.0, 0.96, 0.72, 1.0],
            );
        }
        let result = renderer.render_with_lighting(
            view_projection(aspect),
            CAMERA_EYE,
            &hud,
            &self.lighting,
        );
        match result {
            FrameResult::Presented => {
                self.phase_presented += 1;
                self.total_presented += 1;
            }
            FrameResult::Retry => return Ok(false),
            FrameResult::OutOfMemory => {
                return Err("renderer reported out of memory".into());
            }
            FrameResult::Fatal(error) => {
                return Err(format!("renderer fatal: {error}"));
            }
        }
        if let Some(fixture) = self.fixture.as_mut() {
            fixture.physics.set_flying_eye(CAMERA_EYE);
            fixture.physics.step_objects(FIXED_DT);
        }
        Ok(true)
    }

    fn load_baseline(&mut self, context: &str) -> Result<(), String> {
        let fixture = self.fixture.as_mut().ok_or("fixture unavailable")?;
        fixture
            .physics
            .load(&fixture.baseline)
            .map_err(|error| format!("{context}: load baseline save: {error}"))
    }

    fn check_counts(
        &self,
        context: &str,
        bodies: usize,
        constraints: usize,
        colliders: usize,
    ) -> Result<(), String> {
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .check_counts(context, bodies, constraints, colliders)
    }

    fn check_source(&self, context: &str) -> Result<(), String> {
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .assert_source_state(context)
    }

    fn check_finite(&self, context: &str) -> Result<(), String> {
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .check_finite(context)
    }

    fn check_unit_pieces(&self, context: &str) -> Result<f32, String> {
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .check_unit_pieces(context)
    }

    fn check_retained_bound(&self, context: &str) -> Result<usize, String> {
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .check_retained_bound(context)
    }

    fn fracture_all(&mut self) -> Result<(), String> {
        self.fixture
            .as_mut()
            .ok_or("fixture unavailable")?
            .fracture_all()
    }

    fn simulate(&mut self, steps: usize) {
        if let Some(fixture) = self.fixture.as_mut() {
            fixture.simulate(steps);
        }
    }

    fn mesh_boxes(&self) -> Result<usize, String> {
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .mesh_boxes()
    }

    fn apply_source_phase(&mut self) -> Result<Vec<String>, String> {
        self.load_baseline("source")?;
        self.check_source("source")?;
        self.sync_dynamic()?;
        let boxes = self.mesh_boxes()?;
        Ok(vec![format!(
            "phase=0 source bodies={SOURCE_BODIES} welds={SOURCE_WELDS} colliders={} voxels={SOURCE_VOXELS} mass_kg={SOURCE_MASS_KG:.3} mesh_boxes={boxes}",
            colliders(SOURCE_BODIES)
        )])
    }

    fn apply_loose_fracture_phase(&mut self) -> Result<Vec<String>, String> {
        self.load_baseline("loose-fracture")?;
        self.fixture
            .as_mut()
            .ok_or("fixture unavailable")?
            .fracture_loose()?;
        self.check_counts(
            "loose-fracture",
            LOOSE_FRACTURE_BODIES,
            SOURCE_WELDS,
            colliders(LOOSE_FRACTURE_BODIES),
        )?;
        self.fixture
            .as_ref()
            .ok_or("fixture unavailable")?
            .check_welds_match_baseline("loose-fracture")?;
        self.simulate(SIM_STEPS);
        self.check_finite("loose-fracture")?;
        self.sync_dynamic()?;
        let boxes = self.mesh_boxes()?;
        Ok(vec![format!(
            "phase=1 loose-fracture bodies={LOOSE_FRACTURE_BODIES} welds={SOURCE_WELDS} colliders={} mesh_boxes={boxes} simulated_steps={SIM_STEPS}",
            colliders(LOOSE_FRACTURE_BODIES)
        )])
    }

    fn apply_full_fracture_phase(&mut self) -> Result<Vec<String>, String> {
        self.load_baseline("full-fracture")?;
        self.fracture_all()?;
        self.check_counts("full-fracture", MAX_BODIES, 0, colliders(MAX_BODIES))?;
        let mass = self.check_unit_pieces("full-fracture")?;
        self.simulate(SIM_STEPS);
        self.check_finite("full-fracture")?;
        self.sync_dynamic()?;
        let boxes = self.mesh_boxes()?;
        let retained = self.check_retained_bound("full-fracture")?;
        Ok(vec![format!(
            "phase=2 full-fracture-64 bodies={MAX_BODIES} welds=0 colliders={} unit_pieces=true mass_kg={mass:.3} retained_bytes={retained} mesh_boxes={boxes} simulated_steps={SIM_STEPS}",
            colliders(MAX_BODIES)
        )])
    }

    fn apply_serialize_phase(&mut self) -> Result<Vec<String>, String> {
        self.load_baseline("serialize-reload")?;
        self.fracture_all()?;
        self.check_counts("serialize-reload", MAX_BODIES, 0, colliders(MAX_BODIES))?;
        self.simulate(SIM_STEPS);
        self.check_finite("serialize-reload")?;
        let (json_bytes, decoded) = {
            let fixture = self.fixture.as_ref().ok_or("fixture unavailable")?;
            let save = fixture.physics.save();
            let json = serde_json::to_string(&save)
                .map_err(|error| format!("encode PhysicsSave: {error}"))?;
            let decoded: PhysicsSave = serde_json::from_str(&json)
                .map_err(|error| format!("decode PhysicsSave: {error}"))?;
            if decoded != save {
                return Err("decoded PhysicsSave differs from the live save".into());
            }
            (json.len(), decoded)
        };
        let reloaded = {
            let fixture = self.fixture.as_ref().ok_or("fixture unavailable")?;
            let mut reloaded = Physics::new(&fixture.world);
            reloaded
                .load(&decoded)
                .map_err(|error| format!("load decoded save: {error}"))?;
            if reloaded.body_count() != MAX_BODIES
                || reloaded.constraint_count() != 0
                || reloaded.collider_count() != colliders(MAX_BODIES)
            {
                return Err(format!(
                    "reloaded counts bodies={} welds={} colliders={}, expected {MAX_BODIES}/0/{}",
                    reloaded.body_count(),
                    reloaded.constraint_count(),
                    reloaded.collider_count(),
                    colliders(MAX_BODIES)
                ));
            }
            snapshots_match(&decoded.snapshot, &reloaded.snapshot(), 1e-5)?;
            reloaded
        };
        // The welded structure is preserved by the same persisted contract.
        let restored_welds = {
            let fixture = self.fixture.as_ref().ok_or("fixture unavailable")?;
            let mut structure = Physics::new(&fixture.world);
            structure
                .load(&fixture.baseline)
                .map_err(|error| format!("load baseline save: {error}"))?;
            if structure.body_count() != SOURCE_BODIES
                || structure.constraint_count() != SOURCE_WELDS
            {
                return Err(format!(
                    "baseline reload bodies={} welds={}, expected {SOURCE_BODIES}/{SOURCE_WELDS}",
                    structure.body_count(),
                    structure.constraint_count()
                ));
            }
            if structure.save().constraints != fixture.baseline.constraints {
                return Err("baseline reload changed the weld topology".into());
            }
            structure.constraint_count()
        };
        self.fixture.as_mut().ok_or("fixture unavailable")?.physics = reloaded;
        self.sync_dynamic()?;
        let boxes = self.mesh_boxes()?;
        Ok(vec![format!(
            "phase=3 serialize-reload json_bytes={json_bytes} decoded_equal=true reloaded_bodies={MAX_BODIES} reloaded_welds=0 reloaded_colliders={} baseline_bodies={SOURCE_BODIES} baseline_welds={restored_welds} mesh_boxes={boxes}",
            colliders(MAX_BODIES)
        )])
    }

    fn apply_lifecycle_phase(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<Vec<String>, String> {
        self.check_counts("lifecycle", MAX_BODIES, 0, colliders(MAX_BODIES))?;
        let mut lines = Vec::new();
        {
            let renderer = self.renderer.as_mut().ok_or("lifecycle: no renderer")?;
            renderer.resize(0, 0);
            let result = renderer.render_with_lighting(
                view_projection(1.0),
                CAMERA_EYE,
                &Hud::new(1.0, 1.0),
                &self.lighting,
            );
            if !matches!(result, FrameResult::Retry) {
                return Err(format!(
                    "zero-extent frame returned {result:?}, expected Retry"
                ));
            }
        }
        lines.push("zero-extent=Retry (no frame presented at zero size)".into());
        self.release_renderer();
        self.internal_recreations += 1;
        self.create_renderer(event_loop)?;
        self.sync_dynamic()?;
        lines.push(format!(
            "phase=4 lifecycle-recreate internal_recreations={} renderer_epoch={} (internal recreation, not an Android HOME/RESUME event); dynamic mesh re-uploaded",
            self.internal_recreations, self.renderer_epoch
        ));
        Ok(lines)
    }

    fn apply_reset_cycle_phase(&mut self, cycle: usize) -> Result<Vec<String>, String> {
        let context = format!("reset-cycle-{cycle:02}");
        self.load_baseline(&context)?;
        self.check_source(&context)?;
        // Reset resources first: the 6-box mesh is rebuilt and uploaded before
        // the cycle fractures again.
        self.sync_dynamic()?;
        let reset_boxes = self.mesh_boxes()?;
        self.fracture_all()?;
        self.check_counts(&context, MAX_BODIES, 0, colliders(MAX_BODIES))?;
        let mass = self.check_unit_pieces(&context)?;
        self.simulate(SIM_STEPS);
        self.check_finite(&context)?;
        self.sync_dynamic()?;
        let fractured_boxes = self.mesh_boxes()?;
        // Retained CPU geometry must stay inside the declared bound and must not
        // grow after the first cycle, so the 20 resets cannot leak capacity.
        let retained = self.check_retained_bound(&context)?;
        match self.retained_bytes_after_first_cycle {
            None => self.retained_bytes_after_first_cycle = Some(retained),
            Some(first) if retained != first => {
                return Err(format!(
                    "{context}: retained dynamic mesh bytes {retained} grew from the first cycle's {first}"
                ));
            }
            Some(_) => {}
        }
        Ok(vec![format!(
            "phase={} {context} reset_bodies={SOURCE_BODIES} reset_welds={SOURCE_WELDS} reset_colliders={} reset_mesh_boxes={reset_boxes} fractured_bodies={MAX_BODIES} fractured_welds=0 fractured_colliders={} fractured_mass_kg={mass:.3} fractured_mesh_boxes={fractured_boxes} retained_bytes={retained} simulated_steps={SIM_STEPS}",
            self.phase,
            colliders(SOURCE_BODIES),
            colliders(MAX_BODIES)
        )])
    }

    fn apply_phase(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let phase = phase_for(self.phase);
        let lines = match phase {
            Phase::Source => self.apply_source_phase()?,
            Phase::LooseFracture => self.apply_loose_fracture_phase()?,
            Phase::FullFracture => self.apply_full_fracture_phase()?,
            Phase::SerializeReload => self.apply_serialize_phase()?,
            Phase::LifecycleRecreate => self.apply_lifecycle_phase(event_loop)?,
            Phase::ResetCycle(cycle) => self.apply_reset_cycle_phase(cycle)?,
        };
        for line in lines {
            self.record(line);
        }
        Ok(())
    }

    fn phase_summary(&self) -> String {
        let (bodies, constraints, colliders) = self.fixture.as_ref().map_or((0, 0, 0), |fixture| {
            (
                fixture.physics.body_count(),
                fixture.physics.constraint_count(),
                fixture.physics.collider_count(),
            )
        });
        let boxes = self.fixture.as_ref().map_or(0, |fixture| {
            fixture.mesh.mesh().vertices.len() / BOX_VERTICES
        });
        format!(
            "phase={} {} drawn={} frames uploads={} rebuilds={} mesh_boxes={boxes} bodies={bodies} constraints={constraints} colliders={colliders}",
            self.phase,
            phase_name(phase_for(self.phase)),
            self.phase_presented,
            self.phase_uploads,
            self.phase_rebuilds,
        )
    }

    /// Resets through the persisted baseline and presents one frame of the
    /// restored welded structure. `Ok(false)` means the window is not
    /// drawable yet; the caller retries on the next redraw.
    fn finalize(&mut self) -> Result<bool, String> {
        if !self.final_reset_done {
            self.load_baseline("final-reset")?;
            self.check_source("final-reset")?;
            self.final_reset_done = true;
            self.sync_dynamic()?;
            let boxes = self.mesh_boxes()?;
            self.record(format!(
                "final-reset bodies={SOURCE_BODIES} welds={SOURCE_WELDS} colliders={} voxels={SOURCE_VOXELS} mesh_boxes={boxes}",
                colliders(SOURCE_BODIES)
            ));
        }
        self.draw()
    }

    fn pass_line(&self) -> String {
        let source_colliders = colliders(SOURCE_BODIES);
        let fractured_colliders = colliders(MAX_BODIES);
        let retained = self.retained_bytes_after_first_cycle.unwrap_or(0);
        format!(
            "PASS destruction: source {SOURCE_BODIES} bodies/{SOURCE_VOXELS} voxels in {SOURCE_WELDS} persistent welds rendered; fractured to exactly {MAX_BODIES} unit pieces with finite conserved mass {SOURCE_MASS_KG:.3} kg and rendered while simulated; PhysicsSave JSON round trip reloaded {MAX_BODIES} bodies and restored the {SOURCE_WELDS}-weld structure; {RESET_CYCLES} internal reset/load cycles each returned to {SOURCE_BODIES} bodies/{source_colliders} colliders/{SOURCE_WELDS} constraints and re-fractured to {MAX_BODIES}/{fractured_colliders}/0, with retained dynamic mesh bytes {retained} <= {MESH_RETAINED_BOUND_BYTES} and no growth after the first cycle; total_presented={} uploads={} rebuilds={} internal_renderer_recreations={} platform_suspends={} platform_resumes={} zero_extent=Retry",
            self.total_presented,
            self.total_uploads,
            self.total_rebuilds,
            self.internal_recreations,
            self.platform_suspends,
            self.platform_resumes,
        )
    }
}

impl ApplicationHandler for DestructionCheck {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.renderer.is_some() {
            return;
        }
        if let Some(error) = self.init_error.take() {
            self.fail(event_loop, format!("fixture: {error}"));
            return;
        }
        if let Err(error) = self.create_renderer(event_loop) {
            self.fail(event_loop, error);
            return;
        }
        if self.platform_suspends > self.platform_resumes {
            self.platform_resumes += 1;
            self.prepared = None;
            self.record(format!(
                "platform-resumed (Android HOME/RESUME): renderer recreated; phase {} {} restarts from the saved baseline",
                self.phase,
                phase_name(phase_for(self.phase))
            ));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
                self.prepared = None;
                return;
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {}
            _ => return,
        }
        if self.finished || self.renderer.is_none() {
            return;
        }
        if self.prepared != Some(self.phase) {
            if let Err(error) = self.apply_phase(event_loop) {
                self.fail(event_loop, error);
                return;
            }
            self.prepared = Some(self.phase);
        }
        match self.draw() {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        }
        if self.phase_presented < PHASE_FRAMES {
            return;
        }
        let summary = self.phase_summary();
        self.record(summary);
        if self.phase + 1 < PHASE_COUNT {
            self.phase += 1;
            self.phase_presented = 0;
            self.phase_uploads = 0;
            self.phase_rebuilds = 0;
            self.prepared = None;
            return;
        }
        match self.finalize() {
            Ok(true) => {
                let pass = self.pass_line();
                self.record(pass);
                self.finished = true;
                self.renderer = None;
                self.window = None;
                event_loop.exit();
            }
            Ok(false) => {}
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.platform_suspends += 1;
        self.release_renderer();
        self.record(format!(
            "platform-suspended (Android HOME/RESUME): renderer released; phase {} {} will restart from the saved baseline",
            self.phase,
            phase_name(phase_for(self.phase))
        ));
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn fixture_source_is_exactly_sixty_four_voxels_in_six_welded_bodies() {
        let fixture = Fixture::new().unwrap();
        fixture.assert_source_state("test").unwrap();
        assert_eq!(fixture.physics.body_count(), SOURCE_BODIES);
        assert_eq!(fixture.physics.constraint_count(), SOURCE_WELDS);
        assert_eq!(fixture.physics.collider_count(), colliders(SOURCE_BODIES));
        assert_eq!(
            fixture.mesh.mesh().vertices.len(),
            SOURCE_BODIES * BOX_VERTICES
        );
        assert_eq!(fixture.mesh_boxes().unwrap(), SOURCE_BODIES);
    }

    #[test]
    fn loose_fracture_preserves_the_welded_structure_topology() {
        let mut fixture = Fixture::new().unwrap();
        fixture.fracture_loose().unwrap();
        fixture
            .check_counts(
                "loose",
                LOOSE_FRACTURE_BODIES,
                SOURCE_WELDS,
                colliders(LOOSE_FRACTURE_BODIES),
            )
            .unwrap();
        fixture.check_welds_match_baseline("loose").unwrap();
        fixture.mesh.update(&fixture.physics);
        assert_eq!(
            fixture.mesh.mesh().vertices.len(),
            LOOSE_FRACTURE_BODIES * BOX_VERTICES
        );
        fixture.simulate(SIM_STEPS);
        fixture.check_finite("loose").unwrap();
        // The two welds still join the surviving tower cubes and beam.
        assert_eq!(
            fixture.physics.save().constraints,
            fixture.baseline.constraints
        );
    }

    #[test]
    fn full_fracture_reaches_exactly_max_bodies_with_unit_pieces_and_conserved_mass() {
        let mut fixture = Fixture::new().unwrap();
        let source_mass = fixture.physics.body_mass(2).unwrap() + 5.0 * 3.0;
        assert!((source_mass - SOURCE_MASS_KG).abs() < 1e-4);
        fixture.fracture_all().unwrap();
        fixture
            .check_counts("fracture", MAX_BODIES, 0, colliders(MAX_BODIES))
            .unwrap();
        fixture.mesh.update(&fixture.physics);
        assert_eq!(
            fixture.mesh.mesh().vertices.len(),
            MAX_BODIES * BOX_VERTICES
        );
        assert_eq!(fixture.mesh.mesh().indices.len(), MAX_BODIES * BOX_INDICES);
        let mass = fixture.check_unit_pieces("fracture").unwrap();
        assert!(
            (mass - SOURCE_MASS_KG).abs() < 1e-4,
            "fractured mass {mass} != source {SOURCE_MASS_KG}"
        );
        let retained = fixture.check_retained_bound("fracture").unwrap();
        assert!(retained <= MESH_RETAINED_BOUND_BYTES);
        fixture.simulate(SIM_STEPS);
        fixture.check_finite("fracture").unwrap();
    }

    #[test]
    fn save_load_json_round_trip_preserves_state_and_weld_topology() {
        let mut fixture = Fixture::new().unwrap();
        fixture.fracture_all().unwrap();
        fixture.simulate(SIM_STEPS);
        let save = fixture.physics.save();
        let json = serde_json::to_string(&save).unwrap();
        let decoded: PhysicsSave = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, save);
        let mut reloaded = Physics::new(&fixture.world);
        reloaded.load(&decoded).unwrap();
        assert_eq!(reloaded.body_count(), MAX_BODIES);
        assert_eq!(reloaded.constraint_count(), 0);
        assert_eq!(reloaded.collider_count(), colliders(MAX_BODIES));
        snapshots_match(&decoded.snapshot, &reloaded.snapshot(), 1e-5).unwrap();

        let mut structure = Physics::new(&fixture.world);
        structure.load(&fixture.baseline).unwrap();
        assert_eq!(structure.body_count(), SOURCE_BODIES);
        assert_eq!(structure.constraint_count(), SOURCE_WELDS);
        assert_eq!(structure.save().constraints, fixture.baseline.constraints);
    }

    #[test]
    fn twenty_reset_cycles_preserve_counts_topology_and_retained_bytes() {
        let mut fixture = Fixture::new().unwrap();
        let mut retained_after_first = None;
        for cycle in 0..RESET_CYCLES {
            let context = format!("cycle-{cycle:02}");
            fixture.physics.load(&fixture.baseline).unwrap();
            fixture.assert_source_state(&context).unwrap();
            fixture.mesh.update(&fixture.physics);
            assert_eq!(fixture.mesh_boxes().unwrap(), SOURCE_BODIES);
            fixture.fracture_all().unwrap();
            fixture
                .check_counts(&context, MAX_BODIES, 0, colliders(MAX_BODIES))
                .unwrap();
            fixture.check_unit_pieces(&context).unwrap();
            fixture.simulate(SIM_STEPS);
            fixture.check_finite(&context).unwrap();
            fixture.mesh.update(&fixture.physics);
            assert_eq!(fixture.mesh_boxes().unwrap(), MAX_BODIES);
            let retained = fixture.check_retained_bound(&context).unwrap();
            match retained_after_first {
                None => retained_after_first = Some(retained),
                Some(first) => assert_eq!(
                    retained, first,
                    "cycle {cycle}: retained bytes grew after the first cycle"
                ),
            }
        }
        fixture.physics.load(&fixture.baseline).unwrap();
        fixture.assert_source_state("final").unwrap();
        fixture.mesh.update(&fixture.physics);
        assert_eq!(fixture.mesh_boxes().unwrap(), SOURCE_BODIES);
    }

    #[test]
    fn full_fracture_is_deterministic_and_does_not_mutate_on_refusal() {
        let (mut first, mut second) = (Fixture::new().unwrap(), Fixture::new().unwrap());
        first.fracture_all().unwrap();
        second.fracture_all().unwrap();
        assert_eq!(first.physics.snapshot(), second.physics.snapshot());
        // At the cap every body is a single voxel, so no split is possible and
        // a refused fracture must leave the state untouched.
        let before = first.physics.snapshot();
        let target = before.bodies[0].position;
        assert!(!first.break_at(target));
        assert_eq!(first.physics.snapshot(), before);
        assert_eq!(first.physics.body_count(), MAX_BODIES);
    }

    #[test]
    fn phase_plan_covers_source_fracture_persistence_lifecycle_and_twenty_cycles() {
        assert_eq!(PHASE_COUNT, 25);
        assert_eq!(PHASE_COUNT, RESET_CYCLE_FIRST + RESET_CYCLES);
        assert!(matches!(phase_for(0), Phase::Source));
        assert!(matches!(phase_for(1), Phase::LooseFracture));
        assert!(matches!(phase_for(2), Phase::FullFracture));
        assert!(matches!(phase_for(3), Phase::SerializeReload));
        assert!(matches!(phase_for(4), Phase::LifecycleRecreate));
        assert!(matches!(phase_for(RESET_CYCLE_FIRST), Phase::ResetCycle(0)));
        assert!(matches!(
            phase_for(PHASE_COUNT - 1),
            Phase::ResetCycle(index) if index == RESET_CYCLES - 1
        ));
        let names: BTreeSet<String> = (0..PHASE_COUNT).map(|i| phase_name(phase_for(i))).collect();
        assert_eq!(
            names.len(),
            PHASE_COUNT,
            "every phase index needs a unique report name"
        );
    }

    #[test]
    fn diagnostic_overlay_reports_phase_cycle_and_live_counts() {
        let lines = diagnostic_lines(
            0,
            SOURCE_BODIES,
            SOURCE_WELDS,
            colliders(SOURCE_BODIES),
            7,
            1,
        );
        assert_eq!(lines[0], "phase 0/24 source");
        assert_eq!(lines[1], "bodies 6  welds 2  colliders 23");
        assert_eq!(lines[2], "drawn 7  epoch 1");
        let cycle = diagnostic_lines(
            RESET_CYCLE_FIRST + 7,
            MAX_BODIES,
            0,
            colliders(MAX_BODIES),
            480,
            2,
        );
        assert_eq!(cycle[0], "phase 12/24 reset-cycle-07");
        assert_eq!(cycle[1], "bodies 64  welds 0  colliders 81");
    }
}
