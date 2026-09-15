//! Dense, wind-blown vegetation for the landscape sample.
//!
//! Three pieces meet here and nothing else does:
//!
//! - [`matterweave_core::landscape::plan_flora_into`] decides *where* plants
//!   stand, from the same seed and generator the terrain comes from. That is a
//!   pure function of the eye position; this module never invents a placement.
//! - [`matterweave_detail::landscape_flora`] decides *what* a plant looks like:
//!   `prototype_for` names the prototype, `landscape_prototype` builds it, and
//!   the shared [`DetailRuntime`] pools its `Source`/`Half`/`Quarter` meshes.
//! - [`matterweave_render::Renderer::replace_flora_scene`] draws them, with the
//!   per-instance wind record `world.wgsl` displaces from.
//!
//! # When the field is rebuilt
//!
//! Placement is a function of the eye, so it only has to change when the eye
//! does - and only enough for the plan to differ. A rebuild is triggered only
//! when the eye's [`REBUILD_CELL_M`] cell has moved more than
//! [`REBUILD_MARGIN_CELLS`] cells from the committed plan's cell, so a steady
//! flight cannot rebuild every frame: at the scripted exercise path's one cell
//! per frame the field rebuilds about once every six frames, and walking at
//! 5 m/s rebuilds when the margin is crossed rather than at every cell edge.
//! Every other frame touches nothing but the wind uniform.
//!
//! A rebuild is incremental. [`matterweave_core::landscape::FloraField`] memoises
//! the generated population of every lattice cell, so the plan for the new eye
//! reuses the cells the old window already covered and pays only for the cells
//! the new window adds; the completed plan is bit-identical to
//! [`matterweave_core::landscape::plan_flora`] at the same eye. Work is admitted
//! in [`MAX_PLAN_SAMPLES_PER_FRAME`]-cell slices, so a first plan or a large
//! jump cannot overrun the frame. The plan, the instance list and the upload
//! complete before the renderer swaps its buffers, and any failure or partial
//! slice leaves the previous field resident and drawn: a field never flickers,
//! empties for a frame, or shows a half-uploaded set.
//!
//! # Why the cell generation stays on the main thread
//!
//! The tile stream runs its mesh generation on a worker because a tile mesh is
//! expensive and every tile is independent. A plant population is not: the
//! plan is assembled from the memo in lattice order, generation is interleaved
//! with that assembly, and the memo turns a rebuild into a few hundred cell
//! samples rather than thousands. Backgrounding it would add a request/response
//! latency to every rebuild and a second copy of the memo, to move roughly
//! 120 cells per frame at 60 Hz - less than the frame's slice on its worst
//! frame. The bounded memo is the reusable part of that pattern; the worker is
//! not.
//!
//! # What is not here
//!
//! No collision: walking through a flower is free, exactly as
//! [`matterweave_detail::landscape_flora`] documents for its decorative
//! materials. No per-plant culling either - the renderer culls whole prototype
//! batches, which is the same bound the static path has always had.

use matterweave_core::landscape::{
    FloraField, FloraKind, FloraPlan, PlacedFlora, FLORA_MAX_VARIATION_PERCENT,
    FLORA_MIN_VARIATION_PERCENT, LANDSCAPE_FLORA_TIERS,
};
use matterweave_core::Mesh;
use matterweave_detail::{
    landscape_prototype, prototype_for, DetailScene, Lod, LANDSCAPE_FLORA_SPECIES,
};
use matterweave_render::{FloraInstance, Renderer, StaticSceneStats, MAX_FLORA_INSTANCES};
use std::collections::BTreeMap;
use std::time::Instant;

use crate::detail_runtime::{DetailRuntime, RESIDENT_LODS};

/// Ground-cover placements one plan may carry. Together with
/// [`MAX_PLANNED_TREES`] this stays inside the renderer's
/// [`MAX_FLORA_INSTANCES`] budget with room for the caps to be raised.
pub const MAX_PLANNED_SITES: usize = 9_000;
/// Tree placements one plan may carry.
pub const MAX_PLANNED_TREES: usize = 700;
/// Eye movement, in metres, that names a rebuild cell. The committed field is
/// only re-anchored when the eye has left this cell by [`REBUILD_MARGIN_CELLS`].
pub const REBUILD_CELL_M: i32 = 4;
/// Whole [`REBUILD_CELL_M`] cells the eye may drift from the committed plan's
/// cell before the field is rebuilt.
///
/// The margin must exceed the eye's per-frame travel at flight speed -
/// 90 m/s at 60 Hz is 1.5 m, well inside the 4 m cell - and must carry the
/// scripted exercise path (4 m, one cell, per frame) far below one rebuild per
/// frame. Five cells of margin means a rebuild is not even requested more often
/// than once every six frames on that path, and the plan commits within a few
/// frames of the request, so the field never trails the eye by more than the
/// margin plus that latency.
pub const REBUILD_MARGIN_CELLS: i32 = 5;
/// Cells one frame may newly generate while planning.
///
/// The field only generates cells the previous window did not cover, so a
/// rebuild after a normal hysteresis step needs a few hundred to about a
/// thousand; this cap commits one in a few frames and keeps every planning
/// frame's admitted work small, while a pathological jump is carried by the
/// next frame's slice.
pub const MAX_PLAN_SAMPLES_PER_FRAME: usize = 256;
/// Main-thread milliseconds one rebuild may spend planning. Reported, not
/// enforced by truncation: a plan that overran is still a correct plan, and
/// hiding the overrun would be worse than showing it.
pub const MAX_REBUILD_MS: f64 = 8.0;

/// Wind the sample blows. Direction is a fixed compass bearing; strength is
/// well inside [`matterweave_render::Wind::MAX_STRENGTH_M`].
pub const WIND_DIRECTION_XZ: [f32; 2] = [0.82, 0.57];
pub const WIND_STRENGTH_M: f32 = 0.35;
/// Radius the player parts the field within, in metres.
pub const PUSH_RADIUS_M: f32 = 1.6;

/// Prototypes this sample builds: the landscape catalogue plus the reed cluster
/// `prototype_for` maps swamp reeds onto.
fn species() -> Vec<&'static str> {
    let mut ids = LANDSCAPE_FLORA_SPECIES.to_vec();
    ids.push("reed_cluster");
    ids
}

/// How compliant each kind is in the wind, in `0..=1`. A cactus barely moves; a
/// blade of grass gives completely. This is the *stiffest* an instance of the
/// kind bends: [`bend_factor`] scales it down per site, so two tufts of the same
/// kind do not sway as one.
fn bend_of(kind: FloraKind) -> f32 {
    match kind {
        FloraKind::GrassTuft | FloraKind::Reed => 1.0,
        FloraKind::FlowerRed | FloraKind::FlowerWhite | FloraKind::FlowerYellow => 0.85,
        FloraKind::Fern => 0.7,
        FloraKind::Shrub => 0.35,
        FloraKind::TreeBroadleaf | FloraKind::TreeConifer => 0.22,
        FloraKind::Cactus => 0.04,
    }
}

/// The plan's per-site bend percentage as a factor on the kind's compliance, in
/// `0.75..=1.0`.
///
/// The percentage maps onto the top of the compliance range rather than around
/// its middle for two reasons: the stiffest instance of every kind bends exactly
/// as far as it did before the variation existed, and `bend_of(kind) * factor`
/// stays inside the wind record's `0..=1` for every kind - the two most compliant
/// kinds sit at 1.0 and 0.85, so a factor above 1 would simply clamp and make
/// their variation invisible.
fn bend_factor(bend_percent: u8) -> f32 {
    let span = f32::from(FLORA_MAX_VARIATION_PERCENT - FLORA_MIN_VARIATION_PERCENT);
    let offset = f32::from(bend_percent.saturating_sub(FLORA_MIN_VARIATION_PERCENT));
    0.75 + 0.25 * (offset / span).min(1.0)
}

/// Detail level for a ground-cover placement's density band. The near band
/// draws the authoritative source clump; the far band draws the derived half
/// level, which is a low mat of merged blades at 12.5 cm cells - the right read
/// for ground cover between 28 and 48 m and strictly cheaper than the source.
/// Beyond the last band there is no ground cover at all: see
/// [`matterweave_core::landscape::LANDSCAPE_FLORA_TIERS`] for the crossover.
fn lod_for_tier(tier: u8) -> Lod {
    match tier {
        0 => Lod::Source,
        _ => Lod::Half,
    }
}

/// Detail level for a tree at `distance_m` metres from the eye.
///
/// Trees are landmarks rather than ground cover: they are planned to
/// [`matterweave_core::landscape::LANDSCAPE_TREE_RADIUS_M`], far past the
/// outermost ground band, so their bands are their own. The first band keeps the
/// source crown (fronds and gaps) where it can be resolved, the middle band the
/// half crown and the outer band the quarter, which is still a trunk with a
/// crown at 100 m and beyond.
fn tree_lod(distance_m: i32) -> Lod {
    if distance_m <= 32 {
        Lod::Source
    } else if distance_m <= 96 {
        Lod::Half
    } else {
        Lod::Quarter
    }
}

/// Deterministic sway phase in `0..=1` from a placement's metre coordinate, so
/// neighbouring plants never pulse together and the same plant always carries
/// the same phase across rebuilds.
fn phase_of(x: i32, z: i32) -> f32 {
    // Mix the two coordinates, then run the Murmur3 `fmix32` finalizer so the
    // low 16 bits the phase reads have full avalanche: a neighbour one metre
    // away must not visibly share the same sway.
    let mut v = (x as u32).wrapping_mul(0x9E37_79B9) ^ (z as u32).wrapping_mul(0x85EB_CA6B);
    v ^= v >> 16;
    v = v.wrapping_mul(0x7FEB_352D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846C_A68B);
    v ^= v >> 16;
    (v & 0xFFFF) as f32 / 65535.0
}

/// Counters the HUD shows and the smoke line prints. Every one is something
/// this module actually did, not a target.
#[derive(Clone, Copy, Debug, Default)]
pub struct FloraCounters {
    pub planned_sites: usize,
    pub planned_trees: usize,
    /// Every planned placement that is not drawn: the planner's own cap
    /// refusals, a placement whose prototype has no resident geometry at its
    /// level or at the source fallback, and a placement refused because the
    /// renderer's instance budget was already full. `planned_sites +
    /// planned_trees - drawn` equals it exactly, which is what makes the number
    /// checkable rather than trusted.
    pub dropped: usize,
    /// Instances handed to the renderer, after prototype mapping.
    pub drawn: usize,
    pub batches: usize,
    /// Allocated bytes of the two per-instance buffers.
    pub instance_bytes: usize,
    pub plan_ms: f64,
    pub upload_ms: f64,
    pub rebuilds: u64,
    pub worst_plan_ms: f64,
    pub worst_upload_ms: f64,
    /// Rebuilds whose plan exceeded [`MAX_REBUILD_MS`].
    pub over_budget: u64,
    /// A slice of a rebuild is still pending; the resident field is unchanged.
    pub pending: bool,
    /// Lattice cells the field has generated over the run, i.e. memo misses.
    pub samples: u64,
}

/// Pool index and prototype height in metres per `(prototype id, level)`.
type Resident = BTreeMap<(&'static str, Lod), (usize, f32)>;

/// Resident flora geometry plus the current field.
pub struct LandscapeFlora {
    runtime: DetailRuntime,
    resident: Resident,
    /// Incremental placement field: the committed plan plus the per-cell memo.
    field: FloraField,
    instances: Vec<FloraInstance>,
    installed: bool,
    /// [`FloraField::generation`] the renderer's current buffers were filled
    /// from. Differs from the field's generation when a rebuild has committed
    /// but not yet been uploaded.
    uploaded_generation: u64,
    counters: FloraCounters,
}

impl LandscapeFlora {
    /// Build and pool every prototype the catalogue can place.
    ///
    /// This runs once, at sample construction, and is the only place geometry
    /// is built: a rebuild only re-selects among these pooled meshes.
    pub fn new() -> Result<Self, String> {
        let mut scene = DetailScene::new();
        for id in species() {
            let volume = landscape_prototype(id).map_err(|e| format!("build {id}: {e}"))?;
            scene
                .add_prototype(volume)
                .map_err(|e| format!("add {id}: {e}"))?;
        }
        let runtime = DetailRuntime::preload(&mut scene)?;
        let mut resident = BTreeMap::new();
        for id in species() {
            for lod in RESIDENT_LODS {
                // A level the detail engine could not build stays absent;
                // `instance_for` falls back to the source level, exactly as
                // `DetailScene::ensure_mesh` degrades.
                let Some(index) = runtime.instance_index(id, lod) else {
                    continue;
                };
                let height = mesh_height_m(&runtime.meshes()[index]);
                if height <= 0.0 {
                    continue;
                }
                resident.insert((id, lod), (index, height));
            }
        }
        if resident.is_empty() {
            return Err("no landscape flora prototype produced drawable geometry".into());
        }
        Ok(Self {
            runtime,
            resident,
            field: FloraField::new(crate::landscape::SEED),
            instances: Vec::with_capacity(MAX_PLANNED_SITES + MAX_PLANNED_TREES),
            installed: false,
            uploaded_generation: 0,
            counters: FloraCounters::default(),
        })
    }

    pub fn counters(&self) -> FloraCounters {
        self.counters
    }

    /// The rebuild cell an eye position belongs to.
    fn cell_of(eye: [f32; 3]) -> [i32; 2] {
        [
            (eye[0].floor() as i32).div_euclid(REBUILD_CELL_M),
            (eye[2].floor() as i32).div_euclid(REBUILD_CELL_M),
        ]
    }

    /// The rebuild cell a committed metre anchor belongs to.
    fn metre_cell(metres: [i32; 2]) -> [i32; 2] {
        [
            metres[0].div_euclid(REBUILD_CELL_M),
            metres[1].div_euclid(REBUILD_CELL_M),
        ]
    }

    /// Whether the committed field still covers the eye within the hysteresis
    /// margin, so no rebuild is due yet.
    ///
    /// This is the hysteresis: the field follows the eye once it is more than
    /// [`REBUILD_MARGIN_CELLS`] cells from the cell the committed plan was made
    /// in, not the instant the eye crosses a cell edge.
    fn field_covers_eye(&self, eye: [f32; 3]) -> bool {
        self.field
            .anchor()
            .is_some_and(|anchor| anchor_covers(anchor, eye))
    }

    /// Advance and upload the field when the eye leaves the committed plan's
    /// hysteresis window.
    ///
    /// Returns without touching the renderer while the committed field covers
    /// the eye and is already installed, which is every frame until the eye has
    /// moved [`REBUILD_MARGIN_CELLS`] cells past the plan's cell. A slice that
    /// runs out of budget also returns without uploading: the resident field
    /// stays exactly as it was and the next frame carries the remainder. On an
    /// upload error the plan stays committed but uninstalled, so the next frame
    /// retries the upload rather than re-planning the field.
    pub fn sync(&mut self, renderer: &mut Renderer, eye: [f32; 3]) -> Result<(), String> {
        if self.installed
            && self.uploaded_generation == self.field.generation()
            && self.field_covers_eye(eye)
        {
            return Ok(());
        }
        let previous = self.counters;
        let plan_begin = Instant::now();
        let generation_before = self.field.generation();
        let ready = self.field.advance(
            eye,
            &LANDSCAPE_FLORA_TIERS,
            MAX_PLANNED_SITES,
            MAX_PLANNED_TREES,
            MAX_PLAN_SAMPLES_PER_FRAME,
        );
        let plan_ms = plan_begin.elapsed().as_secs_f64() * 1000.;
        if !ready {
            // A slice, not a failure: the committed field stays resident and
            // drawn, and the next frame carries the remainder.
            self.counters = FloraCounters {
                plan_ms,
                worst_plan_ms: previous.worst_plan_ms.max(plan_ms),
                over_budget: previous.over_budget + u64::from(plan_ms > MAX_REBUILD_MS),
                pending: true,
                samples: self.field.samples(),
                ..previous
            };
            return Ok(());
        }
        let rebuilt = self.field.generation() != generation_before;
        let (planned_sites, planned_trees, dropped, refused) = {
            let plan = self.field.plan();
            let refused = fill_instances(&self.resident, plan, eye, &mut self.instances);
            (plan.sites.len(), plan.trees.len(), plan.dropped, refused)
        };
        let upload_begin = Instant::now();
        // Transactional in both directions: the renderer validates and packs
        // the whole list before it swaps or writes a buffer, so a rejected
        // rebuild leaves the field that is on screen exactly as it was.
        let stats: StaticSceneStats = if self.installed {
            renderer.update_flora_instances(&self.instances)?
        } else {
            renderer.replace_flora_scene(self.runtime.meshes(), &self.instances)?
        };
        let upload_ms = upload_begin.elapsed().as_secs_f64() * 1000.;
        self.installed = true;
        self.uploaded_generation = self.field.generation();
        self.counters = FloraCounters {
            planned_sites,
            planned_trees,
            // The planner's own refusals plus the placements this module could
            // not draw, so `dropped` accounts for every planned plant that is
            // not on screen: dropping is fine, under-reporting is not.
            dropped: dropped + refused,
            drawn: stats.instances,
            batches: stats.batches,
            instance_bytes: stats.instance_bytes,
            plan_ms,
            upload_ms,
            rebuilds: previous.rebuilds + u64::from(rebuilt),
            worst_plan_ms: previous.worst_plan_ms.max(plan_ms),
            worst_upload_ms: previous.worst_upload_ms.max(upload_ms),
            over_budget: previous.over_budget + u64::from(plan_ms > MAX_REBUILD_MS),
            pending: false,
            samples: self.field.samples(),
        };
        Ok(())
    }

    /// Forget the renderer this field was installed into.
    ///
    /// A renderer recreation (Android suspend/resume, a lost device) destroys
    /// the flora buffers with the renderer. The committed field and its cell
    /// memo survive, so this only marks the field uninstalled: the next
    /// [`Self::sync`] re-uploads the committed plan instead of re-planning it.
    pub fn forget_renderer(&mut self) {
        self.installed = false;
    }
}

/// Whether a committed metre anchor still covers an eye under the rebuild
/// hysteresis. Shared by [`LandscapeFlora::field_covers_eye`] and its test so
/// the trigger arithmetic is exercised without building any prototype.
fn anchor_covers(anchor: [i32; 2], eye: [f32; 3]) -> bool {
    let anchor = LandscapeFlora::metre_cell(anchor);
    let eye = LandscapeFlora::cell_of(eye);
    (anchor[0] - eye[0]).abs().max((anchor[1] - eye[1]).abs()) <= REBUILD_MARGIN_CELLS
}

/// Fill `instances` with every placement of `plan` that has resident geometry,
/// in plan order, and return how many planned placements this module refused.
///
/// Refusing is allowed - a placement whose geometry failed to build, or a field
/// larger than the renderer's instance budget, must not be drawn - but the count
/// is returned rather than discarded so [`LandscapeFlora::sync`] can add it to
/// the reported `dropped` total. An earlier version broke out of the loop on the
/// budget and skipped silent placements, which made `dropped` a number nobody
/// could check.
fn fill_instances(
    resident: &Resident,
    plan: &FloraPlan,
    eye: [f32; 3],
    instances: &mut Vec<FloraInstance>,
) -> usize {
    let mut refused = 0;
    instances.clear();
    for placed in plan.sites.iter().chain(&plan.trees) {
        if instances.len() >= MAX_FLORA_INSTANCES {
            // Keep counting: the field is refused from here on, and every
            // refused placement is reported, not just the first.
            refused += 1;
            continue;
        }
        match instance_for(resident, placed, eye) {
            Some(instance) => instances.push(instance),
            None => refused += 1,
        }
    }
    refused
}

/// Map one placement onto a pooled prototype at the level its band asks for,
/// falling back to the source level when the coarse one is absent. A placement
/// whose prototype produced no geometry at all is skipped, and [`fill_instances`]
/// counts that skip rather than letting it disappear.
///
/// The plan's per-site variation lands here: `height_percent` becomes the
/// instance scale (0.75x to 1.25x, so neighbours differ in size without a
/// prototype per size) and `bend_percent` scales the kind's compliance down
/// through [`bend_factor`].
fn instance_for(resident: &Resident, placed: &PlacedFlora, eye: [f32; 3]) -> Option<FloraInstance> {
    let id = prototype_for(&placed.as_site());
    let lod = match placed.kind {
        FloraKind::TreeBroadleaf | FloraKind::TreeConifer => tree_lod(
            (placed.x - eye[0] as i32)
                .abs()
                .max((placed.z - eye[2] as i32).abs()),
        ),
        _ => lod_for_tier(placed.tier),
    };
    let &(prototype, height_m) = resident
        .get(&(id, lod))
        .or_else(|| resident.get(&(id, Lod::Source)))?;
    Some(FloraInstance {
        prototype,
        // Plants stand on top of the surface voxel, which is the same surface
        // walk mode puts the eye above. Beyond the streaming window the visible
        // ground is the derived ring tile, whose vertices interpolate 4 m
        // generator samples, so a distant plant can sit a metre or two off the
        // surface it is drawn against.
        translation: [
            placed.x as f32 + 0.5,
            placed.y as f32 + 1.0,
            placed.z as f32 + 0.5,
        ],
        yaw_quarters: placed.yaw_quarters.min(3),
        phase: phase_of(placed.x, placed.z),
        bend: bend_of(placed.kind) * bend_factor(placed.bend_percent),
        height_m,
        scale: f32::from(placed.height_percent) / 100.0,
    })
}

/// Height of a pooled prototype mesh in metres: the largest local `y` its
/// vertices reach, which is what `world.wgsl` normalizes displacement by.
fn mesh_height_m(mesh: &Mesh) -> f32 {
    mesh.vertices
        .iter()
        .map(|vertex| vertex.position[1])
        .filter(|y| y.is_finite())
        .fold(0.0f32, f32::max)
}

/// Bridge back to the placement vocabulary `prototype_for` speaks.
trait AsSite {
    fn as_site(&self) -> matterweave_core::landscape::FloraSite;
}

impl AsSite for PlacedFlora {
    fn as_site(&self) -> matterweave_core::landscape::FloraSite {
        matterweave_core::landscape::FloraSite {
            kind: self.kind,
            x: self.x,
            y: self.y,
            z: self.z,
            yaw_quarters: self.yaw_quarters,
            scale_eighths: self.scale_eighths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_core::landscape::{self, LANDSCAPE_TREE_RADIUS_M};

    #[test]
    fn every_prototype_the_catalogue_can_place_is_pooled_with_a_height() {
        let flora = LandscapeFlora::new().expect("prototypes must build");
        for id in species() {
            let (_, height) = flora
                .resident
                .get(&(id, Lod::Source))
                .unwrap_or_else(|| panic!("{id} has no resident source geometry"));
            assert!(
                *height > 0.0 && *height < matterweave_render::MAX_FLORA_HEIGHT_M,
                "{id} height {height} m is outside the wind path's range"
            );
        }
    }

    #[test]
    fn every_planned_placement_maps_to_a_drawable_instance() {
        let flora = LandscapeFlora::new().expect("prototypes must build");
        let mut plan = FloraPlan::default();
        // A plains eye and a forest eye between them exercise ground cover,
        // flowers and both tree kinds.
        for eye in [[0.0f32, 40.0, 0.0], [1200.5, 40.0, -800.0]] {
            landscape::plan_flora_into(
                crate::landscape::SEED,
                eye,
                &LANDSCAPE_FLORA_TIERS,
                MAX_PLANNED_SITES,
                MAX_PLANNED_TREES,
                &mut plan,
            );
            for placed in plan.sites.iter().chain(&plan.trees) {
                let instance = instance_for(&flora.resident, placed, eye)
                    .unwrap_or_else(|| panic!("{placed:?} produced no instance"));
                assert!((0.0..=1.0).contains(&instance.phase));
                assert!((0.0..=1.0).contains(&instance.bend));
                assert!(instance.height_m > 0.0);
                assert!(
                    (0.75..=1.25).contains(&instance.scale),
                    "{placed:?} scale {} outside the plan's variation range",
                    instance.scale
                );
                assert!(instance.yaw_quarters <= 3);
                assert!(instance.translation.iter().all(|v| v.is_finite()));
            }
        }
    }

    #[test]
    fn every_planned_placement_is_drawn_or_counted_as_dropped() {
        let mut flora = LandscapeFlora::new().expect("prototypes must build");
        let mut plan = FloraPlan::default();
        landscape::plan_flora_into(
            crate::landscape::SEED,
            [0.0f32, 40.0, 0.0],
            &LANDSCAPE_FLORA_TIERS,
            MAX_PLANNED_SITES,
            MAX_PLANNED_TREES,
            &mut plan,
        );
        let planned = plan.sites.len() + plan.trees.len();
        assert!(planned > 1_000, "a plains eye plans a field, got {planned}");
        let mut instances = Vec::new();
        let refused = fill_instances(&flora.resident, &plan, [0.0, 40.0, 0.0], &mut instances);
        // The catalogue has geometry for every id at every level, so nothing is
        // refused here and the two counts still agree.
        assert_eq!(refused, 0, "the full catalogue must draw every placement");
        assert_eq!(instances.len() + refused, planned);

        // Remove one prototype's geometry entirely: its placements must still be
        // accounted for, not silently vanish.
        let victim = prototype_for(&plan.sites[0].as_site());
        flora.resident.remove(&(victim, Lod::Source));
        for lod in RESIDENT_LODS {
            flora.resident.remove(&(victim, lod));
        }
        let mut instances = Vec::new();
        let refused = fill_instances(&flora.resident, &plan, [0.0, 40.0, 0.0], &mut instances);
        let expected = plan
            .sites
            .iter()
            .chain(&plan.trees)
            .filter(|placed| prototype_for(&placed.as_site()) == victim)
            .count();
        assert!(expected > 0, "{victim} must be planned for this eye");
        assert_eq!(refused, expected, "refused placements must be counted");
        assert_eq!(
            instances.len() + refused,
            planned,
            "every planned placement is drawn or dropped"
        );
    }

    #[test]
    fn per_site_variation_reaches_the_instances() {
        // The plan's variation is only real if it changes the drawn instance:
        // neighbouring placements of the same kind must differ in scale, and
        // their sway must not be in lockstep either.
        let flora = LandscapeFlora::new().expect("prototypes must build");
        let mut plan = FloraPlan::default();
        landscape::plan_flora_into(
            crate::landscape::SEED,
            [0.0f32, 40.0, 0.0],
            &LANDSCAPE_FLORA_TIERS,
            MAX_PLANNED_SITES,
            MAX_PLANNED_TREES,
            &mut plan,
        );
        let grass: Vec<_> = plan
            .sites
            .iter()
            .filter(|placed| placed.kind == FloraKind::GrassTuft)
            .collect();
        assert!(grass.len() > 32, "a plains eye holds a real field");
        let scales: std::collections::BTreeSet<u32> = grass
            .iter()
            .map(|placed| (instance_for(&flora.resident, placed, [0.0, 40.0, 0.0]).unwrap().scale * 1000.0) as u32)
            .collect();
        assert!(
            scales.len() >= 40,
            "grass instances collapsed onto {} sizes",
            scales.len()
        );
        let bends: std::collections::BTreeSet<u32> = grass
            .iter()
            .map(|placed| (instance_for(&flora.resident, placed, [0.0, 40.0, 0.0]).unwrap().bend * 1000.0) as u32)
            .collect();
        assert!(
            bends.len() >= 8,
            "grass compliance collapsed onto {} values",
            bends.len()
        );
        // Neighbours must not be twins: over the thousands of adjacent pairs a
        // plains eye plans, the share matching on *both* size and sway is the
        // birthday rate for a 51x51 field, far under one percent. An earlier
        // version of this mapping clamped grass compliance at 1.0 for every
        // site and measured 17 twin pairs here, so the check has teeth.
        let mut adjacent = 0usize;
        let mut matching = 0usize;
        for window in grass.windows(2) {
            if (window[0].x - window[1].x).abs() > 1 || (window[0].z - window[1].z).abs() > 1 {
                continue;
            }
            adjacent += 1;
            let a = instance_for(&flora.resident, window[0], [0.0, 40.0, 0.0]).unwrap();
            let b = instance_for(&flora.resident, window[1], [0.0, 40.0, 0.0]).unwrap();
            if a.scale == b.scale && a.bend == b.bend {
                matching += 1;
            }
        }
        assert!(adjacent > 1000, "only {adjacent} adjacent grass pairs");
        assert!(
            matching * 100 < adjacent,
            "{matching} of {adjacent} adjacent tufts share size and sway"
        );
    }

    #[test]
    fn distance_bands_select_coarser_geometry() {
        // Ground cover: the near band draws the source clump, the far band its
        // half level. There is no third band - beyond 48 m the field is ground
        // colour, because a 6.25 cm blade is sub-pixel there.
        assert_eq!(lod_for_tier(0), Lod::Source);
        assert_eq!(lod_for_tier(1), Lod::Half);
        for beyond in 2..8 {
            assert_eq!(lod_for_tier(beyond), Lod::Half);
        }
        // Trees carry their own bands out to the tree radius.
        assert_eq!(tree_lod(0), Lod::Source);
        assert_eq!(tree_lod(32), Lod::Source);
        assert_eq!(tree_lod(33), Lod::Half);
        assert_eq!(tree_lod(96), Lod::Half);
        assert_eq!(tree_lod(97), Lod::Quarter);
        assert_eq!(tree_lod(160), Lod::Quarter);
        let flora = LandscapeFlora::new().expect("prototypes must build");
        // The pooled indices really are different meshes, so a band change is
        // a geometry change rather than a relabelling.
        let near = flora.resident[&("tree_broadleaf_m", Lod::Source)].0;
        let far = flora.resident[&("tree_broadleaf_m", Lod::Quarter)].0;
        assert_ne!(near, far);
        let meshes = flora.runtime.meshes();
        assert!(
            meshes[far].indices.len() < meshes[near].indices.len(),
            "the coarse level must cost fewer triangles: {} vs {}",
            meshes[far].indices.len() / 3,
            meshes[near].indices.len() / 3
        );
        // A blade clump must coarsen into something far cheaper, or the far
        // band would not be worth drawing at all.
        let clump = flora.resident[&("grass_tuft_m", Lod::Source)].0;
        let clump_half = flora.resident[&("grass_tuft_m", Lod::Half)].0;
        assert!(
            meshes[clump_half].indices.len() < meshes[clump].indices.len(),
            "the half clump must cost fewer triangles: {} vs {}",
            meshes[clump_half].indices.len() / 3,
            meshes[clump].indices.len() / 3
        );
    }

    #[test]
    fn the_plan_caps_stay_inside_the_renderer_budget() {
        const { assert!(MAX_PLANNED_SITES + MAX_PLANNED_TREES <= MAX_FLORA_INSTANCES) };
        // The tree radius must reach past the outermost ground-cover band, or a
        // forest would stop being a forest at the edge of the field.
        assert!(LANDSCAPE_TREE_RADIUS_M > LANDSCAPE_FLORA_TIERS[LANDSCAPE_FLORA_TIERS.len() - 1].radius_m);
    }

    #[test]
    fn phases_are_bounded_and_decorrelate_neighbours() {
        let mut distinct = std::collections::BTreeSet::new();
        let mut equal_neighbours = 0usize;
        for x in -20..20 {
            for z in -20..20 {
                let phase = phase_of(x, z);
                assert!((0.0..=1.0).contains(&phase));
                distinct.insert((phase * 65535.0) as u32);
                // The property that matters: a plant must not sway in lockstep
                // with the one next to it, in either direction.
                if phase == phase_of(x + 1, z) || phase == phase_of(x, z + 1) {
                    equal_neighbours += 1;
                }
            }
        }
        assert_eq!(
            equal_neighbours, 0,
            "a plant shares its phase with a neighbour"
        );
        // And the field must not collapse onto a handful of phases. A 16-bit
        // hash over 1600 samples is *expected* to collide into about 1580
        // distinct values by the birthday bound, so this threshold guards a
        // collapse, not perfect uniqueness.
        assert!(
            distinct.len() > 1200,
            "the phase field collapsed: {} distinct values",
            distinct.len()
        );
        // Deterministic across calls, so a rebuild does not restart the sway.
        assert_eq!(phase_of(7, -13), phase_of(7, -13));
    }

    #[test]
    fn a_rebuild_is_only_asked_for_when_the_eye_leaves_its_cell() {
        let base = LandscapeFlora::cell_of([10.0, 0.0, -10.0]);
        assert_eq!(base, LandscapeFlora::cell_of([11.9, 50.0, -9.1]));
        assert_ne!(base, LandscapeFlora::cell_of([14.0, 0.0, -10.0]));
        assert_ne!(base, LandscapeFlora::cell_of([10.0, 0.0, -14.0]));
        // Negative coordinates floor toward minus infinity, so the cell grid
        // does not double in width around the origin.
        assert_eq!(
            LandscapeFlora::cell_of([-1.0, 0.0, -1.0]),
            LandscapeFlora::cell_of([-3.9, 0.0, -3.9])
        );
    }

    #[test]
    fn the_hysteresis_margin_exceeds_a_frame_of_flight() {
        // Flight speed is 90 m/s: at 60 Hz one frame covers 1.5 m, far inside
        // both the 4 m cell and the margin's 20 m. The scripted exercise path
        // moves a whole 4 m cell per frame, which the margin must still absorb.
        let flight_m = 90.0 / 60.0;
        let margin_m = (REBUILD_MARGIN_CELLS * REBUILD_CELL_M) as f32;
        assert!(flight_m < margin_m);
        assert!(REBUILD_CELL_M as f32 <= margin_m);
        // A whole cell of margin beyond the first, or a 4 m step would trigger
        // the instant it crossed a cell edge.
        const {
            assert!(REBUILD_MARGIN_CELLS >= 1);
        }
    }

    #[test]
    fn the_exercise_path_stays_well_under_the_rebuild_budget() {
        // One step of the scripted path is 200 m * 0.02 rad = 4 m, exactly one
        // rebuild cell, so a field that followed every cell edge would rebuild
        // on all 300 frames. The acceptance target is 60 or fewer.
        let mut anchor: Option<[i32; 2]> = None;
        let mut rebuilds = 0usize;
        for frame in 0..300 {
            let angle = frame as f32 * 0.02;
            let eye = [200.0 * angle.sin(), 60.0, 200.0 * angle.cos()];
            if !anchor.is_some_and(|anchor| anchor_covers(anchor, eye)) {
                // The field commits at the eye metre, exactly as `advance` does.
                anchor = Some([eye[0].floor() as i32, eye[2].floor() as i32]);
                rebuilds += 1;
            }
        }
        assert!(
            rebuilds <= 60,
            "the exercise path rebuilt {rebuilds} times in 300 frames"
        );
    }

    #[test]
    fn a_step_inside_the_margin_does_not_rebuild_and_a_step_past_it_does() {
        let anchor = [0, 0];
        // The margin is five cells; a step within it stays covered.
        let inside = [
            REBUILD_MARGIN_CELLS as f32 * REBUILD_CELL_M as f32,
            0.0,
            0.0,
        ];
        assert!(anchor_covers(anchor, inside));
        let outside = [
            (REBUILD_MARGIN_CELLS + 1) as f32 * REBUILD_CELL_M as f32,
            0.0,
            0.0,
        ];
        assert!(!anchor_covers(anchor, outside));
        // Distance is Chebyshev on the cell grid: a diagonal inside the margin
        // is still covered, and a diagonal past it is not.
        let diagonal = [inside[0], 0.0, inside[0]];
        assert!(anchor_covers(anchor, diagonal));
        let past_diagonal = [outside[0], 0.0, outside[0]];
        assert!(!anchor_covers(anchor, past_diagonal));
    }

    #[test]
    fn each_rebuild_publishes_exactly_the_planner_output_for_its_camera() {
        // `sync` commits only at the eye of the frame that completes an
        // advance, and that commit must be exactly the old planner's output for
        // the same camera position: sites, species, yaw, scale, height and
        // bend variation, and the wind phase derived from them.
        let mut field = FloraField::new(crate::landscape::SEED);
        let mut rebuilds = 0usize;
        for frame in 0..300 {
            let angle = frame as f32 * 0.02;
            let eye = [200.0 * angle.sin(), 60.0, 200.0 * angle.cos()];
            if field
                .anchor()
                .is_some_and(|anchor| anchor_covers(anchor, eye))
            {
                continue;
            }
            assert!(field.advance(
                eye,
                &LANDSCAPE_FLORA_TIERS,
                MAX_PLANNED_SITES,
                MAX_PLANNED_TREES,
                usize::MAX
            ));
            assert_eq!(
                field.plan(),
                &landscape::plan_flora(
                    crate::landscape::SEED,
                    eye,
                    &LANDSCAPE_FLORA_TIERS,
                    MAX_PLANNED_SITES,
                    MAX_PLANNED_TREES,
                ),
                "rebuild {rebuilds} committed at frame {frame}"
            );
            rebuilds += 1;
        }
        assert!(rebuilds <= 60, "the path rebuilt {rebuilds} times");
    }
}
