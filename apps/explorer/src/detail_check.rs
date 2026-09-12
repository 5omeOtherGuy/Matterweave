//! Native Vulkan automatic-detail (LOD) selection gate, not a mobile performance
//! benchmark. Builds a small disposable engine fixture (an irregular solid voxel
//! object and a one-cell-thick sheet, placed as several instances including
//! negative coordinates and yaw), preloads the `Source`/`Half`/`Quarter` derived
//! meshes once for every nonempty prototype, maps each `(prototype, Lod)` to a
//! resident static-prototype index, and then drives approach/retreat/zoom camera
//! phases that change *only* the packed instance selection while rendering.
//!
//! Owns no world, physics or save path. Writes a local report; Android
//! screenshots are captured externally with adb by the device lead.

use crate::detail_runtime::{lod_histogram, DetailRuntime, RESIDENT_LODS};
use glam::{Mat4, Vec3};
use matterweave_detail::{
    material, Camera as LodCamera, DetailScene, DetailVolume, Lod, LodConfig, Projection, Scale,
    Transform, Yaw, SCALE_FINE_M,
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

/// The derived levels preloaded once per nonempty prototype.
const LODS: [Lod; 3] = RESIDENT_LODS;

const BOULDER: &str = "boulder";
const SHEET: &str = "sheet";
const OPENING: &str = "opening";
const OPENING_INSTANCE: &str = "opening_guard";
const SHEET_INSTANCE: &str = "sheet_yaw";
const NEAR_INSTANCE: &str = "solid_near";

/// Point strictly inside the identity-placed boulder instance, used to prove the
/// authoritative source collision/sample answer is invariant to camera changes.
const COLLISION_PROBE_M: [f32; 3] = [0.1, 0.1, 0.1];

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

/// Dense control with no local voids: safe to coarsen under the default guard.
fn dense_solid(id: &str) -> DetailVolume {
    let mut v = DetailVolume::new(id, Scale::new(SCALE_FINE_M).expect("fine scale"));
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

/// The disposable engine fixture: three prototypes and five instances covering the
/// origin, a yaw-rotated negative-coordinate placement, a far placement and the
/// thin sheet plus a protected local opening.
pub fn fixture_scene() -> DetailScene {
    let mut scene = DetailScene::new();
    scene
        .add_prototype(dense_solid(BOULDER))
        .expect("add boulder");
    scene.add_prototype(thin_sheet(SHEET)).expect("add sheet");
    scene.add_prototype(opening_solid()).expect("add opening");
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
            SHEET_INSTANCE,
            SHEET,
            Transform::new([5.0, 0.0, -1.0], Yaw::Deg270).expect("sheet transform"),
        )
        .expect("place sheet");
    scene
}

fn histogram_label(counts: &BTreeMap<Lod, usize>) -> String {
    LODS.iter()
        .map(|lod| format!("{lod:?}={}", counts.get(lod).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A camera phase: eye, look target, projection and selection policy.
#[derive(Clone, Copy)]
pub struct PhasePlan {
    pub name: &'static str,
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub projection: Projection,
    pub config: LodConfig,
    /// True for phases that edit the source and rebuild resident geometry once.
    pub edit: bool,
    /// True for the phase that drops and recreates the renderer.
    pub recreate: bool,
}

const FOV: f32 = std::f32::consts::PI / 3.0;

/// The ordered phases. Approach/retreat/zoom under perspective and orthographic
/// projection drive selection; the last three exercise edit invalidation, a work
/// resident zero-build budget and a renderer lifecycle recreation.
pub fn phases() -> Vec<PhasePlan> {
    let origin = [0.2, 0.2, 0.0];
    let default = LodConfig::default();
    vec![
        PhasePlan {
            name: "approach-near-perspective",
            eye: [0.2, 0.2, 2.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV,
            },
            config: default,
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "retreat-far-perspective",
            eye: [0.2, 0.2, 220.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV,
            },
            config: default,
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "mid-perspective",
            eye: [0.2, 0.2, 70.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV,
            },
            config: default,
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "perspective-fov-zoom-in",
            eye: [0.2, 0.2, 70.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: 0.03,
            },
            config: default,
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "orthographic-zoomed-out",
            eye: [0.2, 0.2, 30.0],
            target: origin,
            projection: Projection::Orthographic {
                view_height_m: 200.0,
            },
            config: default,
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "orthographic-zoomed-in",
            eye: [0.2, 0.2, 30.0],
            target: origin,
            projection: Projection::Orthographic { view_height_m: 4.0 },
            config: default,
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "edit-invalidate-rebuild",
            eye: [0.2, 0.2, 70.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV,
            },
            config: default,
            edit: true,
            recreate: false,
        },
        PhasePlan {
            name: "resident-zero-build-budget",
            eye: [0.2, 0.2, 220.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV,
            },
            config: LodConfig {
                max_coarse_builds: Some(0),
                ..default
            },
            edit: false,
            recreate: false,
        },
        PhasePlan {
            name: "lifecycle-recreate",
            eye: [0.2, 0.2, 2.0],
            target: origin,
            projection: Projection::Perspective {
                vertical_fov_rad: FOV,
            },
            config: default,
            edit: false,
            recreate: true,
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

#[cfg(target_os = "android")]
const PHASE_FRAMES: u32 = 120;
#[cfg(not(target_os = "android"))]
const PHASE_FRAMES: u32 = 6;

/// Authoritative source answers that must not change with the camera.
#[derive(Clone, Debug, PartialEq)]
struct SourceBaseline {
    collidable: bool,
    sample: Option<(String, u8)>,
}

impl SourceBaseline {
    fn capture(scene: &DetailScene) -> Result<Self, String> {
        Ok(Self {
            collidable: scene
                .is_collidable_world_metres(COLLISION_PROBE_M)
                .map_err(|e| e.to_string())?,
            sample: scene
                .sample_world_metres(COLLISION_PROBE_M)
                .map_err(|e| e.to_string())?,
        })
    }
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
    baseline: Option<SourceBaseline>,
    lods_seen: BTreeSet<Lod>,
    near_source_seen: bool,
    near_coarsened_seen: bool,
    frame: u32,
    prepared: Option<u32>,
    edited: bool,
    recreated: bool,
    finished: bool,
}

impl DetailCheck {
    pub fn new(report_path: std::path::PathBuf) -> Self {
        Self {
            report_path,
            report: Vec::new(),
            renderer: None,
            window: None,
            scene: fixture_scene(),
            runtime: None,
            plans: phases(),
            lighting: LightingSettings {
                sun: Sun {
                    direction_to_sun: [0.35, 1.0, 0.25],
                    intensity: 0.9,
                },
                ..Default::default()
            },
            baseline: None,
            lods_seen: BTreeSet::new(),
            near_source_seen: false,
            near_coarsened_seen: false,
            frame: 0,
            prepared: None,
            edited: false,
            recreated: false,
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

    /// Preloads the derived geometry and installs it in the renderer's static
    /// scene, with the current phase's instance selection. One `replace_static_scene`
    /// per renderer; later frames call `update_static_instances` only.
    fn install_static_scene(&mut self, phase: usize) -> Result<(), String> {
        let mut runtime = DetailRuntime::preload(&mut self.scene)?;
        let plan = self.plans[phase];
        let height = self
            .window
            .as_ref()
            .map(|w| w.inner_size().height.max(1) as f32)
            .unwrap_or(480.0);
        let update = runtime.prepare(&mut self.scene, &plan.lod_camera(height), &plan.config)?;
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

    fn apply_phase(&mut self, phase: usize) -> Result<String, String> {
        let plan = self.plans[phase];
        let height = self
            .window
            .as_ref()
            .map(|w| w.inner_size().height.max(1) as f32)
            .unwrap_or(480.0);

        if plan.edit && !self.edited {
            let before = self.scene.source_version();
            self.scene
                .edit_prototype(BOULDER, [12, 12, 12], material::BANK_STONE)
                .map_err(|e| format!("edit_prototype: {e}"))?;
            let after = self.scene.source_version();
            if after == before {
                return Err("edit did not advance the source version".into());
            }
            self.edited = true;
            // Rebuild resident geometry once against the edited source and
            // re-baseline the (unchanged at the probe) source answer.
            self.install_static_scene(phase)?;
            self.baseline = Some(SourceBaseline::capture(&self.scene)?);
            let runtime = self.runtime.as_ref().expect("runtime after rebuild");
            let rev = runtime
                .revision(BOULDER, Lod::Quarter)
                .ok_or("missing rebuilt boulder revision")?;
            let source_rev = self
                .scene
                .prototype(BOULDER)
                .ok_or("boulder vanished")?
                .revision();
            if rev != source_rev {
                return Err(format!(
                    "rebuilt boulder revision {rev} != source {source_rev}"
                ));
            }
        }

        let update = self.runtime.as_mut().ok_or("no runtime")?.prepare(
            &mut self.scene,
            &plan.lod_camera(height),
            &plan.config,
        )?;
        let frame = &update.frame;

        // The prepared frame must have been derived from the live source.
        if frame.source_version != self.scene.source_version() {
            return Err("prepared frame source version disagrees with the scene".into());
        }

        // The gate installs resident geometry before any phase runs; a
        // mid-phase geometry change would leave the renderer's static scene
        // stale, so it is an error, not a silent instance-only update.
        if update.geometry_changed {
            return Err("resident geometry changed outside static-scene install".into());
        }

        let runtime = self.runtime.as_ref().ok_or("no runtime")?;

        // Trace resident revisions against the authoritative source.
        for item in &frame.selected {
            let source_rev = self
                .scene
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

        let histogram = lod_histogram(frame);
        for lod in histogram.keys() {
            self.lods_seen.insert(*lod);
        }

        // The thin sheet must stay at Source under the default cap.
        let sheet = frame
            .selected
            .iter()
            .find(|s| s.instance == SHEET_INSTANCE)
            .ok_or("sheet instance missing from selection")?;
        if !plan.edit && plan.config.max_coarse_builds.is_none() && sheet.lod != Lod::Source {
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

        // Work-budget cap. Because every LOD is preloaded once per renderer, the
        // cap on *new* coarse builds is a no-op here: the truthful guarantee it
        // still enforces is that this frame builds no new geometry. The engine's
        // retreat-to-Source fallback when a coarse mesh is not yet resident is
        // proved by the unit tests on a fresh, un-preloaded scene.
        if plan.config.max_coarse_builds == Some(0) && frame.mesh_builds_this_call != 0 {
            return Err(format!(
                "work-budget cap still built {} meshes this frame",
                frame.mesh_builds_this_call
            ));
        }

        // Camera-invariant source collision / sample answer.
        let baseline = self.baseline.as_ref().ok_or("no baseline")?.clone();
        let now = SourceBaseline::capture(&self.scene)?;
        if now != baseline {
            return Err("source collision/sample answer changed with the camera".into());
        }

        let renderer = self.renderer.as_mut().ok_or("no renderer")?;
        renderer
            .update_static_instances(&update.instances)
            .map_err(|e| format!("update_static_instances: {e}"))?;

        let selections = frame
            .selected
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{:?}:r{}",
                    s.instance,
                    s.prototype,
                    s.lod,
                    runtime.revision(&s.prototype, s.lod).unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let counts = self.scene.counts();
        Ok(format!(
            "phase={phase} {} proj={} lods[{}] instances={} builds_this_call={} cached_bytes={} source_cells={} source_bytes={} sheet=Source opening=Source selected=[{selections}] resident_prototypes={}",
            plan.name,
            match plan.projection {
                Projection::Perspective { .. } => "perspective",
                Projection::Orthographic { .. } => "orthographic",
            },
            histogram_label(&histogram),
            update.instances.len(),
            frame.mesh_builds_this_call,
            frame.cached_mesh_bytes,
            counts.unique_stored_cells,
            counts.source_bytes,
            runtime.meshes().len(),
        ))
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
        if !self.edited {
            return Err("edit invalidation phase never ran".into());
        }
        Ok(())
    }
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
        if self.baseline.is_none() {
            match SourceBaseline::capture(&self.scene) {
                Ok(baseline) => self.baseline = Some(baseline),
                Err(error) => {
                    self.record(format!("FAIL detail: baseline capture: {error}"));
                    el.exit();
                    return;
                }
            }
        }
        let phase = (self.frame / PHASE_FRAMES) as usize;
        let phase = phase.min(self.plans.len() - 1);
        if let Err(error) = self.install_static_scene(phase) {
            self.record(format!("FAIL detail: install static scene: {error}"));
            el.exit();
            return;
        }
        let caps = self.renderer.as_ref().unwrap().capabilities.to_string();
        self.record(format!("capabilities: {caps}"));
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
            // which re-preloads geometry and re-installs the static scene.
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
        let hud = Hud::new(size.width as f32, size.height as f32);
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
                        "PASS detail: approach/retreat/zoom perspective+orthographic selection over {} phases; LODs chosen {}; thin sheet and local opening held at Source; source collision/query invariant; edit invalidation rebuilt once; resident zero-build budget, zero extent and lifecycle recreation verified (cold-cache fallback: unit test only)",
                        self.plans.len(),
                        seen.join("/")
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
        // Resident GPU geometry is renderer-owned; a fresh renderer re-preloads.
        self.runtime = None;
        self.prepared = None;
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

    #[test]
    fn stale_runtime_is_rejected_after_source_edit() {
        let mut scene = fixture_scene();
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        scene
            .edit_prototype(BOULDER, [12, 12, 12], material::BANK_STONE)
            .unwrap();
        let plan = phases()[0];
        let frame = scene
            .prepare_batches(&plan.lod_camera(480.0), &plan.config)
            .unwrap();
        assert!(runtime.instances_for_frame(&frame).is_err());
    }

    #[test]
    fn preload_builds_each_lod_once_with_source_revisions() {
        let mut scene = fixture_scene();
        let before = scene.counts().mesh_builds;
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        // Three nonempty prototypes x three LODs.
        assert_eq!(runtime.meshes().len(), 9);
        assert_eq!(scene.counts().mesh_builds, before + 9);
        for id in [BOULDER, SHEET, OPENING] {
            let source_rev = scene.prototype(id).unwrap().revision();
            for lod in LODS {
                assert!(runtime.instance_index(id, lod).is_some());
                assert_eq!(runtime.revision(id, lod), Some(source_rev));
            }
        }
        // Distinct pool indices.
        let mut indices: Vec<usize> = LODS
            .iter()
            .flat_map(|&lod| {
                [
                    runtime.instance_index(BOULDER, lod),
                    runtime.instance_index(SHEET, lod),
                    runtime.instance_index(OPENING, lod),
                ]
            })
            .map(|i| i.unwrap())
            .collect();
        indices.sort();
        indices.dedup();
        assert_eq!(indices.len(), 9);
    }

    #[test]
    fn native_fixture_preserves_opening_while_dense_control_coarsens() {
        let mut scene = fixture_scene();
        let plan = phases()[1];
        let frame = scene
            .prepare_batches(&plan.lod_camera(1080.0), &plan.config)
            .unwrap();
        let opening = frame
            .selected
            .iter()
            .find(|s| s.instance == "opening_guard")
            .expect("native fixture must exercise the local opening guard");
        assert_eq!(opening.lod, Lod::Source);
        let control = frame
            .selected
            .iter()
            .find(|s| s.instance == NEAR_INSTANCE)
            .unwrap();
        assert!(
            control.lod > Lod::Source,
            "safe dense control must still coarsen"
        );
    }

    #[test]
    fn approach_selects_source_and_retreat_coarsens_the_same_instance() {
        let plans = phases();
        let near = plans[0];
        let far = plans[1];
        let mut scene = fixture_scene();
        let approach = scene
            .prepare_batches(&near.lod_camera(1080.0), &near.config)
            .unwrap();
        let near_lod = approach
            .selected
            .iter()
            .find(|s| s.instance == NEAR_INSTANCE)
            .unwrap()
            .lod;
        assert_eq!(near_lod, Lod::Source, "near instance is authoritative");

        let mut fresh = fixture_scene();
        let retreat = fresh
            .prepare_batches(&far.lod_camera(1080.0), &far.config)
            .unwrap();
        let far_lod = retreat
            .selected
            .iter()
            .find(|s| s.instance == NEAR_INSTANCE)
            .unwrap()
            .lod;
        assert!(far_lod > Lod::Source, "far instance coarsened: {far_lod:?}");
    }

    #[test]
    fn thin_sheet_stays_at_source_across_default_camera_phases() {
        for plan in phases()
            .into_iter()
            .filter(|p| !p.edit && p.config.max_coarse_builds.is_none())
        {
            let mut scene = fixture_scene();
            let frame = scene
                .prepare_batches(&plan.lod_camera(1080.0), &plan.config)
                .unwrap();
            let sheet = frame
                .selected
                .iter()
                .find(|s| s.instance == SHEET_INSTANCE)
                .unwrap();
            assert_eq!(sheet.lod, Lod::Source, "{}", plan.name);
        }
    }

    #[test]
    fn source_collision_and_sample_are_camera_invariant() {
        let mut scene = fixture_scene();
        let baseline = SourceBaseline::capture(&scene).unwrap();
        assert!(baseline.collidable);
        for plan in phases().into_iter().filter(|p| !p.edit) {
            scene
                .prepare_batches(&plan.lod_camera(1080.0), &plan.config)
                .unwrap();
            assert_eq!(SourceBaseline::capture(&scene).unwrap(), baseline);
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
            .unwrap();
        // Fresh scene: no coarse mesh is resident, so the cap forces the
        // authoritative Source and flags the coarse-desiring boulders.
        let mut scene = fixture_scene();
        let frame = scene
            .prepare_batches(&far.lod_camera(1080.0), &far.config)
            .unwrap();
        assert!(frame.selected.iter().all(|s| s.lod == Lod::Source));
        assert!(frame
            .selected
            .iter()
            .filter(|s| s.prototype == BOULDER)
            .all(|s| s.fallback));
        assert!(frame.selected.iter().any(|s| s.fallback));
        // Once every LOD is resident, the same cap builds nothing more.
        let runtime = DetailRuntime::preload(&mut scene).unwrap();
        let again = scene
            .prepare_batches(&far.lod_camera(1080.0), &far.config)
            .unwrap();
        assert_eq!(again.mesh_builds_this_call, 0);
        let instances = runtime.instances_for_frame(&again).unwrap();
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
        let plan = phases()[2];
        let frame = scene
            .prepare_batches(&plan.lod_camera(1080.0), &plan.config)
            .unwrap();
        let instances = runtime.instances_for_frame(&frame).unwrap();
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
        // Multiple instances including a yaw-rotated negative placement.
        assert!(instances.iter().any(|i| i.yaw_quarters != 0));
        assert!(instances.iter().any(|i| i.translation[0] < 0.0));
    }
}
