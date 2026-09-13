//! Native MeshProxy lighting publication diagnostic (D3.1/D3.2 positive gate).
//!
//! Drives the real Vulkan renderer through the production publication entries:
//! a [`MeshProxy`] is built from the same prototype/instance geometry uploaded
//! to the GPU, and matching-proxy GI ([`IndirectVolume`]) plus reflection
//! ([`ReflectionVolume`]) publication is demonstrated, invalidated by a real
//! instance move, republished, then removed back to world-only `None`
//! compatibility. Every phase also presents frames carrying a visible phase
//! label, so the lead can capture them on device.
//!
//! The CPU oracles (indirect face samples, [`reflect_sample_with_mesh`] plus
//! [`shade_sample`]) discriminate the actual response in every phase; the
//! renderer's enabled flags are recorded but never asserted as correctness.
//! A static control object and a control reflection ray pin down drift.
//!
//! Known limits, kept explicit and unsolved here:
//!
//! * The proxy digest is cell-resolution: a fractional move that keeps every
//!   triangle inside the cells it already occupied leaves the digest unchanged,
//!   and cached output for that representation stays valid. Pinned by
//!   `subcell_translation_keeps_the_proxy_identity`.
//! * A surface strictly inside its own proxy cell self-terminates its CPU
//!   oracle reflection as a miss (the shipped start-cell rule). Pinned by
//!   `fractional_self_hit_terminates_as_a_miss`.
//! * Neither limit is worked around with a one-cell skip or any other raster
//!   hack: the proxy build is used exactly as shipped.
//!
//! Owns no world save and never reads or writes user data. The fixture world
//! lives in memory; the only file touched is the report beside the
//! caller-provided path. No performance is measured.

use glam::{Mat4, Vec3};
use matterweave_core::{Mesh, World};
use matterweave_render::indirect::{IndirectVolume, MeshGeometry, MeshProxy, UpdateBudget};
use matterweave_render::reflection::{
    reflect_sample_with_mesh, shade_sample, MaterialTable, ReflectionVolume, DEFAULT_TRACE_STEPS,
    SURFACE_OFFSET,
};
use matterweave_render::{FrameResult, Hud, LightingSettings, Renderer, StaticInstance, Sun};
use std::{
    future::Future,
    path::PathBuf,
    sync::Arc,
    task::{Context, Poll, Waker},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

const FLOOR: u8 = 1;
const WALL: u8 = 2;
const OBJECT: u8 = 3;
const ORIGIN: [i32; 3] = [-5, -2, -5];
const DIMENSIONS: [u32; 3] = [11, 6, 11];
const SAMPLES: u32 = 64;
const GATHER_DISTANCE: f32 = 16.;
const EPOCH: u64 = 0;
const OBJECT_CELL: [i32; 3] = [0, 0, 0];
const LIFTED_CELL: [i32; 3] = [0, 2, 0];
const CONTROL_CELL: [i32; 3] = [-3, 0, -3];
/// Faces whose hemispheres contain the sunlit floor for a cube standing on it.
const SIDE_FACES: [usize; 4] = [0, 1, 4, 5];
/// The control face gathers toward -X while every moved object lies at `x >= 0`
/// under a vertical sun, so no ray of this face can reach the move.
const CONTROL_FACE: usize = 1;
/// Mirror probe: +X face of the column cell `[-4, 0, 0]`, eye east of the face
/// so the reflected ray travels +X toward the object cell.
const MIRROR_POINT: [f32; 3] = [-2.999, 0.5, 0.5];
const MIRROR_NORMAL: [f32; 3] = [1., 0., 0.];
const MIRROR_EYE: [f32; 3] = [3., 0.5, 0.5];
/// Control reflection: -X face of the same column, eye west, so the reflected
/// ray travels -X away from every object position and the volume wall.
const CONTROL_POINT: [f32; 3] = [-4.001, 0.5, 0.5];
const CONTROL_NORMAL: [f32; 3] = [-1., 0., 0.];
const CONTROL_EYE: [f32; 3] = [-8., 0.5, 0.5];
const CAMERA_EYE: [f32; 3] = [7., 5., 12.];
const CAMERA_TARGET: [f32; 3] = [0., 1., 0.];

#[cfg(target_os = "android")]
const PHASE_FRAMES: u32 = 60;
#[cfg(not(target_os = "android"))]
const PHASE_FRAMES: u32 = 6;
const PHASE_COUNT: usize = 5;

fn phase_name(phase: usize) -> &'static str {
    match phase {
        0 => "world-only-rejected",
        1 => "matched-publish",
        2 => "moved-invalidates",
        3 => "recompute-republish",
        4 => "removed-world-only",
        _ => "unknown",
    }
}

fn sun() -> Sun {
    Sun {
        direction_to_sun: [0., 1., 0.],
        intensity: 1.,
    }
}

fn palette() -> [[f32; 3]; 256] {
    let mut palette = [[0.5; 3]; 256];
    palette[0] = [0.; 3];
    palette[FLOOR as usize] = [0.9, 0.05, 0.02];
    palette[WALL as usize] = [0.5; 3];
    palette[OBJECT as usize] = [0.05, 0.8, 0.1];
    palette
}

fn table() -> MaterialTable {
    let mut table = MaterialTable::new(palette()).unwrap();
    table.set_mirror(WALL, 1.0).unwrap();
    table
}

/// Sunlit floor plus one unit-voxel receiver column at `x = -4`, `y in 0..=2`.
fn fixture_world() -> World {
    let mut world = World::new(20260907);
    for x in -5..5 {
        for z in -5..5 {
            world.set([x, -1, z], FLOOR);
        }
    }
    for y in 0..=2 {
        world.set([-4, y, 0], WALL);
    }
    world
}

/// One unit cube: exactly the geometry the engine's own voxel mesher produces
/// for a single cell, so this is a detail prototype, not synthetic soup.
fn prototype() -> Mesh {
    let mut voxel = World::new(0);
    voxel.set([0, 0, 0], 1);
    voxel.mesh()
}

/// Source cell of one world-mesh vertex: the solid cell whose exposed face
/// with this normal emitted it. The naive `floor(p - n * eps)` lookup fails on
/// voxel-boundary edges (e.g. the top edge of a side face resolves one cell
/// too high, into air), so boundary axes try both adjacent cells and keep a
/// solid cell whose face along the normal is actually exposed. Deterministic:
/// the naive cell first, then ascending neighbours. Exact for this fixture,
/// where same-material neighbours share every ambiguous edge.
fn source_cell(world: &World, position: [f32; 3], normal: [f32; 3]) -> Result<[i32; 3], String> {
    let biased = std::array::from_fn(|a| position[a] - normal[a] * SURFACE_OFFSET);
    let base = biased.map(|v| v.floor() as i32);
    let mut candidates = vec![base];
    for axis in 0..3 {
        if biased[axis].fract() == 0.0 {
            let mut extra = Vec::new();
            for cell in &candidates {
                let mut below = *cell;
                below[axis] -= 1;
                extra.push(below);
            }
            candidates.extend(extra);
        }
    }
    candidates.sort();
    candidates.dedup();
    // Keep the naive lookup first: it is the common interior case.
    candidates.sort_by_key(|cell| if *cell == base { 0 } else { 1 });
    let step = normal.map(|v| v as i32);
    for cell in candidates {
        if world.get(cell) == 0 {
            continue;
        }
        let neighbour = std::array::from_fn(|a| cell[a] + step[a]);
        if world.get(neighbour) == 0 {
            return Ok(cell);
        }
    }
    Err(format!(
        "world mesh vertex has no exposed source face: {position:?}"
    ))
}

/// Authoritative fixture-world raster geometry with fixture-consistent colours.
/// `World::mesh` paints engine-default material colours, so every vertex is
/// recolored from the CPU fixture palette through its own source cell.
/// An empty mesh is an error: the floor and receiver column must reach the
/// rasterizer, otherwise the lighting publication would cover instance
/// geometry floating over a missing world.
fn world_mesh(world: &World) -> Result<Mesh, String> {
    let mut mesh = world.mesh();
    if mesh.vertices.is_empty() {
        return Err("fixture world produced no raster geometry".into());
    }
    let colors = palette();
    for vertex in &mut mesh.vertices {
        let cell = source_cell(world, vertex.position, vertex.normal)?;
        vertex.color = colors[world.get(cell) as usize];
    }
    Ok(mesh)
}

/// Truthful one-line proxy identity for phase evidence: the digest plus the
/// actual occupied-cell count, never a packed material id under a count label.
fn describe_proxy(digest: u64, occupied_cells: usize) -> String {
    format!("proxy digest={digest} occupied_cells={occupied_cells}")
}

fn placement(cell: [i32; 3]) -> StaticInstance {
    StaticInstance {
        prototype: 0,
        translation: cell.map(|v| v as f32),
        yaw_quarters: 0,
    }
}

fn rest_instances() -> Vec<StaticInstance> {
    vec![placement(OBJECT_CELL), placement(CONTROL_CELL)]
}

fn proxy_for(
    meshes: &[Mesh],
    instances: &[StaticInstance],
    materials: &[u8],
) -> Result<MeshProxy, String> {
    MeshProxy::build(
        &MeshGeometry {
            meshes,
            instances,
            materials,
        },
        ORIGIN,
        DIMENSIONS,
    )
}

fn volume_with(proxy: MeshProxy, world: &World) -> Result<IndirectVolume, String> {
    let mut volume = IndirectVolume::new(ORIGIN, DIMENSIONS, SAMPLES, GATHER_DISTANCE, palette())?;
    volume.set_mesh_proxy(Some(proxy));
    finish(&mut volume, world);
    Ok(volume)
}

fn finish(volume: &mut IndirectVolume, world: &World) {
    for _ in 0..1000 {
        let stats = volume
            .update(
                world,
                EPOCH,
                sun(),
                UpdateBudget {
                    rays: 4096,
                    work: 4096,
                },
            )
            .unwrap();
        if stats.complete {
            return;
        }
    }
    panic!("bounded fixture never completed");
}

fn pack_with(world: &World, proxy: Option<&MeshProxy>) -> Result<ReflectionVolume, String> {
    match proxy {
        Some(proxy) => ReflectionVolume::pack_with_mesh(
            world,
            EPOCH,
            ORIGIN,
            DIMENSIONS,
            &table(),
            DEFAULT_TRACE_STEPS,
            proxy,
        ),
        None => ReflectionVolume::pack(
            world,
            EPOCH,
            ORIGIN,
            DIMENSIONS,
            &table(),
            DEFAULT_TRACE_STEPS,
        ),
    }
}

/// The diagnostic overlay text: phase identification plus the publication flags
/// behind the frame. Kept pure so the on-device strings are unit covered.
fn diagnostic_lines(phase: usize, presented: u32, indirect: bool, reflection: bool) -> [String; 3] {
    [
        format!(
            "MESH LIGHTING phase {phase}/{} {}",
            PHASE_COUNT - 1,
            phase_name(phase)
        ),
        format!("presented {presented}"),
        format!("gi={indirect} reflection={reflection}"),
    ]
}

fn view_projection(aspect: f32) -> [[f32; 4]; 4] {
    (Mat4::perspective_rh(60f32.to_radians(), aspect, 0.1, 200.)
        * Mat4::look_at_rh(
            Vec3::from_array(CAMERA_EYE),
            Vec3::from_array(CAMERA_TARGET),
            Vec3::Y,
        ))
    .to_cols_array_2d()
}

pub(crate) struct MeshLightingCheck {
    report_path: PathBuf,
    report: Vec<String>,
    world: World,
    meshes: Vec<Mesh>,
    instances: Vec<StaticInstance>,
    materials: Vec<u8>,
    lighting: LightingSettings,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    rest_volume: Option<IndirectVolume>,
    rest_pack: Option<ReflectionVolume>,
    rest_digest: Option<u64>,
    lifted_digest: Option<u64>,
    phase: usize,
    phase_presented: u32,
    total_presented: u32,
    prepared: Option<usize>,
    evidence: Vec<String>,
    capabilities_recorded: bool,
    suspends: u32,
    resumes: u32,
    finished: bool,
    failed: bool,
}

impl MeshLightingCheck {
    pub(crate) fn new(report_path: PathBuf) -> Self {
        let mut check = Self {
            report_path,
            report: vec![
                "Matterweave mesh lighting check report (standalone; no user save is read or written)"
                    .into(),
                "coverage: proxy-less rejection while mesh geometry is resident; matching-proxy GI/reflection publication with CPU-oracle response; moved-instance invalidation with stale-pack rejection; recompute and republish; removal back to world-only None. Not covered: sustained performance, thermal behavior, user saves, process death, sub-cell movement identity, fractional self-reflection".into(),
            ],
            world: fixture_world(),
            meshes: vec![prototype()],
            instances: rest_instances(),
            materials: vec![OBJECT],
            lighting: LightingSettings {
                sun: sun(),
                ..Default::default()
            },
            renderer: None,
            window: None,
            rest_volume: None,
            rest_pack: None,
            rest_digest: None,
            lifted_digest: None,
            phase: 0,
            phase_presented: 0,
            total_presented: 0,
            prepared: None,
            evidence: Vec::new(),
            capabilities_recorded: false,
            suspends: 0,
            resumes: 0,
            finished: false,
            failed: false,
        };
        check.write_report();
        check
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    fn write_report(&mut self) {
        if let Err(error) = std::fs::write(&self.report_path, self.report.join("\n") + "\n") {
            // The report sink itself is unavailable: log the failure through the
            // platform logger and terminate with a failed gate, without panicking
            // or recursively trying to write another report.
            log::error!("FAIL mesh-lighting: report write failed: {error}");
            self.failed = true;
            self.finished = true;
        }
    }

    fn record(&mut self, entry: String) {
        if self.failed {
            return;
        }
        log::info!("{entry}");
        #[cfg(not(target_os = "android"))]
        eprintln!("{entry}");
        self.report.push(entry);
        self.write_report();
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.record(format!("FAIL mesh-lighting: {error}"));
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
                        .with_title("Matterweave mesh lighting gate")
                        .with_inner_size(winit::dpi::PhysicalSize::new(960, 720)),
                )
                .map_err(|error| format!("create window: {error}"))?,
        );
        let mut init = Box::pin(Renderer::new(window.clone()));
        let Poll::Ready(renderer) = init.as_mut().poll(&mut Context::from_waker(Waker::noop()))
        else {
            return Err("renderer initialization suspended".into());
        };
        let mut renderer = renderer.map_err(|error| format!("renderer init: {error}"))?;
        // The CPU fixture world must reach the rasterizer on every renderer
        // creation, including resume: without this upload the floor/receiver
        // exist only in the lighting volumes while the frames show instances
        // over a missing world. Static proxy/instance geometry is still
        // paired separately in each phase via `sync_static_scene`.
        let world = world_mesh(&self.world)?;
        let (vertices, indices) = (world.vertices.len(), world.indices.len());
        renderer
            .upload(&world)
            .map_err(|error| format!("fixture world upload: {error}"))?;
        self.record(format!(
            "world geometry uploaded: {vertices} vertices {indices} indices (fixture palette)"
        ));
        if !self.capabilities_recorded {
            self.record(format!("capabilities: {}", renderer.capabilities));
            self.capabilities_recorded = true;
        }
        self.renderer = Some(renderer);
        self.window = Some(window);
        Ok(())
    }

    fn release_renderer(&mut self) {
        self.renderer = None;
        self.window = None;
    }

    fn renderer(&mut self) -> Result<(), String> {
        if self.renderer.is_none() {
            return Err("no renderer: platform has not resumed".into());
        }
        Ok(())
    }

    /// Uploads the current instance/prototype geometry to the GPU. Always a
    /// full replace: idempotent, transactional, and safe to repeat after a
    /// platform resume that dropped GPU state.
    fn sync_static_scene(&mut self) -> Result<(), String> {
        self.renderer()?;
        let renderer = self.renderer.as_mut().expect("renderer");
        renderer
            .replace_static_scene(&self.meshes, &self.instances)
            .map_err(|error| format!("static scene upload: {error}"))?;
        Ok(())
    }

    /// Honest GPU residency for phase evidence: legacy world bytes plus the
    /// resident static prototype/instance counts behind the frame.
    fn gpu_geometry_line(&self) -> String {
        match self.renderer.as_ref() {
            Some(renderer) => match renderer.static_scene_stats() {
                Some(stats) => format!(
                    "gpu mesh_bytes={} static prototypes={} instances={} vertices={} indices={}",
                    renderer.mesh_bytes,
                    stats.prototypes,
                    stats.instances,
                    stats.vertices,
                    stats.indices
                ),
                None => format!(
                    "gpu mesh_bytes={} static scene cleared",
                    renderer.mesh_bytes
                ),
            },
            None => "gpu no renderer".into(),
        }
    }

    fn check_oracle_rest(&self, volume: &IndirectVolume) -> Result<String, String> {
        for face in SIDE_FACES {
            let sample = volume.sample(OBJECT_CELL, face);
            if !(sample[0] > 0.05 && sample[0] > 10. * sample[1] && sample[0] > 10. * sample[2]) {
                return Err(format!(
                    "rest object face {face} must gather the sunlit red floor, got {sample:?}"
                ));
            }
        }
        if volume.sample(OBJECT_CELL, 2) != [0.; 3] {
            return Err("rest object +Y gathers upward and must stay dark".into());
        }
        Ok(format!(
            "oracle rest side faces red-dominated: {:?}",
            SIDE_FACES.map(|face| volume.sample(OBJECT_CELL, face))
        ))
    }

    fn check_oracle_moved(
        &self,
        rest: &IndirectVolume,
        lifted: &IndirectVolume,
    ) -> Result<String, String> {
        for face in SIDE_FACES {
            let at_rest = rest.sample(OBJECT_CELL, face)[0];
            let raised = lifted.sample(LIFTED_CELL, face)[0];
            if at_rest <= 0.05 {
                return Err(format!("rest face {face} response {at_rest}"));
            }
            if raised >= at_rest {
                return Err(format!(
                    "face {face}: lifting away from the floor must lower the response, {raised} vs {at_rest}"
                ));
            }
        }
        for face in 0..6 {
            if lifted.sample(OBJECT_CELL, face) != [0.; 3] {
                return Err(format!("the vacated cell is air again: face {face}"));
            }
        }
        if lifted.sample(CONTROL_CELL, CONTROL_FACE) != rest.sample(CONTROL_CELL, CONTROL_FACE) {
            return Err("the static control object must not drift".into());
        }
        if lifted.sample(CONTROL_CELL, 0) == rest.sample(CONTROL_CELL, 0) {
            return Err(
                "a control face whose hemisphere holds the object must see the move".into(),
            );
        }
        Ok("oracle moved: lifted response lower, vacated cell zero, control holds".into())
    }

    fn check_reflection_rest(&self, proxy: &MeshProxy) -> Result<String, String> {
        let pack = pack_with(&self.world, Some(proxy))?;
        if pack.material_at(OBJECT_CELL) != OBJECT {
            return Err("the packed grid must carry the proxy cell".into());
        }
        let sample = reflect_sample_with_mesh(
            &pack,
            &self.world,
            proxy,
            MIRROR_POINT,
            MIRROR_NORMAL,
            MIRROR_EYE,
        );
        if !sample.hit || sample.cell != OBJECT_CELL || sample.material != OBJECT {
            return Err(format!(
                "the mirror ray must hit the rest object, got {sample:?}"
            ));
        }
        let shaded = shade_sample(&pack, &sample, sun()).map_err(|e| e.to_string())?;
        if !(shaded[1] > 0.05 && shaded[1] > 5. * shaded[0] && shaded[1] > 5. * shaded[2]) {
            return Err(format!(
                "the shaded rest reflection must carry the green albedo, got {shaded:?}"
            ));
        }
        let control = reflect_sample_with_mesh(
            &pack,
            &self.world,
            proxy,
            CONTROL_POINT,
            CONTROL_NORMAL,
            CONTROL_EYE,
        );
        if control.hit {
            return Err(format!(
                "the away-facing control ray must miss, got {control:?}"
            ));
        }
        Ok(format!(
            "oracle reflection rest hit {:?}/{}, shaded {shaded:?}, control miss",
            sample.cell, sample.material
        ))
    }

    fn check_reflection_lifted(
        &self,
        rest_proxy: &MeshProxy,
        lifted_proxy: &MeshProxy,
    ) -> Result<String, String> {
        let lifted_pack = pack_with(&self.world, Some(lifted_proxy))?;
        let sample = reflect_sample_with_mesh(
            &lifted_pack,
            &self.world,
            lifted_proxy,
            MIRROR_POINT,
            MIRROR_NORMAL,
            MIRROR_EYE,
        );
        if sample.hit {
            return Err(format!(
                "the lifted object must leave the mirror ray, got {sample:?}"
            ));
        }
        let rest_pack = pack_with(&self.world, Some(rest_proxy))?;
        let rest_control = reflect_sample_with_mesh(
            &rest_pack,
            &self.world,
            rest_proxy,
            CONTROL_POINT,
            CONTROL_NORMAL,
            CONTROL_EYE,
        );
        let lifted_control = reflect_sample_with_mesh(
            &lifted_pack,
            &self.world,
            lifted_proxy,
            CONTROL_POINT,
            CONTROL_NORMAL,
            CONTROL_EYE,
        );
        if rest_control != lifted_control {
            return Err(
                "the control reflection must be identical before and after the move".into(),
            );
        }
        Ok("oracle reflection lifted miss, control identical".into())
    }

    fn apply_world_only_rejected(&mut self) -> Result<(), String> {
        self.instances = rest_instances();
        self.sync_static_scene()?;
        let mut volume =
            IndirectVolume::new(ORIGIN, DIMENSIONS, SAMPLES, GATHER_DISTANCE, palette())
                .map_err(|e| e.to_string())?;
        finish(&mut volume, &self.world);
        for face in 0..6 {
            if volume.sample(OBJECT_CELL, face) != [0.; 3] {
                return Err(format!("an air cell has no sampled face: face {face}"));
            }
        }
        let renderer = self.renderer.as_mut().expect("renderer");
        let indirect_error = match renderer.upload_indirect(&volume, &self.world, EPOCH, None) {
            Ok(()) => return Err("proxy-less GI published over resident mesh geometry".into()),
            Err(error) => error,
        };
        if !indirect_error.contains("mesh-only") {
            return Err(format!("unexpected GI rejection: {indirect_error}"));
        }
        let pack = pack_with(&self.world, None)?;
        let reflection_error = match renderer.upload_reflection(&pack, &self.world, EPOCH, None) {
            Ok(_) => {
                return Err("proxy-less reflection published over resident mesh geometry".into());
            }
            Err(error) => error,
        };
        if !reflection_error.contains("mesh-only") {
            return Err(format!(
                "unexpected reflection rejection: {reflection_error}"
            ));
        }
        if renderer.indirect_enabled() || renderer.reflection_enabled() {
            return Err("rejected publication must leave lighting disabled".into());
        }
        self.evidence = vec![
            format!("gi rejected: {indirect_error}"),
            format!("reflection rejected: {reflection_error}"),
            "oracle world-only object cell exactly zero on all faces".into(),
            "gi=false reflection=false after rejection".into(),
        ];
        Ok(())
    }

    fn apply_matched_publish(&mut self) -> Result<(), String> {
        self.instances = rest_instances();
        self.sync_static_scene()?;
        let proxy = proxy_for(&self.meshes, &self.instances, &self.materials)?;
        let digest = proxy.digest();
        let occupied = proxy.occupied_cells();
        if occupied == 0 {
            return Err("the rest proxy marks no cell".into());
        }
        let oracle = self.check_oracle_rest(&volume_with(
            proxy_for(&self.meshes, &self.instances, &self.materials)?,
            &self.world,
        )?)?;
        let reflection_oracle = self.check_reflection_rest(&proxy_for(
            &self.meshes,
            &self.instances,
            &self.materials,
        )?)?;
        let volume = volume_with(proxy, &self.world)?;
        let pack = pack_with(
            &self.world,
            Some(&proxy_for(&self.meshes, &self.instances, &self.materials)?),
        )?;
        let renderer = self.renderer.as_mut().expect("renderer");
        renderer
            .upload_indirect(&volume, &self.world, EPOCH, Some(digest))
            .map_err(|error| format!("matching GI publish: {error}"))?;
        let stats = renderer
            .upload_reflection(&pack, &self.world, EPOCH, Some(digest))
            .map_err(|error| format!("matching reflection publish: {error}"))?;
        if !renderer.indirect_enabled() || !renderer.reflection_enabled() {
            return Err("matching publication must enable lighting".into());
        }
        self.evidence = vec![
            describe_proxy(digest, occupied),
            oracle,
            reflection_oracle,
            format!("published {} reflection bytes", stats.bytes),
            "gi=true reflection=true".into(),
        ];
        self.rest_digest = Some(digest);
        // Rebuild owned rest representations from the same slices for the move
        // phase. The build is deterministic, so the stored identity equals the
        // live one (asserted by the unit tests).
        let stored_proxy = proxy_for(&self.meshes, &self.instances, &self.materials)?;
        self.rest_volume = Some(volume_with(stored_proxy, &self.world)?);
        let stored_proxy = proxy_for(&self.meshes, &self.instances, &self.materials)?;
        self.rest_pack = Some(pack_with(&self.world, Some(&stored_proxy))?);
        Ok(())
    }

    fn apply_moved_invalidates(&mut self) -> Result<(), String> {
        let rest_digest = self
            .rest_digest
            .ok_or("moved phase needs the rest digest from the publish phase")?;
        self.instances = rest_instances();
        self.instances[0] = placement(LIFTED_CELL);
        self.sync_static_scene()?;
        let lifted_proxy = proxy_for(&self.meshes, &self.instances, &self.materials)?;
        let lifted_digest = lifted_proxy.digest();
        if lifted_digest == rest_digest {
            return Err("a moved instance must change the proxy identity".into());
        }
        self.lifted_digest = Some(lifted_digest);
        let (rest_volume, rest_pack) = match (&self.rest_volume, &self.rest_pack) {
            (Some(volume), Some(pack)) => (volume, pack),
            _ => return Err("moved phase needs the stored rest publication".into()),
        };
        let renderer = self.renderer.as_mut().expect("renderer");
        let indirect_error =
            match renderer.upload_indirect(rest_volume, &self.world, EPOCH, Some(lifted_digest)) {
                Ok(()) => return Err("stale GI published under the new digest".into()),
                Err(error) => error,
            };
        let reflection_error =
            match renderer.upload_reflection(rest_pack, &self.world, EPOCH, Some(lifted_digest)) {
                Ok(_) => return Err("stale reflection published under the new digest".into()),
                Err(error) => error,
            };
        if renderer.indirect_enabled() || renderer.reflection_enabled() {
            return Err("stale rejection must disable previous lighting".into());
        }
        let rest_proxy = proxy_for(&self.meshes, &rest_instances(), &self.materials)?;
        let oracle = self.check_oracle_moved(
            &volume_with(rest_proxy, &self.world)?,
            &volume_with(lifted_proxy, &self.world)?,
        )?;
        let rest_proxy = proxy_for(&self.meshes, &rest_instances(), &self.materials)?;
        let moved_instances = vec![placement(LIFTED_CELL), placement(CONTROL_CELL)];
        let lifted_proxy = proxy_for(&self.meshes, &moved_instances, &self.materials)?;
        let reflection_oracle = self.check_reflection_lifted(&rest_proxy, &lifted_proxy)?;
        self.evidence = vec![
            format!("rest digest={rest_digest} lifted digest={lifted_digest}"),
            format!("stale GI rejected: {indirect_error}"),
            format!("stale reflection rejected: {reflection_error}"),
            oracle,
            reflection_oracle,
            "gi=false reflection=false after stale rejection".into(),
        ];
        Ok(())
    }

    fn apply_recompute_republish(&mut self) -> Result<(), String> {
        let lifted_digest = self
            .lifted_digest
            .ok_or("republish phase needs the lifted digest from the move phase")?;
        self.instances = vec![placement(LIFTED_CELL), placement(CONTROL_CELL)];
        self.sync_static_scene()?;
        let proxy = proxy_for(&self.meshes, &self.instances, &self.materials)?;
        if proxy.digest() != lifted_digest {
            return Err("recomputed proxy must carry the move identity".into());
        }
        let volume = volume_with(proxy, &self.world)?;
        for face in SIDE_FACES {
            let raised = volume.sample(LIFTED_CELL, face)[0];
            if raised <= 0.0 {
                return Err(format!("republished lifted face {face} is dark: {raised}"));
            }
        }
        let pack = pack_with(
            &self.world,
            Some(&proxy_for(&self.meshes, &self.instances, &self.materials)?),
        )?;
        let renderer = self.renderer.as_mut().expect("renderer");
        renderer
            .upload_indirect(&volume, &self.world, EPOCH, Some(lifted_digest))
            .map_err(|error| format!("recomputed GI publish: {error}"))?;
        renderer
            .upload_reflection(&pack, &self.world, EPOCH, Some(lifted_digest))
            .map_err(|error| format!("recomputed reflection publish: {error}"))?;
        if !renderer.indirect_enabled() || !renderer.reflection_enabled() {
            return Err("recomputed publication must enable lighting".into());
        }
        self.evidence = vec![
            format!("republished digest={lifted_digest}"),
            format!(
                "oracle lifted side faces red-lit: {:?}",
                SIDE_FACES.map(|face| volume.sample(LIFTED_CELL, face))
            ),
            "gi=true reflection=true".into(),
        ];
        Ok(())
    }

    fn apply_removed_world_only(&mut self) -> Result<(), String> {
        self.sync_static_scene_empty()?;
        let mut volume =
            IndirectVolume::new(ORIGIN, DIMENSIONS, SAMPLES, GATHER_DISTANCE, palette())
                .map_err(|e| e.to_string())?;
        finish(&mut volume, &self.world);
        for face in 0..6 {
            if volume.sample(OBJECT_CELL, face) != [0.; 3]
                || volume.sample(LIFTED_CELL, face) != [0.; 3]
            {
                return Err("removed geometry leaves air cells with no samples".into());
            }
        }
        let pack = pack_with(&self.world, None)?;
        let renderer = self.renderer.as_mut().expect("renderer");
        renderer
            .upload_indirect(&volume, &self.world, EPOCH, None)
            .map_err(|error| format!("world-only GI publish: {error}"))?;
        renderer
            .upload_reflection(&pack, &self.world, EPOCH, None)
            .map_err(|error| format!("world-only reflection publish: {error}"))?;
        if !renderer.indirect_enabled() || !renderer.reflection_enabled() {
            return Err("world-only None publication must enable lighting".into());
        }
        self.evidence = vec![
            "static scene cleared; world-only None GI and reflection published".into(),
            "oracle removed cells exactly zero on all faces".into(),
            "gi=true reflection=true".into(),
        ];
        Ok(())
    }

    fn sync_static_scene_empty(&mut self) -> Result<(), String> {
        self.renderer()?;
        let renderer = self.renderer.as_mut().expect("renderer");
        renderer
            .replace_static_scene(&self.meshes, &[])
            .map_err(|error| format!("static scene clear: {error}"))?;
        Ok(())
    }

    fn apply_phase(&mut self) -> Result<(), String> {
        // Fresh evidence per application; the completion record consumes it once,
        // so a resume-driven re-application cannot duplicate report lines.
        self.evidence = Vec::new();
        match self.phase {
            0 => self.apply_world_only_rejected(),
            1 => self.apply_matched_publish(),
            2 => self.apply_moved_invalidates(),
            3 => self.apply_recompute_republish(),
            4 => self.apply_removed_world_only(),
            other => Err(format!("unknown phase {other}")),
        }
    }

    /// Presents one frame of the current phase. Zero extent and swapchain Retry
    /// present nothing and are never counted; fatal draws are errors.
    fn draw(&mut self) -> Result<bool, String> {
        let Some(window) = self.window.as_ref() else {
            return Ok(false);
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(false);
        }
        let renderer = self.renderer.as_mut().expect("renderer");
        let mut hud = Hud::new(size.width as f32, size.height as f32);
        let lines = diagnostic_lines(
            self.phase,
            self.total_presented,
            renderer.indirect_enabled(),
            renderer.reflection_enabled(),
        );
        hud.rect([8.0, 8.0, 560.0, 72.0], [0.0, 0.0, 0.0, 0.72]);
        for (index, line) in lines.iter().enumerate() {
            hud.text(
                16.0,
                14.0 + index as f32 * 22.0,
                line,
                2.0,
                [1.0, 0.96, 0.72, 1.0],
            );
        }
        let result = renderer.render_with_lighting(
            view_projection(size.width as f32 / size.height as f32),
            CAMERA_EYE,
            &hud,
            &self.lighting,
        );
        match result {
            FrameResult::Presented => Ok(true),
            FrameResult::Retry => Ok(false),
            FrameResult::OutOfMemory => Err("renderer reported out of memory".into()),
            FrameResult::Fatal(error) => Err(format!("renderer fatal: {error}")),
        }
    }

    fn presented(&mut self) {
        self.phase_presented += 1;
        self.total_presented += 1;
    }

    #[cfg(test)]
    fn suspended_helper_for_test(&mut self) {
        self.suspends += 1;
        self.release_renderer();
        self.prepared = None;
    }

    /// The GPU operations are callbacks so the accounting is covered by
    /// deterministic tests with scripted Retry results, without a driver.
    fn advance(
        &mut self,
        prepare: impl FnOnce(&mut Self) -> Result<(), String>,
        mut draw: impl FnMut(&mut Self) -> Result<bool, String>,
    ) -> Result<bool, String> {
        if self.prepared != Some(self.phase) {
            prepare(self)?;
            self.prepared = Some(self.phase);
        }
        if !draw(self)? {
            return Ok(false);
        }
        self.presented();
        if self.phase_presented < PHASE_FRAMES {
            return Ok(false);
        }
        let evidence = std::mem::take(&mut self.evidence);
        let residency = self.gpu_geometry_line();
        self.record(format!(
            "phase={} {} drawn={PHASE_FRAMES} {} | {residency}",
            self.phase,
            phase_name(self.phase),
            evidence.join(" | ")
        ));
        if self.failed {
            return Err("report write failed".into());
        }
        self.phase += 1;
        self.phase_presented = 0;
        self.prepared = None;
        if self.phase >= PHASE_COUNT {
            let pass = format!(
                "PASS mesh-lighting: proxy-less world-only lighting rejected while mesh geometry is resident; matching-proxy GI/reflection published (rest digest {:?}); moved instance invalidated lighting and stale packs were rejected (lifted digest {:?}); recomputed and republished; removed geometry back to world-only None; total_presented={} suspends={} resumes={}; limits: cell-resolution proxy digest (sub-cell moves keep identity) and in-cell self-hit oracle miss, both pinned by unit tests",
                self.rest_digest,
                self.lifted_digest,
                self.total_presented,
                self.suspends,
                self.resumes,
            );
            self.record(pass);
            self.finished = true;
            self.release_renderer();
            return Ok(true);
        }
        Ok(false)
    }
}

impl ApplicationHandler for MeshLightingCheck {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.failed {
            event_loop.exit();
            return;
        }
        if self.finished || self.renderer.is_some() {
            return;
        }
        if let Err(error) = self.create_renderer(event_loop) {
            self.fail(event_loop, error);
            return;
        }
        if self.suspends > self.resumes {
            self.resumes += 1;
            self.record(format!(
                "platform-resumed: renderer recreated; phase {} {} progress preserved",
                self.phase,
                phase_name(self.phase)
            ));
        }
        // GPU state was dropped: the current phase must re-apply its uploads.
        self.prepared = None;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size.width, size.height);
                }
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
        match self.advance(MeshLightingCheck::apply_phase, MeshLightingCheck::draw) {
            Ok(true) => event_loop.exit(),
            Ok(false) => {}
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.suspends += 1;
        self.release_renderer();
        // Simulation progress (phase, presented counts, stored packs) survives;
        // only GPU uploads are re-applied after resume.
        self.prepared = None;
        self.record(format!(
            "platform-suspended: renderer released; phase {} {} progress preserved",
            self.phase,
            phase_name(self.phase)
        ));
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.failed {
            self.release_renderer();
            event_loop.exit();
            return;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Desktop entry used by `--mesh-lighting-check`. Returns whether the gate failed.
pub fn run(report_path: PathBuf) -> bool {
    let mut check = MeshLightingCheck::new(report_path);
    EventLoop::new()
        .expect("event loop")
        .run_app(&mut check)
        .expect("mesh lighting check loop");
    check.failed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_core::Vertex;

    #[test]
    fn unavailable_report_sink_fails_without_panicking() {
        // A directory is a deterministic write failure, even when tests run as root.
        let mut check = MeshLightingCheck::new(std::env::temp_dir());
        assert!(check.failed());
        assert!(check.finished);
        let entries = check.report.len();
        check.record("PASS must never be appended after a sink failure".into());
        assert_eq!(check.report.len(), entries);
    }

    #[test]
    fn report_sink_failure_during_run_marks_gate_failed() {
        let path = report("sink-failure");
        let mut check = MeshLightingCheck::new(path.clone());
        assert!(!check.failed());
        check.report_path = std::env::temp_dir();
        check.record("phase evidence".into());
        assert!(check.failed());
        assert!(check.finished);
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("PASS mesh-lighting"));
        std::fs::remove_file(path).unwrap();
    }

    fn report(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "matterweave-mesh-lighting-{}-{name}.txt",
            std::process::id()
        ))
    }

    fn with_check(name: &str, test: impl FnOnce(&mut MeshLightingCheck)) {
        let path = report(name);
        let mut check = MeshLightingCheck::new(path.clone());
        test(&mut check);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn world_mesh_uses_fixture_palette_on_every_vertex() {
        let world = fixture_world();
        let mesh = world_mesh(&world).unwrap();
        assert!(!mesh.vertices.is_empty());
        let colors = palette();
        let (mut floor, mut wall) = (0, 0);
        for vertex in &mesh.vertices {
            let cell = source_cell(&world, vertex.position, vertex.normal).unwrap();
            let material = world.get(cell);
            assert!(material == FLOOR || material == WALL, "cell {cell:?}");
            assert_eq!(vertex.color, colors[material as usize], "cell {cell:?}");
            if material == FLOOR {
                floor += 1;
            } else {
                wall += 1;
            }
        }
        assert!(floor > 0 && wall > 0, "floor {floor} wall {wall}");
        // Engine-default mesher colours must be gone: moss is material 1's
        // default, soil material 2's.
        for vertex in &mesh.vertices {
            assert_ne!(vertex.color, [0.29, 0.48, 0.27]);
            assert_ne!(vertex.color, [0.35, 0.24, 0.17]);
        }
    }

    #[test]
    fn describe_proxy_reports_digest_and_occupied_count() {
        let meshes = [prototype()];
        let materials = [OBJECT];
        let proxy = proxy_for(&meshes, &rest_instances(), &materials).unwrap();
        let line = describe_proxy(proxy.digest(), proxy.occupied_cells());
        assert_eq!(
            line,
            format!(
                "proxy digest={} occupied_cells={}",
                proxy.digest(),
                proxy.occupied_cells()
            )
        );
        assert!(line.contains("occupied_cells="));
    }

    #[test]
    fn world_only_cell_is_dark_and_proxy_response_is_red_dominated() {
        let world = fixture_world();
        let mut plain =
            IndirectVolume::new(ORIGIN, DIMENSIONS, SAMPLES, GATHER_DISTANCE, palette()).unwrap();
        finish(&mut plain, &world);
        assert!(!plain.has_mesh_proxy() && plain.mesh_digest().is_none());
        for face in 0..6 {
            assert_eq!(plain.sample(OBJECT_CELL, face), [0.; 3]);
        }
        let meshes = [prototype()];
        let materials = [OBJECT];
        let rest = proxy_for(&meshes, &rest_instances(), &materials).unwrap();
        assert!(rest.occupied_cells() > 0);
        let lit = volume_with(rest, &world).unwrap();
        assert!(lit.has_mesh_proxy() && lit.mesh_digest().is_some());
        for face in SIDE_FACES {
            let sample = lit.sample(OBJECT_CELL, face);
            assert!(
                sample[0] > 0.05 && sample[0] > 10. * sample[1] && sample[0] > 10. * sample[2],
                "face {face}: {sample:?}"
            );
        }
        assert_eq!(lit.sample(OBJECT_CELL, 2), [0.; 3]);
        // Rebuilding from the same slices reproduces the identity exactly.
        let again = proxy_for(&meshes, &rest_instances(), &materials).unwrap();
        assert_eq!(again.digest(), lit.mesh_digest().unwrap());
    }

    #[test]
    fn moved_response_lowers_and_static_control_holds() {
        let world = fixture_world();
        let meshes = [prototype()];
        let materials = [OBJECT];
        let rest = proxy_for(&meshes, &rest_instances(), &materials).unwrap();
        let moved_instances = vec![placement(LIFTED_CELL), placement(CONTROL_CELL)];
        let lifted = proxy_for(&meshes, &moved_instances, &materials).unwrap();
        assert_ne!(rest.digest(), lifted.digest());
        let rest_volume = volume_with(rest, &world).unwrap();
        let lifted_volume = volume_with(lifted, &world).unwrap();
        for face in SIDE_FACES {
            let at_rest = rest_volume.sample(OBJECT_CELL, face)[0];
            let raised = lifted_volume.sample(LIFTED_CELL, face)[0];
            assert!(at_rest > 0.05, "face {face}: {at_rest}");
            assert!(raised < at_rest, "face {face}: {raised} vs {at_rest}");
        }
        for face in 0..6 {
            assert_eq!(lifted_volume.sample(OBJECT_CELL, face), [0.; 3]);
        }
        assert_eq!(
            lifted_volume.sample(CONTROL_CELL, CONTROL_FACE),
            rest_volume.sample(CONTROL_CELL, CONTROL_FACE)
        );
        assert_ne!(
            lifted_volume.sample(CONTROL_CELL, 0),
            rest_volume.sample(CONTROL_CELL, 0)
        );
    }

    #[test]
    fn reflection_oracle_hits_rest_misses_lifted_and_control_is_identical() {
        let world = fixture_world();
        let meshes = [prototype()];
        let materials = [OBJECT];
        let rest = proxy_for(&meshes, &rest_instances(), &materials).unwrap();
        let moved_instances = vec![placement(LIFTED_CELL), placement(CONTROL_CELL)];
        let lifted = proxy_for(&meshes, &moved_instances, &materials).unwrap();
        let rest_pack = pack_with(&world, Some(&rest)).unwrap();
        assert_eq!(rest_pack.material_at(OBJECT_CELL), OBJECT);
        assert!(rest_pack.valid_for_scene(&world, EPOCH, Some(rest.digest())));
        assert!(!rest_pack.valid_for_scene(&world, EPOCH, Some(lifted.digest())));
        let hit = reflect_sample_with_mesh(
            &rest_pack,
            &world,
            &rest,
            MIRROR_POINT,
            MIRROR_NORMAL,
            MIRROR_EYE,
        );
        assert!(hit.hit && hit.cell == OBJECT_CELL && hit.material == OBJECT);
        let shaded = shade_sample(&rest_pack, &hit, sun()).unwrap();
        assert!(shaded[1] > 0.05 && shaded[1] > 5. * shaded[0] && shaded[1] > 5. * shaded[2]);
        let lifted_pack = pack_with(&world, Some(&lifted)).unwrap();
        let miss = reflect_sample_with_mesh(
            &lifted_pack,
            &world,
            &lifted,
            MIRROR_POINT,
            MIRROR_NORMAL,
            MIRROR_EYE,
        );
        assert!(!miss.hit);
        let rest_control = reflect_sample_with_mesh(
            &rest_pack,
            &world,
            &rest,
            CONTROL_POINT,
            CONTROL_NORMAL,
            CONTROL_EYE,
        );
        let lifted_control = reflect_sample_with_mesh(
            &lifted_pack,
            &world,
            &lifted,
            CONTROL_POINT,
            CONTROL_NORMAL,
            CONTROL_EYE,
        );
        assert!(!rest_control.hit);
        assert_eq!(rest_control, lifted_control);
    }

    #[test]
    fn subcell_translation_keeps_the_proxy_identity() {
        // Known cell-resolution limit: a fractional move that keeps every
        // triangle inside its cells is invisible to the digest, so cached
        // output stays valid there. This must stay explicit, never hacked
        // around with a one-cell skip.
        let mesh = Mesh {
            vertices: vec![
                Vertex {
                    position: [0.2, 0.2, 0.2],
                    normal: [0., 0., 1.],
                    color: [1., 1., 1.],
                },
                Vertex {
                    position: [0.4, 0.2, 0.2],
                    normal: [0., 0., 1.],
                    color: [1., 1., 1.],
                },
                Vertex {
                    position: [0.2, 0.4, 0.2],
                    normal: [0., 0., 1.],
                    color: [1., 1., 1.],
                },
            ],
            indices: vec![0, 1, 2],
            revision: 1,
        };
        let meshes = [mesh];
        let materials = [OBJECT];
        let here = [StaticInstance {
            prototype: 0,
            translation: [0., 0., 0.],
            yaw_quarters: 0,
        }];
        let shifted = [StaticInstance {
            prototype: 0,
            translation: [0.1, 0., 0.],
            yaw_quarters: 0,
        }];
        let a = proxy_for(&meshes, &here, &materials).unwrap();
        let b = proxy_for(&meshes, &shifted, &materials).unwrap();
        assert_eq!(a.occupied_cells(), 1);
        assert_eq!(a.digest(), b.digest());
    }

    #[test]
    fn fractional_self_hit_terminates_as_a_miss() {
        // Known oracle limit: a surface strictly inside its own proxy cell
        // starts inside solid geometry, so the shipped rule reports a miss.
        let world = fixture_world();
        let meshes = [prototype()];
        let materials = [OBJECT];
        let proxy = proxy_for(&meshes, &rest_instances(), &materials).unwrap();
        let pack = pack_with(&world, Some(&proxy)).unwrap();
        let sample = reflect_sample_with_mesh(
            &pack,
            &world,
            &proxy,
            [0.5, 0.5, 0.5],
            [0., 1., 0.],
            [0.5, 3., 0.5],
        );
        assert!(!sample.hit);
        assert!(sample.distance > 0.);
    }

    #[test]
    fn overlay_labels_phases_for_lead_capture() {
        let lines = diagnostic_lines(2, 7, false, true);
        assert!(lines[0].contains("phase 2/4 moved-invalidates"));
        assert!(lines[1].contains("presented 7"));
        assert_eq!(lines[2], "gi=false reflection=true");
    }

    #[test]
    fn retry_and_resize_never_replay_mutations_or_count_presentations() {
        with_check("accounting", |check| {
            check.phase = 1;
            let mut preparations = 0;
            assert!(!check
                .advance(
                    |check| {
                        preparations += 1;
                        check.evidence = vec!["prepared-once".into()];
                        Ok(())
                    },
                    |_| Ok(false),
                )
                .unwrap());
            assert_eq!(preparations, 1);
            assert_eq!(check.phase_presented, 0);
            // Retry, zero-extent and missing-renderer draws all present nothing.
            for _ in 0..3 {
                assert!(!check
                    .advance(
                        |_| panic!("prepared phase repeated after Retry"),
                        |_| Ok(false),
                    )
                    .unwrap());
            }
            assert_eq!(check.phase_presented, 0);
            assert_eq!(check.total_presented, 0);
            for _ in 0..PHASE_FRAMES {
                assert!(!check
                    .advance(|_| panic!("prepared phase repeated"), |_| Ok(true))
                    .unwrap());
            }
            assert_eq!(check.phase, 2);
            assert_eq!(check.total_presented, PHASE_FRAMES);
            let summaries: Vec<_> = check
                .report
                .iter()
                .filter(|line| line.starts_with("phase=1 "))
                .collect();
            assert_eq!(summaries.len(), 1);
            assert!(summaries[0].contains("prepared-once"));
        });
    }

    #[test]
    fn suspend_preserves_progress_and_resume_reapplies_without_duplicates() {
        with_check("lifecycle", |check| {
            check.phase = 3;
            check.phase_presented = 2;
            check.prepared = Some(3);
            check.evidence = vec!["pending".into()];
            check.suspended_helper_for_test();
            assert!(check.prepared.is_none());
            assert_eq!(check.phase, 3);
            assert_eq!(check.phase_presented, 2);
            let mut preparations = 0;
            for _ in 0..(PHASE_FRAMES - 2) {
                assert!(!check
                    .advance(
                        |check| {
                            preparations += 1;
                            check.evidence = vec!["re-applied".into()];
                            Ok(())
                        },
                        |_| Ok(true),
                    )
                    .unwrap());
            }
            // One re-application after resume, then draws only.
            assert_eq!(preparations, 1);
            assert_eq!(check.phase, 4);
            assert_eq!(
                check
                    .report
                    .iter()
                    .filter(|line| line.starts_with("phase=3 "))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn fixture_never_touches_user_saves() {
        with_check("isolated", |check| {
            let dir = std::env::temp_dir().join(format!(
                "matterweave-mesh-lighting-save-{}",
                std::process::id()
            ));
            assert!(!dir.exists());
            // The diagnostic owns no save path and performs no save or load.
            assert!(check
                .report_path
                .to_string_lossy()
                .contains("mesh-lighting"));
            assert!(!dir.exists());
        });
    }
}
