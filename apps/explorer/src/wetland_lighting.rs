//! Production wetland proxy lighting: bounded GI and reflection for the
//! authored play area, built from the geometry the renderer actually holds.
//!
//! The wetland's visible world is mesh-only: the authoritative [`World`] is
//! empty and every object is a detail instance or a physics body. The renderer's
//! indirect and reflection volumes therefore cannot be packed from world data —
//! they need a [`MeshProxy`], and the renderer rejects a proxy-less publication
//! while mesh geometry is resident.
//!
//! Each frame this module:
//!
//! 1. builds a box-local proxy from the *installed* detail selection and the
//!    resident mesh pool, plus the merged dynamic (physics) mesh;
//! 2. attaches it to one [`IndirectVolume`] inside an authored coverage box;
//! 3. advances that volume by a fixed CPU budget and publishes it once complete;
//! 4. publishes the matching [`ReflectionVolume`], because the frame's geometry
//!    installs are what retire the previous publications.
//!
//! Invariants:
//!
//! - Publication happens only after a successful detail install. Every renderer
//!   geometry path (`replace_static_scene`, `update_static_instances`,
//!   `upload_dynamic`) disables GI, and a sun change disables it at render time,
//!   so the proxy must describe the selection the renderer accepted and the
//!   publication must follow those calls in the same frame.
//! - A frame whose install is unavailable does no lighting work. Renderer
//!   installs are transactional — a rejected install leaves the previous
//!   resident scene in place — so the last accepted publication still describes
//!   what is drawn and is deliberately not withdrawn; nothing is built or
//!   published from a selection the renderer has not accepted.
//! - Rebuilds are digest-gated. An install that changes the selection but not
//!   the proxy's occupied cells — a sub-cell move, a LOD swap that keeps the
//!   cells — discards the freshly built proxy and keeps the cached volume, so
//!   camera-driven churn cannot starve convergence. A changed footprint retires
//!   the stale publication before the volume recomputes. This gate absorbs
//!   camera and LOD churn only; body motion that crosses cell boundaries is a
//!   changed footprint and is disclosed in "Scope and limitations".
//! - The volume advances by a fixed work/ray budget per frame, and a partially
//!   accumulated key is never published.
//! - Any build, attach, update or upload error withdraws indirect and reflection
//!   and is reported once; the session keeps running without them. A failed
//!   attach drops the superseded representation, so the withdrawn state cannot
//!   republish geometry the renderer has already retired.
//!
//! # Scope and limitations
//!
//! These are deliberate bounds of this slice, not measurements:
//!
//! - **One authored region.** GI and reflection exist for a 20x10x20 m box
//!   anchored on the destruction clearing — the flat play area on the ground
//!   route where the physics bodies rest — so a real body move or edit lands
//!   inside the published region. This is a bounded local lighting volume, not
//!   full-world GI: nothing outside the box participates, and the box does not
//!   follow the camera (a travelling or multi-volume scheme is future work).
//! - **One material per pool entry.** `MeshGeometry` accepts exactly one
//!   material per prototype, while the raster path draws per-cell materials. A
//!   proxy cell therefore carries the dominant drawn colour of its mesh. The
//!   authoritative scene materials and every game rule are untouched; the proxy
//!   is a derived representation.
//! - **Per-body materials are not represented.** The merged physics mesh is one
//!   pool entry with one material, so a moving body of a minority material
//!   contributes the dominant material's albedo and mirror strength.
//! - **Cell resolution.** The proxy identity is the rasterised cell footprint:
//!   a sub-cell move that keeps every triangle inside the cells it already
//!   occupied leaves it unchanged, and cached radiance for that identical
//!   representation stays valid.
//! - **Continuous cell-crossing body motion withdraws GI until convergence.**
//!   Every drawn-vertex change rebuilds the proxy, and every footprint change
//!   retires the publication; while a body keeps crossing cell boundaries the
//!   footprint keeps changing, so indirect radiance is off while the volume
//!   recomputes and stays off until the fixed `UPDATE_BUDGET` has finished it.
//!   The digest gate absorbs camera and LOD churn, not body motion.
//!   Dependency retention (`INDIRECT_DEPENDENCY_BYTES`) shortens that recompute:
//!   an edit invalidates only the completed faces whose recorded proxy cells
//!   changed, so convergence is bounded by the invalidated set instead of the
//!   box. It does not make a partially recomputed volume publishable —
//!   complete-only publication is unchanged — so a moving body still turns GI
//!   off for the frames its recompute needs. Continuous moving-cell/body GI
//!   continuity is an open functional requirement, not part of this slice and
//!   not Phase B cost/thermal work.
//! - **No performance claim.** The per-frame budget is a bounded CPU work slice
//!   over bounded volumes; nothing here is a device measurement.

use matterweave_core::{Mesh, World};
use matterweave_detail::{material, material_color};
use matterweave_render::indirect::{
    IndirectVolume, MeshGeometry, MeshProxy, ProxyEdit, UpdateBudget,
};
use matterweave_render::reflection::{MaterialTable, ReflectionVolume, DEFAULT_TRACE_STEPS};
use matterweave_render::{Renderer, StaticInstance, Sun};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

/// Half extents in cells of the one coverage box this slice publishes. Cells are
/// the world's own 1 m voxel grid, so the box is 20x10x20 m and 4000 cells: an
/// authored aspect ratio that matches the clearing, not the largest box either
/// engine cap allows. The indirect volume's 6-face-per-cell residency cap
/// (24,576 face slots) permits 4096 cells, and the reflection volume allows 64
/// cells per axis, so a 16x16x16 box (also 4096) fits both. A test pins this box
/// against both caps.
const BOX_HALF_EXTENT: [i32; 3] = [10, 5, 10];
const BOX_DIMENSIONS: [u32; 3] = [
    (BOX_HALF_EXTENT[0] * 2) as u32,
    (BOX_HALF_EXTENT[1] * 2) as u32,
    (BOX_HALF_EXTENT[2] * 2) as u32,
];
/// Diffuse hemisphere samples per exposed proxy face.
const SAMPLES: u32 = 16;
/// Gather distance in metres: longer than the box, so every ray inside it is
/// bounded by the coverage box rather than by the sample distance.
const GATHER_DISTANCE_M: f32 = 24.0;
/// Fixed CPU work per frame: at most this many DDA rays and face inspections.
/// The volume only publishes when the whole grid has been sampled, so the budget
/// trades convergence latency for frame cost. Tuning is measurement work.
const UPDATE_BUDGET: UpdateBudget = UpdateBudget {
    rays: 1024,
    work: 8192,
};
/// Memory cap for opt-in exact indirect dependency tracking, in bytes. The
/// tracker records the proxy cells each completed face depends on, so a body
/// move or edit recomputes only the faces whose recorded cells changed instead
/// of the whole 4000-cell box. Bitsets are allocated lazily per sampled face
/// under this cap: a face that does not fit is recomputed on every edit instead
/// of being retained, so the cap bounds memory and degrades retention, never
/// correctness. One face bitset is `ceil(4000 / 64) * 8 = 504` bytes, so this
/// cap covers roughly twelve thousand sampled faces.
const INDIRECT_DEPENDENCY_BYTES: usize = 6 * 1024 * 1024;
/// Replacement epoch of the authoritative world. The wetland world is created
/// once per session and never edited (all edits are detail-scene edits), so the
/// session's epoch is constant; a new [`Runtime`] builds a new scheduler.
const SOURCE_EPOCH: u64 = 0;
/// Detail catalogue materials that own a drawn colour. A test fails if the
/// catalogue gains or loses an id without this list following it.
const CATALOGUE_MATERIALS: [u8; 27] = [
    10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
    46, 47, 48,
];
/// Palette id reserved for the merged physics mesh. It is outside the detail
/// catalogue, never assigned to a detail pool entry (the reverse palette lookup
/// only accepts catalogue ids), and carries the drawn colour of the merged mesh.
const DYNAMIC_MATERIAL: u8 = 255;
/// Mirror strength of the surface materials that are drawn wet in this scene.
/// Reflection is opt-in: every other material stays nonreflective. These are
/// art-facing palette constants, not engine or gameplay rules.
const MIRROR_WATER: f32 = 0.8;
const MIRROR_BANK_STONE: f32 = 0.25;
/// Frames between host-visible transition lines; the first line and every
/// publication-state change still print, so a smoke run and a device logcat see
/// the production path without per-frame noise.
const REPORT_INTERVAL_FRAMES: u64 = 30;

/// What the frame's detail install did, as the frame path observed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallState {
    /// The renderer already holds exactly the installed selection.
    Current,
    /// The renderer accepted the current selection this frame.
    Installed,
    /// The renderer rejected the selection, or preparing it failed: resident
    /// geometry and the prepared selection may disagree.
    Unavailable,
}

/// The frame's proxy sources. Every slice is owned by the frame path; this
/// module never mutates authoritative data.
#[derive(Clone, Copy)]
pub(crate) struct FrameSource<'a> {
    /// The selection the renderer accepted at its last successful install.
    pub(crate) installed: &'a [StaticInstance],
    /// Resident derived mesh pool; instance `prototype` fields index this.
    pub(crate) meshes: &'a [Mesh],
    /// Merged, CPU-transformed, world-space physics mesh (may be empty).
    pub(crate) dynamic: &'a Mesh,
    /// Authoritative world. Empty in the wetland, and never edited here.
    pub(crate) world: &'a World,
    /// Sun of this frame's lighting settings.
    pub(crate) sun: Sun,
}

/// Publication surface this scheduler drives. [`Renderer`] is the production
/// implementation; tests drive a recording double through the same calls.
pub(crate) trait Publication {
    fn publish_indirect(
        &mut self,
        volume: &IndirectVolume,
        world: &World,
        source_epoch: u64,
        mesh_digest: Option<u64>,
    ) -> Result<(), String>;
    /// Returns the uploaded bytes.
    fn publish_reflection(
        &mut self,
        volume: &ReflectionVolume,
        world: &World,
        source_epoch: u64,
        mesh_digest: Option<u64>,
    ) -> Result<usize, String>;
    /// Retire both publications. Allocated GPU storage stays resident.
    fn withdraw_lighting(&mut self);
    /// Whether matching indirect data is published for this frame's geometry.
    fn indirect_live(&self) -> bool;
    /// Whether matching reflection data is published for this frame's geometry.
    fn reflection_live(&self) -> bool;
}

impl Publication for Renderer {
    fn publish_indirect(
        &mut self,
        volume: &IndirectVolume,
        world: &World,
        source_epoch: u64,
        mesh_digest: Option<u64>,
    ) -> Result<(), String> {
        self.upload_indirect(volume, world, source_epoch, mesh_digest)
    }

    fn publish_reflection(
        &mut self,
        volume: &ReflectionVolume,
        world: &World,
        source_epoch: u64,
        mesh_digest: Option<u64>,
    ) -> Result<usize, String> {
        self.upload_reflection(volume, world, source_epoch, mesh_digest)
            .map(|stats| stats.bytes)
    }

    fn withdraw_lighting(&mut self) {
        self.disable_indirect();
        self.disable_reflection();
    }

    fn indirect_live(&self) -> bool {
        self.indirect_enabled()
    }

    fn reflection_live(&self) -> bool {
        self.reflection_enabled()
    }
}

/// Outcome of one frame's lighting step, for logging and tests. Publication
/// flags are renderer validity, never a pixel or timing claim.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Summary {
    pub(crate) indirect_live: bool,
    pub(crate) reflection_live: bool,
    /// This call rebuilt the box-local proxy.
    pub(crate) proxy_rebuilt: bool,
    /// Occupied cells of the attached representation.
    pub(crate) proxy_cells: usize,
    /// Meshes in the box-local pool the attached proxy was built from.
    pub(crate) proxy_pool_meshes: usize,
    /// Identity of the attached proxy.
    pub(crate) digest: Option<u64>,
    /// Upper bound on remaining indirect work units for the current key.
    pub(crate) pending_work: usize,
    /// Completed indirect faces this frame's proxy edit retained unchanged.
    pub(crate) retained_faces: usize,
    /// Completed indirect faces this frame's proxy edit invalidated.
    pub(crate) invalidated_faces: usize,
    /// Face slots the current key still has to resolve.
    pub(crate) dirty_faces: usize,
    /// Resident bytes of the attached volume's dependency tracker.
    pub(crate) dependency_bytes: usize,
    /// Wall time of this call's proxy build on the calling thread.
    pub(crate) rebuild_ms: f64,
}

/// One frame's production proxy lighting state machine.
pub(crate) struct WetlandLighting {
    origin: [i32; 3],
    volume: Option<IndirectVolume>,
    /// Drawn colour of the merged physics mesh in the live volume's palette;
    /// the palette is immutable, so a change rebuilds the volume.
    palette_dynamic: Option<[f32; 3]>,
    /// Packed reflection source of the attached representation.
    reflection: Option<ReflectionVolume>,
    /// The stored pack no longer matches the current source. The next proxy
    /// build repacks it from that build's proxy and clears this flag, so a
    /// stale pack whose digest is unchanged clears instead of forcing a rebuild
    /// every frame.
    reflection_stale: bool,
    /// Dynamic-mesh fingerprint of the last successful build.
    built_dynamic: Option<u64>,
    /// A build or attach failed; retry on the next accepted install instead of
    /// every frame, and never publish the withheld representation in between.
    build_failed: bool,
    attached_cells: usize,
    attached_pool_meshes: usize,
    bounds: LocalBounds,
    /// Light identity the live publication was computed for.
    published_sun: Option<[u32; 4]>,
    proxy_rebuilds: u64,
    updates: u64,
    last_error: Option<String>,
    last_reported: Option<(bool, bool, Option<u64>)>,
    last_report_at: u64,
    /// Test-only fault armed by a regression to fail the next attach step; see
    /// [`AttachFault`]. Production builds compile the field and its checks out.
    #[cfg(test)]
    armed_attach_fault: Option<AttachFault>,
}

/// Test-only failure injection for the two `attach` steps that can fail in
/// production only on allocation or pack errors. A regression arms one, so the
/// state transition after a failed attach is exercised deterministically without
/// an allocator fault.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttachFault {
    /// `IndirectVolume::new` fails before a superseded volume is replaced.
    Volume,
    /// `ReflectionVolume::pack_with_mesh` fails after the footprint change.
    Reflection,
}

#[cfg(test)]
impl AttachFault {
    fn message(self) -> &'static str {
        match self {
            Self::Volume => "injected indirect volume allocation failure",
            Self::Reflection => "injected reflection pack failure",
        }
    }
}

impl WetlandLighting {
    /// A session scheduler for the authored region around `anchor_m` (the
    /// destruction-clearing landmark in metres). Construction cannot fail; the
    /// volume is built from the first successful proxy and a failure there is
    /// contained like every other error.
    pub(crate) fn new(anchor_m: [f32; 3]) -> Self {
        Self {
            origin: box_origin(anchor_m),
            volume: None,
            palette_dynamic: None,
            reflection: None,
            reflection_stale: false,
            built_dynamic: None,
            build_failed: false,
            attached_cells: 0,
            attached_pool_meshes: 0,
            bounds: LocalBounds::default(),
            published_sun: None,
            proxy_rebuilds: 0,
            updates: 0,
            last_error: None,
            last_reported: None,
            last_report_at: 0,
            #[cfg(test)]
            armed_attach_fault: None,
        }
    }

    /// Advance one frame. `Ok` means the frame is consistent — including the
    /// legitimate states where the volume is still accumulating or the install
    /// was unavailable. `Err` reports a failure that has already withdrawn both
    /// publications; the repeated identical failure is not reported twice, so a
    /// caller may log every `Err` it receives.
    pub(crate) fn update(
        &mut self,
        sink: &mut impl Publication,
        source: &FrameSource<'_>,
        install: InstallState,
    ) -> Result<Summary, String> {
        self.updates = self.updates.saturating_add(1);
        let outcome = self.step(sink, source, install);
        let summary = match &outcome {
            Ok(summary) => *summary,
            Err(_) => {
                // No stale publication may outlive the source it describes.
                sink.withdraw_lighting();
                self.published_sun = None;
                self.idle_summary(sink)
            }
        };
        self.report(&summary);
        match outcome {
            Ok(_) => {
                self.last_error = None;
                Ok(summary)
            }
            Err(error) => {
                if self.last_error.as_deref() == Some(error.as_str()) {
                    // The identical failure is not reported twice, so a caller
                    // may log every `Err` it receives.
                    Ok(summary)
                } else {
                    self.last_error = Some(error.clone());
                    Err(error)
                }
            }
        }
    }

    /// The state of a frame that does no lighting work: the attached
    /// representation and the live publication flags, never a claim that
    /// something was built or published.
    fn idle_summary(&self, sink: &impl Publication) -> Summary {
        Summary {
            indirect_live: sink.indirect_live(),
            reflection_live: sink.reflection_live(),
            proxy_cells: self.attached_cells,
            proxy_pool_meshes: self.attached_pool_meshes,
            digest: self.volume.as_ref().and_then(IndirectVolume::mesh_digest),
            pending_work: self.volume.as_ref().map_or(0, IndirectVolume::pending_work),
            dirty_faces: self.volume.as_ref().map_or(0, IndirectVolume::dirty_faces),
            dependency_bytes: self
                .volume
                .as_ref()
                .and_then(IndirectVolume::retention_status)
                .map_or(0, |status| status.resident_bytes),
            ..Summary::default()
        }
    }

    fn step(
        &mut self,
        sink: &mut impl Publication,
        source: &FrameSource<'_>,
        install: InstallState,
    ) -> Result<Summary, String> {
        let mut summary = Summary::default();
        if install == InstallState::Unavailable {
            // The renderer's geometry installs are transactional: a rejected
            // install retains the previous resident scene, so the last accepted
            // publication still describes what is drawn and is deliberately left
            // in place. Nothing is built or published from a selection the
            // renderer has not accepted.
            return Ok(self.idle_summary(sink));
        }
        let dynamic_fingerprint = mesh_fingerprint(source.dynamic);
        let rebuild = install == InstallState::Installed
            || (!self.build_failed
                && (self.volume.is_none()
                    || self.built_dynamic != Some(dynamic_fingerprint)
                    || self.reflection_stale));
        if rebuild {
            let built = match self.build_proxy(source) {
                Ok(built) => built,
                Err(error) => {
                    self.build_failed = true;
                    return Err(error);
                }
            };
            let cells = built.proxy.occupied_cells();
            let pool_meshes = built.pool_meshes;
            let millis = built.millis;
            let edit = match self.attach(sink, source, built) {
                Ok(edit) => edit,
                Err(error) => {
                    // An attach failure breaks the same contract as a failed build:
                    // the renderer holds the newly installed geometry, so the
                    // superseded representation must not survive to be republished
                    // by the no-rebuild path. Drop it and withhold until the next
                    // accepted install.
                    self.volume = None;
                    self.palette_dynamic = None;
                    self.reflection = None;
                    self.reflection_stale = false;
                    self.attached_cells = 0;
                    self.attached_pool_meshes = 0;
                    self.build_failed = true;
                    return Err(error);
                }
            };
            self.build_failed = false;
            self.built_dynamic = Some(dynamic_fingerprint);
            self.proxy_rebuilds = self.proxy_rebuilds.saturating_add(1);
            self.attached_cells = cells;
            self.attached_pool_meshes = pool_meshes;
            summary.proxy_rebuilt = true;
            summary.retained_faces = edit.retained_faces;
            summary.invalidated_faces = edit.invalidated_faces;
            summary.dirty_faces = edit.dirty_faces;
            summary.dependency_bytes = edit.dependency_bytes;
            summary.rebuild_ms = millis;
        } else if self.build_failed {
            // A rebuild failed and the source has not moved since: nothing may
            // be published from the superseded representation, and retrying now
            // would only repeat the same failure.
            return Ok(self.idle_summary(sink));
        }
        let Some(volume) = self.volume.as_mut() else {
            return Ok(summary);
        };
        let digest = volume.mesh_digest();
        // Values computed for another sun must not stay visible even before the
        // renderer's own per-frame sun check retires them.
        let sun_key = sun_key(source.sun);
        if sink.indirect_live() && self.published_sun != Some(sun_key) {
            sink.withdraw_lighting();
            self.published_sun = None;
        }
        // A volume that is not valid for the current source or light is either
        // partial or keyed to a superseded one; `update` restarts it from the
        // first face when the key changed. A complete, current volume needs no
        // work at all.
        if !volume.valid_for(source.world, SOURCE_EPOCH, source.sun) {
            volume.update(source.world, SOURCE_EPOCH, source.sun, UPDATE_BUDGET)?;
        }
        let pending_work = volume.pending_work();
        if !sink.indirect_live() && volume.valid_for(source.world, SOURCE_EPOCH, source.sun) {
            sink.publish_indirect(volume, source.world, SOURCE_EPOCH, digest)?;
            self.published_sun = Some(sun_key);
        }
        summary.proxy_cells = self.attached_cells;
        summary.proxy_pool_meshes = self.attached_pool_meshes;
        summary.digest = digest;
        summary.pending_work = pending_work;
        summary.dirty_faces = volume.dirty_faces();
        if let Some(status) = volume.retention_status() {
            summary.dependency_bytes = status.resident_bytes;
        }
        summary.indirect_live = sink.indirect_live();
        summary.reflection_live = sink.reflection_live();
        if !summary.reflection_live {
            match self.reflection.as_ref() {
                Some(pack) if pack.valid_for_scene(source.world, SOURCE_EPOCH, digest) => {
                    sink.publish_reflection(pack, source.world, SOURCE_EPOCH, digest)?;
                    summary.reflection_live = true;
                }
                // A pack that no longer matches the source is rebuilt with the
                // next proxy build; the published state stays withdrawn.
                Some(_) | None => self.reflection_stale = true,
            }
        }
        Ok(summary)
    }

    /// Build the box-local proxy for this frame's sources. Nothing authoritative
    /// is read beyond the installed selection, the resident meshes and the
    /// merged dynamic mesh.
    fn build_proxy(&mut self, source: &FrameSource<'_>) -> Result<Built, String> {
        let start = Instant::now();
        let mut pool = BoxPool::default();
        let mut remap: Vec<Option<usize>> = vec![None; source.meshes.len()];
        for instance in source.installed {
            let mesh = source.meshes.get(instance.prototype).ok_or_else(|| {
                format!(
                    "installed instance references pool entry {} of {}",
                    instance.prototype,
                    source.meshes.len()
                )
            })?;
            if mesh.indices.is_empty() {
                continue;
            }
            let bounds = self.bounds.get(instance.prototype, mesh);
            if !bounds_meets_box(bounds, instance, self.origin) {
                continue;
            }
            let compact = match remap[instance.prototype] {
                Some(index) => index,
                None => {
                    let material = dominant_material(mesh).ok_or_else(|| {
                        format!(
                            "resident mesh {} carries no detail palette colour",
                            instance.prototype
                        )
                    })?;
                    let index = pool.meshes.len();
                    pool.meshes.push(copy_mesh(mesh));
                    pool.materials.push(material);
                    remap[instance.prototype] = Some(index);
                    index
                }
            };
            pool.instances.push(StaticInstance {
                prototype: compact,
                ..*instance
            });
        }
        // The merged physics mesh is world space: one identity placement as the
        // last instance, so a moving body owns a cell it shares with static
        // detail instead of inheriting it.
        let dynamic_color = dynamic_pool_entry(source.dynamic, &mut pool)?;
        let proxy = MeshProxy::build(
            &MeshGeometry {
                meshes: &pool.meshes,
                instances: &pool.instances,
                materials: &pool.materials,
            },
            self.origin,
            BOX_DIMENSIONS,
        )?;
        Ok(Built {
            proxy,
            dynamic_color,
            pool_meshes: pool.meshes.len(),
            millis: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Attach a freshly built proxy and pack its matching reflection source.
    /// A footprint change retires both publications: the cached indirect values
    /// and the packed mirror grid describe the previous representation.
    ///
    /// The indirect volume replaces its proxy through exact dependency
    /// retention, so only the completed faces whose recorded proxy cells changed
    /// are recomputed. Publication is unchanged: the volume still uploads only a
    /// complete key, so the retire-then-reconverge sequence is the same and the
    /// retained values only shorten it. `Ok(ProxyEdit::default())` means the
    /// footprint did not change and nothing was attached.
    fn attach(
        &mut self,
        sink: &mut impl Publication,
        source: &FrameSource<'_>,
        built: Built,
    ) -> Result<ProxyEdit, String> {
        let palette_dynamic = built.dynamic_color;
        if self.volume.is_none() || self.palette_dynamic != palette_dynamic {
            // The palette is fixed at construction, so the reserved dynamic
            // entry changing colour means a new volume.
            #[cfg(test)]
            if self.armed_attach_fault == Some(AttachFault::Volume) {
                return Err(AttachFault::Volume.message().into());
            }
            self.volume = Some(IndirectVolume::new(
                self.origin,
                BOX_DIMENSIONS,
                SAMPLES,
                GATHER_DISTANCE_M,
                palette(palette_dynamic),
            )?);
            if let Some(volume) = self.volume.as_mut() {
                // Opt in to bounded exact dependency retention for this volume.
                // A failure here is an attach failure like every other: the
                // caller drops the superseded representation and withholds.
                volume.enable_proxy_retention(INDIRECT_DEPENDENCY_BYTES)?;
            }
            self.palette_dynamic = palette_dynamic;
            self.published_sun = None;
        }
        let digest = built.proxy.digest();
        let changed = {
            let volume = self
                .volume
                .as_mut()
                .ok_or("indirect volume missing after construction")?;
            volume.mesh_digest() != Some(digest)
        };
        if changed {
            sink.withdraw_lighting();
            self.published_sun = None;
            self.reflection = None;
        }
        if self.reflection.is_none() || self.reflection_stale {
            // The pack is a bake of the current proxy: repack whenever the
            // stored one is missing or marked stale, including a stale pack
            // whose digest is unchanged (the footprint survived a world edit).
            #[cfg(test)]
            if self.armed_attach_fault == Some(AttachFault::Reflection) {
                return Err(AttachFault::Reflection.message().into());
            }
            let table = mirror_table(palette(self.palette_dynamic))?;
            self.reflection = Some(ReflectionVolume::pack_with_mesh(
                source.world,
                SOURCE_EPOCH,
                self.origin,
                BOX_DIMENSIONS,
                &table,
                DEFAULT_TRACE_STEPS,
                &built.proxy,
            )?);
            self.reflection_stale = false;
        }
        let mut edit = ProxyEdit::default();
        if changed {
            if let Some(volume) = self.volume.as_mut() {
                // Exact dependency retention: only the completed faces whose
                // recorded proxy cells changed are recomputed. The representation
                // identity and every publication check are unchanged.
                edit = volume.replace_mesh_proxy(Some(built.proxy));
            }
        }
        Ok(edit)
    }

    /// One host-visible line per publication-state change, rate limited so a
    /// walking camera cannot flood the log with digest churn.
    fn report(&mut self, summary: &Summary) {
        let state = (
            summary.indirect_live,
            summary.reflection_live,
            summary.digest,
        );
        if self.last_reported == Some(state) {
            return;
        }
        let first = self.last_reported.is_none();
        let due = self.updates.saturating_sub(self.last_report_at) >= REPORT_INTERVAL_FRAMES;
        if !first && !due {
            return;
        }
        self.last_reported = Some(state);
        self.last_report_at = self.updates;
        eprintln!(
            "WETLAND PROXY LIGHTING: gi={} reflection={} cells={} pool_meshes={} digest={:?} \
pending={} retained={} invalidated={} dirty={} dependency_kib={} rebuild_ms={:.2}",
            summary.indirect_live,
            summary.reflection_live,
            summary.proxy_cells,
            summary.proxy_pool_meshes,
            summary.digest,
            summary.pending_work,
            summary.retained_faces,
            summary.invalidated_faces,
            summary.dirty_faces,
            summary.dependency_bytes / 1024,
            summary.rebuild_ms,
        );
    }
}

/// One built proxy and what it cost to build.
struct Built {
    proxy: MeshProxy,
    dynamic_color: Option<[f32; 3]>,
    pool_meshes: usize,
    millis: f64,
}

/// Box-local mesh pool: only the resident prototypes an installed instance can
/// place inside the coverage box, plus the merged dynamic mesh. Cloning that
/// subset is what bounds a rebuild by the box instead of by the whole scene,
/// and it is the only way `MeshGeometry` can carry the dynamic mesh alongside a
/// resident pool it does not own.
#[derive(Default)]
struct BoxPool {
    meshes: Vec<Mesh>,
    materials: Vec<u8>,
    instances: Vec<StaticInstance>,
}

/// A deep copy of a resident mesh. `Mesh` is deliberately not `Clone` (copying
/// one is a real allocation decision), so the box-local pool copies its two
/// vectors explicitly and keeps the source revision as the cache identity.
fn copy_mesh(mesh: &Mesh) -> Mesh {
    Mesh {
        vertices: mesh.vertices.clone(),
        indices: mesh.indices.clone(),
        revision: mesh.revision,
    }
}

/// Local-space bounds of one mesh as `(low, high)` corners.
type Bounds = ([f32; 3], [f32; 3]);

/// Local-space bounds of resident pool meshes, cached by pool index and the
/// mesh revision the pool stores with it. A pool slot holds one prototype and
/// derived level, and its content changes only with a new source revision, so
/// this cache is exact without copying mesh data.
#[derive(Default)]
struct LocalBounds {
    entries: Vec<Option<(u64, Bounds)>>,
}

impl LocalBounds {
    fn get(&mut self, index: usize, mesh: &Mesh) -> Bounds {
        if self.entries.len() <= index {
            self.entries.resize(index + 1, None);
        }
        if let Some((revision, bounds)) = self.entries[index] {
            if revision == mesh.revision {
                return bounds;
            }
        }
        let bounds = mesh_bounds(mesh);
        self.entries[index] = Some((mesh.revision, bounds));
        bounds
    }
}

/// Coverage box origin in cells for a landmark anchor in metres: symmetric
/// around the anchor and rounded to the world cell grid, so a fixed scene has a
/// fixed, reproducible box that the camera never moves.
fn box_origin(anchor_m: [f32; 3]) -> [i32; 3] {
    std::array::from_fn(|axis| anchor_m[axis].round() as i32 - BOX_HALF_EXTENT[axis])
}

/// Whether one placed prototype's world bounds can touch the coverage box. The
/// test is conservative (contacts count), so a dropped instance can never have
/// marked a cell.
fn bounds_meets_box(bounds: Bounds, instance: &StaticInstance, origin: [i32; 3]) -> bool {
    let (low, high) = instance_bounds(bounds, instance);
    (0..3).all(|axis| {
        let box_low = origin[axis] as f32;
        let box_high = box_low + BOX_DIMENSIONS[axis] as f32;
        low[axis] <= box_high && high[axis] >= box_low
    })
}

/// World bounds of one placement: a quarter-turn Y rotation maps the local box
/// to the same box with the x/z extents swapped, whatever the sign convention,
/// and the placement translates it.
fn instance_bounds(bounds: Bounds, instance: &StaticInstance) -> Bounds {
    let (local_low, local_high) = bounds;
    let (x_low, x_high, z_low, z_high) = match instance.yaw_quarters {
        1 | 3 => (local_low[2], local_high[2], local_low[0], local_high[0]),
        _ => (local_low[0], local_high[0], local_low[2], local_high[2]),
    };
    let translation = instance.translation;
    (
        [
            translation[0] + x_low,
            translation[1] + local_low[1],
            translation[2] + z_low,
        ],
        [
            translation[0] + x_high,
            translation[1] + local_high[1],
            translation[2] + z_high,
        ],
    )
}

fn mesh_bounds(mesh: &Mesh) -> Bounds {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        for axis in 0..3 {
            low[axis] = low[axis].min(vertex.position[axis]);
            high[axis] = high[axis].max(vertex.position[axis]);
        }
    }
    (low, high)
}

/// Append the merged physics mesh as one identity placement, returning the
/// colour its reserved palette entry must carry.
fn dynamic_pool_entry(dynamic: &Mesh, pool: &mut BoxPool) -> Result<Option<[f32; 3]>, String> {
    if dynamic.indices.is_empty() {
        return Ok(None);
    }
    let color = dominant_color(dynamic)
        .ok_or("merged dynamic mesh has no finite in-range vertex colour")?;
    let index = pool.meshes.len();
    pool.meshes.push(copy_mesh(dynamic));
    pool.materials.push(DYNAMIC_MATERIAL);
    pool.instances.push(StaticInstance {
        prototype: index,
        translation: [0.0; 3],
        yaw_quarters: 0,
    });
    Ok(Some(color))
}

/// Linear diffuse reflectances, indexed by the proxy's material ids: every
/// detail catalogue colour, black for air and unknown ids, and the merged
/// physics mesh's own drawn colour under its reserved id.
fn palette(dynamic: Option<[f32; 3]>) -> [[f32; 3]; 256] {
    let mut palette = [[0.0; 3]; 256];
    for material in CATALOGUE_MATERIALS {
        palette[material as usize] = material_color(material);
    }
    if let Some(color) = dynamic {
        palette[DYNAMIC_MATERIAL as usize] = color;
    }
    palette
}

/// Reflection palette: the same linear reflectances plus the mirror strengths of
/// the materials this scene draws wet.
fn mirror_table(colors: [[f32; 3]; 256]) -> Result<MaterialTable, String> {
    let mut table = MaterialTable::new(colors)?;
    table.set_mirror(material::WATER, MIRROR_WATER)?;
    table.set_mirror(material::BANK_STONE, MIRROR_BANK_STONE)?;
    Ok(table)
}

/// The detail catalogue id whose drawn colour covers most of `mesh`'s vertices.
/// `MeshGeometry` accepts one material per prototype while the mesher paints per
/// quad, so the dominant drawn colour is the closest single-material
/// approximation that still comes from authoritative materials. `None` when no
/// vertex carries a catalogue colour, which the caller reports as an error.
fn dominant_material(mesh: &Mesh) -> Option<u8> {
    let mut counts = [0u32; 256];
    for vertex in &mesh.vertices {
        if let Some(material) = catalogue_material_of(vertex.color) {
            counts[material as usize] = counts[material as usize].saturating_add(1);
        }
    }
    let mut best: Option<(u8, u32)> = None;
    for material in CATALOGUE_MATERIALS {
        let count = counts[material as usize];
        if count > 0 && best.is_none_or(|(_, best_count)| count > best_count) {
            best = Some((material, count));
        }
    }
    best.map(|(material, _)| material)
}

/// Exact reverse lookup of the detail palette. Bit comparison, so a palette
/// colour matches only the id that produced it.
fn catalogue_material_of(color: [f32; 3]) -> Option<u8> {
    let bits = color.map(f32::to_bits);
    CATALOGUE_MATERIALS
        .into_iter()
        .find(|&material| material_color(material).map(f32::to_bits) == bits)
}

/// Most common exact vertex colour of a mesh, validated for the palette
/// contract. Used for the merged physics mesh, whose drawn colours are the
/// physics-side palette rather than the detail catalogue.
fn dominant_color(mesh: &Mesh) -> Option<[f32; 3]> {
    let mut seen: Vec<([u32; 3], u32, [f32; 3])> = Vec::new();
    for vertex in &mesh.vertices {
        let bits = vertex.color.map(f32::to_bits);
        match seen.iter_mut().find(|(color, _, _)| *color == bits) {
            Some((_, count, _)) => *count = count.saturating_add(1),
            None => seen.push((bits, 1, vertex.color)),
        }
    }
    let (_, _, color) = seen.into_iter().max_by_key(|(_, count, _)| *count)?;
    color
        .iter()
        .all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel))
        .then_some(color)
}

/// Identity of the merged dynamic mesh. `Mesh::revision` tracks physics events
/// rather than interpolated positions, so a moving body can leave it unchanged;
/// the drawn geometry itself is hashed. Compared only within one process, so the
/// hasher choice carries no persistence contract.
fn mesh_fingerprint(mesh: &Mesh) -> u64 {
    let mut hasher = DefaultHasher::new();
    mesh.vertices.len().hash(&mut hasher);
    for vertex in &mesh.vertices {
        vertex.position.map(f32::to_bits).hash(&mut hasher);
    }
    mesh.indices.hash(&mut hasher);
    hasher.finish()
}

/// Exact identity of the light a published direct cache was computed for. The
/// volume keys on a normalized direction and intensity, so comparing the
/// caller's values is conservative: a sub-epsilon change republishes.
fn sun_key(sun: Sun) -> [u32; 4] {
    [
        sun.direction_to_sun[0].to_bits(),
        sun.direction_to_sun[1].to_bits(),
        sun.direction_to_sun[2].to_bits(),
        sun.intensity.to_bits(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use matterweave_render::indirect::MAX_FACE_SLOTS;
    use matterweave_render::reflection::MAX_REFLECTION_AXIS;

    /// Fixture sun: straight up, so the floor plate is directly sunlit.
    fn sun() -> Sun {
        Sun {
            direction_to_sun: [0.0, 1.0, 0.0],
            intensity: 1.0,
        }
    }

    /// Empty authoritative world: the production wetland shape, where every
    /// visible object is mesh-only and no unit voxel exists.
    fn empty_world() -> World {
        World::new(20_260_913)
    }

    /// One unit cube exactly as the engine's own voxel mesher emits it, painted
    /// with `material`'s drawn colour: the shape one detail cell contributes.
    fn cube(material: u8) -> Mesh {
        let mut voxel = World::new(0);
        voxel.set([0, 0, 0], 1);
        let mut mesh = voxel.mesh();
        for vertex in &mut mesh.vertices {
            vertex.color = material_color(material);
        }
        mesh
    }

    /// A half-metre cube: small enough that its triangles can move without
    /// leaving the cell they already occupy.
    fn pebble(material: u8) -> Mesh {
        let mut mesh = cube(material);
        for vertex in &mut mesh.vertices {
            vertex.position = vertex.position.map(|value| value * 0.5);
        }
        mesh
    }

    /// A half-metre physics body with every vertex painted `color`, placed so
    /// its local origin sits at `translation` in world space.
    fn body_at(translation: [f32; 3], color: [f32; 3]) -> Mesh {
        let mut mesh = pebble(material::MOSS_TURF);
        for vertex in &mut mesh.vertices {
            vertex.color = color;
            vertex.position = [
                vertex.position[0] + translation[0],
                vertex.position[1] + translation[1],
                vertex.position[2] + translation[2],
            ];
        }
        mesh
    }

    fn placed(prototype: usize, translation: [f32; 3]) -> StaticInstance {
        StaticInstance {
            prototype,
            translation,
            yaw_quarters: 0,
        }
    }

    /// The receiver cell of [`Fixture`] and the +X face sample the proxy-only
    /// coverage test reads.
    const RECEIVER: [i32; 3] = [0, 0, 0];
    const WATER_CELL: [i32; 3] = [3, -1, 0];
    const RECEIVER_FACE_X: usize = 0;

    /// A proxy-only scene inside the coverage box: a `BANK_STONE` floor plate at
    /// cell level y = -1 with one `WATER` cell, and a `MOSS_TURF` receiver on
    /// top of it. Every visible triangle belongs to a mesh, not to the world.
    struct Fixture {
        world: World,
        meshes: Vec<Mesh>,
        instances: Vec<StaticInstance>,
        dynamic: Mesh,
        sun: Sun,
    }

    impl Fixture {
        fn new() -> Self {
            let mut instances = Vec::new();
            for x in -2..2 {
                for z in -2..2 {
                    if [x, -1, z] == WATER_CELL {
                        continue;
                    }
                    instances.push(placed(0, [x as f32, -1.0, z as f32]));
                }
            }
            instances.push(placed(
                2,
                [WATER_CELL[0] as f32, -1.0, WATER_CELL[2] as f32],
            ));
            instances.push(placed(1, [0.0, 0.0, 0.0]));
            Self {
                world: empty_world(),
                meshes: vec![
                    cube(material::BANK_STONE),
                    cube(material::MOSS_TURF),
                    cube(material::WATER),
                ],
                instances,
                dynamic: Mesh::default(),
                sun: sun(),
            }
        }

        fn source(&self) -> FrameSource<'_> {
            FrameSource {
                installed: &self.instances,
                meshes: &self.meshes,
                dynamic: &self.dynamic,
                world: &self.world,
                sun: self.sun,
            }
        }
    }

    /// Records publication traffic and re-checks the renderer's own acceptance
    /// conditions, so a scheduler that publishes a partial or stale
    /// representation fails without a GPU.
    struct FakeSink {
        indirect: bool,
        reflection: bool,
        indirect_calls: usize,
        reflection_calls: usize,
        withdraw_calls: usize,
        fail_indirect: bool,
        fail_reflection: bool,
        /// A representation digest the renderer has already retired. Publishing
        /// it again is a state-machine violation: the engine geometry installs
        /// reject the superseded proxy.
        retired_digest: Option<u64>,
        epoch: u64,
        sun: Sun,
        violations: Vec<String>,
    }

    impl FakeSink {
        fn new(sun: Sun) -> Self {
            Self {
                indirect: false,
                reflection: false,
                indirect_calls: 0,
                reflection_calls: 0,
                withdraw_calls: 0,
                fail_indirect: false,
                fail_reflection: false,
                retired_digest: None,
                epoch: SOURCE_EPOCH,
                sun,
                violations: Vec::new(),
            }
        }

        /// What the renderer's own geometry installs and dynamic uploads do.
        fn renderer_install(&mut self) {
            self.indirect = false;
            self.reflection = false;
        }
    }

    impl Publication for FakeSink {
        fn publish_indirect(
            &mut self,
            volume: &IndirectVolume,
            world: &World,
            source_epoch: u64,
            mesh_digest: Option<u64>,
        ) -> Result<(), String> {
            self.indirect_calls += 1;
            if source_epoch != self.epoch {
                self.violations
                    .push("published under a foreign epoch".into());
            }
            if !volume.complete() {
                self.violations.push("published a partial volume".into());
            }
            if !volume.valid_for(world, source_epoch, self.sun) {
                self.violations.push("published a stale volume".into());
            }
            if mesh_digest.is_none() || volume.mesh_digest() != mesh_digest {
                self.violations
                    .push("published without a matching proxy".into());
            }
            if mesh_digest.is_some() && mesh_digest == self.retired_digest {
                self.violations
                    .push("republished a retired representation".into());
            }
            if self.fail_indirect {
                return Err("forced indirect failure".into());
            }
            self.indirect = true;
            Ok(())
        }

        fn publish_reflection(
            &mut self,
            volume: &ReflectionVolume,
            world: &World,
            source_epoch: u64,
            mesh_digest: Option<u64>,
        ) -> Result<usize, String> {
            self.reflection_calls += 1;
            if !volume.valid_for_scene(world, source_epoch, mesh_digest) {
                self.violations
                    .push("published a stale reflection pack".into());
            }
            if mesh_digest.is_some() && mesh_digest == self.retired_digest {
                self.violations
                    .push("republished a retired representation".into());
            }
            if self.fail_reflection {
                return Err("forced reflection failure".into());
            }
            self.reflection = true;
            Ok(16_384)
        }

        fn withdraw_lighting(&mut self) {
            self.withdraw_calls += 1;
            self.indirect = false;
            self.reflection = false;
        }

        fn indirect_live(&self) -> bool {
            self.indirect
        }

        fn reflection_live(&self) -> bool {
            self.reflection
        }
    }

    /// Frames a fixture may need: the fixed budget advances at most 1024 rays
    /// and 8192 face inspections per frame over a 4000-cell box.
    const CONVERGE_FRAMES: usize = 400;

    fn settle(
        lighting: &mut WetlandLighting,
        sink: &mut FakeSink,
        source: &FrameSource<'_>,
        first: InstallState,
    ) -> Summary {
        let mut summary = lighting
            .update(sink, source, first)
            .expect("lighting frame");
        for _ in 0..CONVERGE_FRAMES {
            if summary.indirect_live && summary.reflection_live {
                return summary;
            }
            summary = lighting
                .update(sink, source, InstallState::Current)
                .expect("lighting frame");
        }
        panic!("proxy lighting did not converge in {CONVERGE_FRAMES} frames: {summary:?}");
    }

    #[test]
    fn publication_follows_a_complete_volume_and_an_accepted_install() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);

        // Nothing is built or published while the renderer does not hold the
        // selection the proxy would describe.
        let summary = lighting
            .update(&mut sink, &source, InstallState::Unavailable)
            .expect("unavailable install is a session state, not an error");
        assert!(!summary.indirect_live && !summary.reflection_live);
        assert_eq!(sink.indirect_calls, 0);
        assert_eq!(sink.reflection_calls, 0);

        // The first accepted install builds the proxy. The mirror grid is a
        // bake, so it publishes immediately; the indirect volume must not
        // publish until it is complete.
        let summary = lighting
            .update(&mut sink, &source, InstallState::Installed)
            .expect("first install");
        assert!(summary.proxy_rebuilt);
        assert!(summary.proxy_cells > 0);
        assert!(summary.digest.is_some());
        assert!(summary.pending_work > 0);
        assert!(!summary.indirect_live);
        assert!(summary.reflection_live);
        assert_eq!(sink.indirect_calls, 0);
        assert_eq!(sink.reflection_calls, 1);

        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Current);
        assert!(summary.indirect_live && summary.reflection_live);
        assert_eq!(summary.pending_work, 0);
        assert_eq!(sink.indirect_calls, 1);
        assert_eq!(sink.reflection_calls, 1);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_rejected_install_leaves_the_last_accepted_publication_in_place() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let (rebuilds, withdraws) = (lighting.proxy_rebuilds, sink.withdraw_calls);

        // A rejected install retains the previous resident scene, so the
        // publication that describes it stays valid and is left alone.
        for _ in 0..3 {
            let summary = lighting
                .update(&mut sink, &source, InstallState::Unavailable)
                .expect("a rejected install is a session state, not an error");
            assert!(summary.indirect_live && summary.reflection_live);
            assert!(!summary.proxy_rebuilt);
        }
        assert_eq!(lighting.proxy_rebuilds, rebuilds);
        assert_eq!(sink.withdraw_calls, withdraws);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_renderer_install_reuses_the_cached_representation() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let (rebuilds, withdraws) = (lighting.proxy_rebuilds, sink.withdraw_calls);

        // The renderer retires both publications in every frame that accepts
        // geometry; the reinstall then reports the same selection and the same
        // proxy footprint.
        sink.renderer_install();
        let summary = lighting
            .update(&mut sink, &source, InstallState::Installed)
            .expect("reinstall");
        assert!(
            summary.proxy_rebuilt,
            "an accepted install rebuilds the proxy"
        );
        assert_eq!(lighting.proxy_rebuilds, rebuilds + 1);
        assert_eq!(
            sink.withdraw_calls, withdraws,
            "an identical footprint keeps the cached volume"
        );
        assert!(summary.indirect_live && summary.reflection_live);
        assert_eq!(summary.pending_work, 0, "republication must not recompute");
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn an_unchanged_state_rebuilds_nothing_and_republishes_nothing() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let (rebuilds, calls, withdraws) = (
            lighting.proxy_rebuilds,
            sink.indirect_calls,
            sink.withdraw_calls,
        );

        for _ in 0..3 {
            let summary = lighting
                .update(&mut sink, &source, InstallState::Current)
                .expect("idle frame");
            assert!(!summary.proxy_rebuilt);
            assert!(summary.indirect_live && summary.reflection_live);
        }
        assert_eq!(lighting.proxy_rebuilds, rebuilds);
        assert_eq!(sink.indirect_calls, calls, "no redundant uploads");
        assert_eq!(sink.withdraw_calls, withdraws);
    }

    #[test]
    fn the_proxy_footprint_gates_invalidation() {
        let mut fixture = Fixture::new();
        fixture.meshes = vec![pebble(material::MOSS_TURF)];
        fixture.instances = vec![placed(0, [1.2, 0.2, 1.2])];
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let digest = summary.digest.expect("digest");

        // A sub-cell move keeps every triangle inside the cell it already
        // occupied: the representation is identical and the cached output for
        // it stays valid.
        let nudged = vec![placed(0, [1.45, 0.2, 1.45])];
        let nudged_source = FrameSource {
            installed: &nudged,
            ..source
        };
        let withdraws = sink.withdraw_calls;
        let summary = lighting
            .update(&mut sink, &nudged_source, InstallState::Installed)
            .expect("sub-cell move");
        assert_eq!(summary.digest, Some(digest));
        assert!(summary.indirect_live && summary.reflection_live);
        assert_eq!(sink.withdraw_calls, withdraws);

        // A move that crosses a cell boundary changes the footprint: the
        // publication that describes the previous representation is retired.
        // Exact dependency retention recomputes only the faces the move can
        // affect, and this fixture's invalidated set (6 faces, measured
        // 2026-09-13: `indirect_live = true`, `dirty_faces = 0`,
        // `pending_work = 0`, one new upload after the retirement) drains
        // inside one `UPDATE_BUDGET`, so the volume reconverges and publishes a
        // complete, current volume within the same frame. The assertion below
        // is unconditional: a retirement that left the volume dark would fail
        // it rather than be accepted.
        let moved = vec![placed(0, [3.2, 0.2, 3.2])];
        let moved_source = FrameSource {
            installed: &moved,
            ..source
        };
        let calls = sink.indirect_calls;
        let summary = lighting
            .update(&mut sink, &moved_source, InstallState::Installed)
            .expect("cell move");
        assert_ne!(summary.digest, Some(digest));
        assert!(
            sink.withdraw_calls > withdraws,
            "a changed representation must retire the previous publication"
        );
        assert!(
            summary.invalidated_faces > 0,
            "the moved object's own faces must be invalidated: {summary:?}"
        );
        assert!(
            summary.indirect_live,
            "the invalidated set must drain inside one budget and republish in the same frame: \
             {summary:?}"
        );
        assert!(
            sink.indirect_calls > calls,
            "a live indirect publication must have been uploaded after the retirement"
        );
        assert_eq!(
            summary.pending_work, 0,
            "a live publication must be complete"
        );
        assert_eq!(
            summary.dirty_faces, 0,
            "the invalidated set must be fully drained, not partly recomputed: {summary:?}"
        );
        assert!(
            summary.reflection_live,
            "the mirror bake of the new representation is immediately valid"
        );

        let summary = settle(
            &mut lighting,
            &mut sink,
            &moved_source,
            InstallState::Current,
        );
        assert!(summary.indirect_live && summary.reflection_live);
        assert_ne!(summary.digest, Some(digest));
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_body_cell_move_retains_the_untouched_gi_faces() {
        let fixture = Fixture::new();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        // A physics body resting in the receiver's cell, drawn from the merged
        // dynamic mesh and represented through the proxy.
        let body = |x: f32| {
            let mut mesh = pebble(material::MOSS_TURF);
            for vertex in &mut mesh.vertices {
                vertex.position = [
                    vertex.position[0] + x,
                    vertex.position[1],
                    vertex.position[2] + 0.2,
                ];
            }
            mesh
        };
        let resting = body(0.2);
        let resting_source = FrameSource {
            dynamic: &resting,
            ..fixture.source()
        };
        settle(
            &mut lighting,
            &mut sink,
            &resting_source,
            InstallState::Installed,
        );

        // One cell in +X: the merged mesh crosses a cell boundary, so the proxy
        // footprint changes and exact dependency retention decides what is
        // recomputed instead of clearing the whole volume.
        let moved = body(1.2);
        let moved_source = FrameSource {
            dynamic: &moved,
            ..fixture.source()
        };
        let summary = lighting
            .update(&mut sink, &moved_source, InstallState::Current)
            .expect("body cell move");
        assert!(summary.proxy_rebuilt);
        assert!(
            summary.invalidated_faces > 0,
            "the move must invalidate the faces it can change: {summary:?}"
        );
        assert!(
            summary.retained_faces > summary.invalidated_faces,
            "a one-cell body move must retain most completed faces: {summary:?}"
        );
        // The invalidated set is small enough to recompute inside this frame's
        // `UPDATE_BUDGET`, so the withdrawn publication is replaced by a
        // complete, current one in the same step instead of staying dark.
        assert_eq!(summary.dirty_faces, 0, "the dirty set drains this frame");
        assert!(
            summary.indirect_live && summary.pending_work == 0,
            "a completed, current volume must be live again: {summary:?}"
        );
        assert!(
            summary.dependency_bytes > 0 && summary.dependency_bytes <= INDIRECT_DEPENDENCY_BYTES,
            "the tracker must be bounded by its cap: {summary:?}"
        );
        println!(
            "[wetland retention] cells={} retained={} invalidated={} dirty={} pending={} \
dependency_kib={} gi_live={}",
            summary.proxy_cells,
            summary.retained_faces,
            summary.invalidated_faces,
            summary.dirty_faces,
            summary.pending_work,
            summary.dependency_bytes / 1024,
            summary.indirect_live,
        );

        let summary = settle(
            &mut lighting,
            &mut sink,
            &moved_source,
            InstallState::Current,
        );
        assert!(summary.indirect_live && summary.reflection_live);
        assert_eq!(summary.pending_work, 0, "the retained volume must converge");
        assert_eq!(summary.dirty_faces, 0);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn proxy_only_coverage_carries_detail_into_gi_and_reflection() {
        let fixture = Fixture::new();
        let source = fixture.source();

        // Control: a proxy-less volume over the same empty world has nothing to
        // sample, so the fixture's response can only come from the proxy.
        let mut alone = IndirectVolume::new(
            box_origin([0.0, 0.0, 0.0]),
            BOX_DIMENSIONS,
            SAMPLES,
            GATHER_DISTANCE_M,
            palette(None),
        )
        .expect("control volume");
        for _ in 0..CONVERGE_FRAMES {
            if alone.complete() {
                break;
            }
            alone
                .update(&fixture.world, SOURCE_EPOCH, fixture.sun, UPDATE_BUDGET)
                .expect("control update");
        }
        assert!(alone.complete());
        for face in 0..6 {
            assert_eq!(
                alone.sample(RECEIVER, face).map(f32::to_bits),
                [0u32; 3],
                "a world with no voxels can light face {face}"
            );
        }

        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        assert!(summary.proxy_cells > 0);

        let volume = lighting.volume.as_ref().expect("attached volume");
        assert!(volume.has_mesh_proxy());
        let lit = volume.sample(RECEIVER, RECEIVER_FACE_X);
        assert!(
            lit.iter().any(|channel| *channel > 0.0),
            "the receiver face must gather the sunlit floor through the proxy: {lit:?}"
        );

        let pack = lighting.reflection.as_ref().expect("published reflection");
        assert_eq!(pack.material_at(RECEIVER), material::MOSS_TURF);
        assert_eq!(pack.material_at(WATER_CELL), material::WATER);
        assert_eq!(pack.material_at([-2, -1, -2]), material::BANK_STONE);
        assert!(pack.mirror_at(WATER_CELL) > 0.0);
        assert_eq!(pack.mirror_at(RECEIVER), 0.0);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_failed_publication_withdraws_and_the_next_frame_republishes() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let calls = sink.indirect_calls;

        // The renderer retired both publications and the upload failed: the
        // frame reports it once and leaves nothing stale behind.
        sink.renderer_install();
        sink.fail_indirect = true;
        let error = lighting
            .update(&mut sink, &source, InstallState::Current)
            .expect_err("a failed upload must be reported");
        assert!(error.contains("forced indirect failure"), "{error}");
        assert!(!sink.indirect_live() && !sink.reflection_live());

        // The identical failure is not reported twice, and the session keeps
        // getting frames.
        let summary = lighting
            .update(&mut sink, &source, InstallState::Current)
            .expect("the session continues after a failed publication");
        assert!(!summary.indirect_live && !summary.reflection_live);

        // With the transient gone, the still-complete cached volume republishes
        // without recomputing.
        sink.fail_indirect = false;
        let summary = lighting
            .update(&mut sink, &source, InstallState::Current)
            .expect("republication");
        assert!(summary.indirect_live && summary.reflection_live);
        assert_eq!(sink.indirect_calls, calls + 3);
        assert_eq!(summary.pending_work, 0, "republication must not recompute");
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_failed_proxy_build_is_contained_and_retried_on_the_next_install() {
        let mut fixture = Fixture::new();
        // A resident mesh with no detail palette colour cannot be described by
        // one proxy material.
        fixture.meshes = vec![{
            let mut mesh = cube(material::MOSS_TURF);
            for vertex in &mut mesh.vertices {
                vertex.color = [0.4, 0.4, 0.4];
            }
            mesh
        }];
        fixture.instances = vec![placed(0, [0.0, 0.0, 0.0])];
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);

        let error = lighting
            .update(&mut sink, &source, InstallState::Installed)
            .expect_err("an unrepresentable mesh must be reported");
        assert!(error.contains("palette colour"), "{error}");
        let rebuilds = lighting.proxy_rebuilds;
        // A repeated failure is not reported per frame, and it is not retried
        // until the source moves.
        let summary = lighting
            .update(&mut sink, &source, InstallState::Current)
            .expect("the session continues");
        assert!(!summary.indirect_live);
        assert_eq!(lighting.proxy_rebuilds, rebuilds);
        assert_eq!(sink.indirect_calls, 0);
    }

    #[test]
    fn a_sun_change_retires_the_cache_and_republishes_under_the_new_light() {
        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let (calls, withdraws) = (sink.indirect_calls, sink.withdraw_calls);

        let other = Sun {
            direction_to_sun: [0.3, 0.9, 0.2],
            intensity: 0.5,
        };
        let other_source = FrameSource {
            sun: other,
            ..source
        };
        sink.sun = other;
        let summary = lighting
            .update(&mut sink, &other_source, InstallState::Current)
            .expect("sun change");
        assert!(
            !summary.indirect_live,
            "a cache computed for another light is not current"
        );
        assert!(sink.withdraw_calls > withdraws);

        let summary = settle(
            &mut lighting,
            &mut sink,
            &other_source,
            InstallState::Current,
        );
        assert!(summary.indirect_live && summary.reflection_live);
        assert!(sink.indirect_calls > calls);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn the_coverage_box_bounds_the_proxy_and_respects_the_engine_caps() {
        let cells: u32 = BOX_DIMENSIONS.iter().product();
        assert_eq!(cells, 4000);
        assert!(cells as usize * 6 <= MAX_FACE_SLOTS);
        assert!(BOX_DIMENSIONS
            .iter()
            .all(|&axis| axis <= MAX_REFLECTION_AXIS));
        // Symmetric around the anchor, on the world's cell grid.
        assert_eq!(box_origin([92.0, 18.2, 58.0]), [82, 13, 48]);

        let fixture = Fixture::new();
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let (digest, pool_meshes) = (summary.digest, summary.proxy_pool_meshes);
        assert_eq!(pool_meshes, 3, "floor, receiver and water");

        // An instance far outside the box is filtered out, so it neither joins
        // the box-local pool nor changes the representation.
        let mut far = fixture.instances.clone();
        far.push(placed(2, [900.0, 0.0, 900.0]));
        let far_source = FrameSource {
            installed: &far,
            ..source
        };
        let summary = lighting
            .update(&mut sink, &far_source, InstallState::Installed)
            .expect("far instance");
        assert_eq!(summary.digest, digest);
        assert_eq!(summary.proxy_pool_meshes, pool_meshes);
        assert!(summary.indirect_live && summary.reflection_live);
    }

    #[test]
    fn the_catalogue_palette_is_complete_injective_and_dominates_drawn_colour() {
        let known: Vec<u8> = (1..=254)
            .filter(|&id| matterweave_detail::material_name(id) != "unknown")
            .collect();
        assert_eq!(
            known,
            CATALOGUE_MATERIALS.to_vec(),
            "the catalogue list must follow the detail palette"
        );
        let mut colors: Vec<[u32; 3]> = CATALOGUE_MATERIALS
            .iter()
            .map(|&id| material_color(id).map(f32::to_bits))
            .collect();
        colors.sort_unstable();
        let distinct = {
            let mut unique = colors.clone();
            unique.dedup();
            unique.len()
        };
        assert_eq!(
            distinct,
            colors.len(),
            "the reverse lookup needs distinct colours"
        );

        assert_eq!(
            dominant_material(&cube(material::BANK_STONE)),
            Some(material::BANK_STONE)
        );
        let mut mixed = cube(material::WATER);
        for (index, vertex) in mixed.vertices.iter_mut().enumerate() {
            vertex.color = if index < 16 {
                material_color(material::WATER)
            } else {
                material_color(material::BANK_STONE)
            };
        }
        assert_eq!(dominant_material(&mixed), Some(material::WATER));
        let mut unknown = cube(material::WATER);
        for vertex in &mut unknown.vertices {
            vertex.color = [0.4, 0.4, 0.4];
        }
        assert_eq!(dominant_material(&unknown), None);
    }

    #[test]
    fn the_merged_dynamic_mesh_keeps_its_own_drawn_colour() {
        let fixture = Fixture::new();
        let dynamic = {
            let mut mesh = cube(material::BANK_STONE);
            for vertex in &mut mesh.vertices {
                vertex.color = [0.9, 0.43, 0.15];
            }
            mesh
        };
        let source = FrameSource {
            dynamic: &dynamic,
            ..fixture.source()
        };
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        assert!(summary.proxy_cells > 0);
        assert_eq!(
            lighting
                .palette_dynamic
                .map(|color| color.map(f32::to_bits)),
            Some([0.9f32.to_bits(), 0.43f32.to_bits(), 0.15f32.to_bits()])
        );
        let pack = lighting.reflection.as_ref().expect("published reflection");
        assert_eq!(
            pack.material_at(RECEIVER),
            DYNAMIC_MATERIAL,
            "the moving mesh owns a cell it shares with static detail"
        );
        let entry = pack.palette()[DYNAMIC_MATERIAL as usize];
        assert_eq!(
            [entry[0], entry[1], entry[2]].map(f32::to_bits),
            [0.9f32.to_bits(), 0.43f32.to_bits(), 0.15f32.to_bits()]
        );
        assert_eq!(pack.material_at(WATER_CELL), material::WATER);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_dynamic_mesh_change_rebuilds_the_proxy_and_retires_a_changed_footprint() {
        let fixture = Fixture::new();
        let empty = Mesh::default();
        let source = FrameSource {
            dynamic: &empty,
            ..fixture.source()
        };
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let digest = summary.digest;

        // A body appears outside the static detail: the merged dynamic mesh
        // moves, so the proxy is rebuilt from it and the changed footprint
        // retires the cached publication.
        let mut body = pebble(material::MOSS_TURF);
        for vertex in &mut body.vertices {
            vertex.position = [
                vertex.position[0] + 5.2,
                vertex.position[1] + 0.2,
                vertex.position[2] + 5.2,
            ];
        }
        let body_source = FrameSource {
            dynamic: &body,
            ..fixture.source()
        };
        let rebuilds = lighting.proxy_rebuilds;
        let withdraws = sink.withdraw_calls;
        let summary = lighting
            .update(&mut sink, &body_source, InstallState::Current)
            .expect("body appears");
        assert!(summary.proxy_rebuilt);
        assert_eq!(lighting.proxy_rebuilds, rebuilds + 1);
        assert_ne!(summary.digest, digest);
        assert!(
            !summary.indirect_live,
            "a changed footprint retires the cached GI"
        );
        assert!(sink.withdraw_calls > withdraws);

        let summary = settle(
            &mut lighting,
            &mut sink,
            &body_source,
            InstallState::Current,
        );
        assert!(summary.indirect_live && summary.reflection_live);
        let body_digest = summary.digest;
        assert_eq!(
            lighting
                .reflection
                .as_ref()
                .expect("published reflection")
                .material_at([5, 0, 5]),
            DYNAMIC_MATERIAL
        );

        // Motion that keeps the merged mesh inside the cell it already occupies
        // changes no representation, so the publication survives the rebuild.
        let mut still = pebble(material::MOSS_TURF);
        for vertex in &mut still.vertices {
            vertex.position = [
                vertex.position[0] + 5.45,
                vertex.position[1] + 0.2,
                vertex.position[2] + 5.45,
            ];
        }
        let nudged = FrameSource {
            dynamic: &still,
            ..fixture.source()
        };
        let withdraws = sink.withdraw_calls;
        let summary = lighting
            .update(&mut sink, &nudged, InstallState::Current)
            .expect("sub-cell body motion");
        assert!(
            summary.proxy_rebuilt,
            "a moving mesh is rebuilt from the drawn geometry"
        );
        assert_eq!(summary.digest, body_digest);
        assert_eq!(sink.withdraw_calls, withdraws);
        assert!(summary.indirect_live && summary.reflection_live);
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn an_attach_allocation_failure_never_republishes_the_superseded_proxy() {
        let fixture = Fixture::new();
        let body = body_at([1.2, 0.2, 1.2], [0.9, 0.43, 0.15]);
        let source = FrameSource {
            dynamic: &body,
            ..fixture.source()
        };
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let digest = summary.digest.expect("digest");
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);

        // The body crosses a cell boundary and changes colour, so the accepted
        // install needs both a new footprint and a new volume palette, and the
        // volume allocation fails.
        let moved = body_at([3.2, 0.2, 3.2], [0.2, 0.6, 0.9]);
        let moved_source = FrameSource {
            dynamic: &moved,
            ..fixture.source()
        };
        sink.retired_digest = Some(digest);
        lighting.armed_attach_fault = Some(AttachFault::Volume);
        let error = lighting
            .update(&mut sink, &moved_source, InstallState::Installed)
            .expect_err("a failed attach must be reported");
        assert!(
            error.contains("injected indirect volume allocation failure"),
            "{error}"
        );
        assert!(!sink.indirect_live() && !sink.reflection_live());
        let calls = sink.indirect_calls;

        // The renderer already holds the new geometry, so the superseded
        // representation is gone: the next frame may not publish it, and the
        // failed attach is not retried every frame.
        let summary = lighting
            .update(&mut sink, &moved_source, InstallState::Current)
            .expect("the session continues after a failed attach");
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
        assert_eq!(
            sink.indirect_calls, calls,
            "retired geometry must not republish"
        );
        assert!(!summary.indirect_live && !summary.reflection_live);
        assert_eq!(summary.digest, None, "no representation stays attached");

        // The transient clears; the next accepted install builds and publishes
        // the representation the renderer now holds.
        lighting.armed_attach_fault = None;
        let summary = settle(
            &mut lighting,
            &mut sink,
            &moved_source,
            InstallState::Installed,
        );
        assert!(summary.indirect_live && summary.reflection_live);
        assert_ne!(summary.digest, Some(digest));
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn an_attach_pack_failure_never_republishes_the_superseded_proxy() {
        let mut fixture = Fixture::new();
        fixture.meshes = vec![pebble(material::MOSS_TURF)];
        fixture.instances = vec![placed(0, [1.2, 0.2, 1.2])];
        let source = fixture.source();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Installed);
        let digest = summary.digest.expect("digest");

        // The pebble crosses a cell boundary: the footprint change retires the
        // cached publication before the mirror bake runs, and the bake fails.
        let moved = vec![placed(0, [3.2, 0.2, 3.2])];
        let moved_source = FrameSource {
            installed: &moved,
            ..source
        };
        sink.retired_digest = Some(digest);
        lighting.armed_attach_fault = Some(AttachFault::Reflection);
        let error = lighting
            .update(&mut sink, &moved_source, InstallState::Installed)
            .expect_err("a failed attach must be reported");
        assert!(
            error.contains("injected reflection pack failure"),
            "{error}"
        );
        assert!(!sink.indirect_live() && !sink.reflection_live());
        let calls = sink.indirect_calls;

        let summary = lighting
            .update(&mut sink, &moved_source, InstallState::Current)
            .expect("the session continues after a failed attach");
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
        assert_eq!(
            sink.indirect_calls, calls,
            "retired geometry must not republish"
        );
        assert!(!summary.indirect_live && !summary.reflection_live);
        assert_eq!(summary.digest, None, "no representation stays attached");

        lighting.armed_attach_fault = None;
        let summary = settle(
            &mut lighting,
            &mut sink,
            &moved_source,
            InstallState::Installed,
        );
        assert!(summary.indirect_live && summary.reflection_live);
        assert_ne!(summary.digest, Some(digest));
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }

    #[test]
    fn a_stale_pack_with_an_unchanged_digest_repacks_without_perpetual_rebuilds() {
        let mut fixture = Fixture::new();
        let mut lighting = WetlandLighting::new([0.0, 0.0, 0.0]);
        let mut sink = FakeSink::new(fixture.sun);
        let digest = {
            let source = fixture.source();
            settle(&mut lighting, &mut sink, &source, InstallState::Installed)
                .digest
                .expect("digest")
        };
        let rebuilds = lighting.proxy_rebuilds;

        // A future editable world: the authoritative revision moves without
        // changing the proxy footprint, and the renderer retires both
        // publications. The retained pack is stale for the same digest.
        fixture.world.set([64, 0, 64], 1);
        sink.renderer_install();
        let source = fixture.source();
        let summary = lighting
            .update(&mut sink, &source, InstallState::Current)
            .expect("world-edit frame");
        assert_eq!(summary.digest, Some(digest));
        assert_eq!(
            lighting.proxy_rebuilds, rebuilds,
            "observing the staleness does not rebuild by itself"
        );

        // The stale pack is repacked by exactly one rebuild, and the repacked
        // bake is valid for the new revision immediately.
        let summary = lighting
            .update(&mut sink, &source, InstallState::Current)
            .expect("repack frame");
        assert_eq!(lighting.proxy_rebuilds, rebuilds + 1);
        assert!(
            summary.reflection_live,
            "the repacked bake must be valid for the current source"
        );

        // GI reconverges under the new revision without any further proxy
        // rebuild: the stale flag cleared with the repack.
        let summary = settle(&mut lighting, &mut sink, &source, InstallState::Current);
        assert!(summary.indirect_live && summary.reflection_live);
        assert_eq!(
            lighting.proxy_rebuilds,
            rebuilds + 1,
            "a stale unchanged digest must clear without rebuilding every frame"
        );
        assert!(sink.violations.is_empty(), "{:?}", sink.violations);
    }
}
