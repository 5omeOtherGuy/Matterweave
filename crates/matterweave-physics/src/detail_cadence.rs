//! Explicit edit-to-collision cadence for editable detail worlds.
//!
//! This wraps [`AsyncDetailCollision`] with the publication policy a
//! simulation loop follows frame by frame, so hosts cannot accidentally
//! publish a superseded result, publish new collision through a body, or lose
//! collision entirely when the worker is gone. It is the integration seam
//! between "the player just edited the authoritative [`DetailScene`]" and
//! "the live [`Physics`] world is safe to step against".
//!
//! # Documented queue / publication semantics
//!
//! - **World semantics**: the live [`Physics`] world — movement, queries and
//!   [`Physics::detail_collision_stats`] — always reflects exactly the *last
//!   accepted publication*. The authoritative scene is mutated by the owner
//!   immediately; collision preparation is only *queued* from an immutable
//!   snapshot. Live collision therefore never has a hole and never silently
//!   loses a wall: an edit that removes a wall keeps the old collider blocking
//!   until the newer shape publishes, which is why movement stays correct.
//! - **Visual semantics**: the owner's render mesh is the edited scene (it can
//!   be updated immediately, as the explorer does). While a publication is
//!   pending, a newly added wall is therefore visible but not yet solid, and a
//!   removed wall is solid but not yet visible. That window lasts through
//!   preparation and any overlap deferral; it has no fixed time bound. The
//!   publication gate below prevents adding solid material through a body.
//! - **Publication gate**: every [`on_edit`](DetailCollisionCadence::on_edit)
//!   call names the world-space regions where it added solid collision
//!   material (`None` when the change is not expressible as added cells, e.g.
//!   added/removed/moved instances). A prepared result publishes only when no
//!   dynamic body AABB overlaps any accumulated added region
//!   ([`Physics::detail_added_blocked`]). When a body has advanced into the
//!   region of a pending new wall, the result is *retained* and retried on
//!   later frames; it is never dropped and never placed through the body. The
//!   wall becomes solid only once the body has left the region. Removals never gate,
//!   so the common "remove a wall while walking toward it" edit still
//!   publishes as soon as preparation completes. Regions accumulate across
//!   rapid edits and clear on every accepted publication, so the gate always
//!   covers the full diff against the last accepted publication. A structural
//!   (`None`) change latches a conservative mode that defers while any body
//!   overlaps any prepared collider; it clears on publication. The region list
//!   is bounded by [`MAX_PENDING_ADDED`]; overflow latches the same
//!   conservative mode.
//! - Why regions, not collider comparison: a whole-collider AABB equality
//!   check cannot establish unchanged shape — filling an interior hole leaves
//!   the outer AABB identical while new solid material appears inside it. The
//!   journal-sourced added region covers exactly that case at cell
//!   granularity, including AABB overlap (not shape contact), which is the
//!   sound direction: publication may defer spuriously, but never through a
//!   body.
//! - [`DetailCollisionCadence::step`] publishes **at most one** result per
//!   call. This is structural, not a convention: the controller buffers at
//!   most one completed result (plus at most one staged synchronous result),
//!   so one `step` can never publish two. The once-*per-frame* part is the
//!   caller's convention — call `step` once on the simulation thread before
//!   stepping, never concurrently with stepping.
//! - Every result is tagged with the scene's opaque
//!   [`SceneVersion`](matterweave_detail::SceneVersion). A superseded result (a
//!   newer edit arrived), a reset-cancelled result, or a result built before a
//!   scene replacement never publishes; the controller drops or retains them
//!   per its documented bounds and `step` simply finds nothing.
//! - If preparation of the **current** scene fails (deterministic budget
//!   rejection), `step` returns the error with live collision untouched and
//!   the accumulated gate region cleared. The owner must restore consistency:
//!   revert the offending edits against the authoritative scene (restoring the
//!   exact prior save-journal entries, not just scene materials) and queue
//!   that reverted source again. The reverted source matches the still-live
//!   collision, so movement stays correct the whole time.
//! - If the worker is unavailable (startup failure or exit), `on_edit` builds
//!   the preparation **synchronously on the calling thread** and applies the
//!   *same gate*: a clear source publishes immediately (`Ok(false)`); a
//!   blocked source is staged in a single bounded slot and publishes from a
//!   later `step` once the body clears (`Ok(true)`). There is no ungated
//!   synchronous path: the fallback runs during pending runtime edits, not
//!   just at load, so bypassing the check would publish new walls through
//!   bodies. Cost truthfully stated: the synchronous build is a full-scene
//!   rebuild on the simulation thread (same cost class as
//!   [`Physics::replace_detail_scene`]) plus at most one staged rebuild per
//!   new source version in `step`; use only when the worker is gone.
//! - `pending_edits` bookkeeping (the host's own revert journal) is bounded by
//!   coalescing: at most one entry per (instance, cell), the entry carrying
//!   the scene material *and* the save-journal entry the cell held at the last
//!   accepted publication, so rollback restores confirmed history instead of
//!   deleting it.

use crate::Physics;
use crate::{AsyncDetailCollision, AsyncDetailStats, DetailCollisionStats};
use matterweave_detail::{DetailScene, SceneVersion};
use rapier3d::prelude::*;

/// Largest number of accumulated added-solid regions held between
/// publications. One entry per edited cell is the normal case; reaching the
/// cap latches the conservative structural gate instead of growing, so pending
/// state stays bounded at ~[`MAX_PENDING_ADDED`] × 24 B.
pub const MAX_PENDING_ADDED: usize = 4096;

/// Edit-to-collision cadence controller for one live scene/physics pair.
///
/// Owner semantics: single simulation thread, `step` called once per frame
/// before physics stepping, `on_edit` called after each authoritative scene
/// edit, `reset` on scene replacement or load.
pub struct DetailCollisionCadence {
    controller: AsyncDetailCollision,
    /// Source version of the last accepted publication (any path). While the
    /// worker is unavailable, a source version other than this one is
    /// republished synchronously once, instead of pending forever.
    published_version: Option<SceneVersion>,
    /// World-space regions where unconfirmed edits added solid collision
    /// material, relative to the last accepted publication. Cleared on every
    /// accepted publication and on preparation failure (the owner reverts the
    /// burst, restoring the empty diff).
    added: Vec<Aabb>,
    /// Latched by a structural/unknown change or region overflow: gate
    /// conservatively against every prepared collider until publication.
    structural: bool,
    /// Synchronously built preparation retained by the gate while the worker
    /// is unavailable. At most one: a newer staged source replaces it.
    staged: Option<PreparedSync>,
}

struct PreparedSync {
    prepared: crate::PreparedDetailCollision,
    version: SceneVersion,
}

impl Default for DetailCollisionCadence {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailCollisionCadence {
    /// Starts the background worker behind the controller.
    pub fn new() -> Self {
        Self {
            controller: AsyncDetailCollision::new(),
            published_version: None,
            added: Vec::new(),
            structural: false,
            staged: None,
        }
    }

    /// Fallback-path constructor: behaves exactly as if the worker failed to
    /// start, so tests and hosts without threads exercise the synchronous
    /// staged path. Every other semantic is unchanged.
    pub fn without_worker() -> Self {
        Self {
            controller: AsyncDetailCollision::without_worker(),
            published_version: None,
            added: Vec::new(),
            structural: false,
            staged: None,
        }
    }

    /// Queues preparation of `scene` after an authoritative edit.
    ///
    /// `added` names the world-space `(min, max)` boxes where this edit added
    /// solid collision material (cells whose material newly maps to
    /// [`MaterialPolicy::Collision`](matterweave_detail::MaterialPolicy));
    /// pass an empty slice for edits that removed solid material or changed
    /// nothing solid, and `None` for changes not expressible as added cells
    /// (added/removed/moved instances), which latches the conservative
    /// structural gate until the next publication.
    ///
    /// Returns `Ok(true)` when preparation is pending (asynchronous or staged
    /// synchronous), `Ok(false)` when the source published synchronously
    /// (worker unavailable and the gate clear; collision is already current),
    /// and `Err(_)` when preparation rejected the source; the caller must
    /// then revert the edit. Live collision is unchanged on `Err`, and the
    /// accumulated gate region is rolled back to its entry state.
    pub fn on_edit(
        &mut self,
        scene: &DetailScene,
        physics: &mut Physics,
        added: Option<&[([f32; 3], [f32; 3])]>,
    ) -> Result<bool, String> {
        let baseline = self.added.len();
        let structural_baseline = self.structural;
        match added {
            Some(regions) => {
                if regions.len() > MAX_PENDING_ADDED - self.added.len() {
                    self.structural = true;
                } else {
                    self.added.extend(
                        regions
                            .iter()
                            .map(|(lo, hi)| Aabb::new((*lo).into(), (*hi).into())),
                    );
                }
            }
            None => {
                // Keep the prior bounded regions until this request succeeds:
                // a synchronous rejection must restore their exact gate.
                self.structural = true;
            }
        }
        if self.controller.request(scene) {
            return Ok(true);
        }
        if self.controller.available() {
            let stats = self.controller.stats();
            if stats.queued > 0 || stats.inflight > 0 || stats.results > 0 {
                // The request was deduplicated against work already tracked
                // for this source; publication follows in `step`.
                return Ok(true);
            }
        }
        // No worker, or no tracked work despite a refusal (worker exited
        // between the checks): build synchronously and apply the same gate.
        // An ungated publish here would place a new wall through a body that
        // moved while earlier edits were pending, exactly the hazard the
        // asynchronous gate exists for.
        let version = scene.source_version();
        let prepared = match crate::PreparedDetailCollision::build(scene) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.added.truncate(baseline);
                self.structural = structural_baseline;
                return Err(error);
            }
        };
        if gate_blocked(self.structural, &self.added, physics, Some(&prepared)) {
            self.staged = Some(PreparedSync { prepared, version });
            return Ok(true);
        }
        let _stats = physics.publish_detail_scene(scene, prepared)?;
        self.published_version = Some(version);
        self.added.clear();
        self.structural = false;
        self.staged = None;
        Ok(false)
    }

    /// Per-frame publication. Polls the controller once and, if a result
    /// current for `scene` is buffered *and* the publication gate is clear,
    /// publishes it. Returns `Ok(Some)` with the accepted publication stats,
    /// `Ok(None)` when nothing was published this frame — including a result
    /// retained by the publication gate because a dynamic body overlaps an
    /// added-solid region — and `Err(_)` when preparation of the current
    /// scene failed (live collision preserved; the gate region is cleared and
    /// the owner reverts the burst).
    pub fn step(
        &mut self,
        scene: &DetailScene,
        physics: &mut Physics,
    ) -> Result<Option<DetailCollisionStats>, String> {
        // Snapshot the gate inputs: the poll closure cannot borrow `self`
        // while `self.controller` is borrowed mutably for the call.
        let structural = self.structural;
        let added = &self.added;
        if !self.controller.available() {
            // Worker gone: a buffered result from before the exit still
            // publishes first (gated); otherwise the current source is built
            // synchronously, at most one build per new source version, and
            // gated the same way.
            if let Some(prepared) = self.controller.poll_retaining(scene, |outcome| {
                outcome.is_err() || !gate_blocked(structural, added, physics, outcome.as_ref().ok())
            }) {
                return self.publish_prepared(scene, physics, prepared);
            }
            let version = scene.source_version();
            if let Some(staged) = &self.staged {
                if staged.version == version {
                    if gate_blocked(structural, added, physics, Some(&staged.prepared)) {
                        return Ok(None);
                    }
                    let staged = self.staged.take().expect("staged present");
                    let stats = physics.publish_detail_scene(scene, staged.prepared)?;
                    self.published_version = Some(version);
                    self.added.clear();
                    self.structural = false;
                    return Ok(Some(stats));
                }
                self.staged = None;
            }
            if self.published_version.as_ref() == Some(&version) {
                return Ok(None);
            }
            let outcome = crate::PreparedDetailCollision::build(scene);
            match outcome {
                Err(error) => {
                    self.added.clear();
                    self.structural = false;
                    self.staged = None;
                    return Err(error);
                }
                Ok(prepared) => {
                    if gate_blocked(structural, added, physics, Some(&prepared)) {
                        self.staged = Some(PreparedSync { prepared, version });
                        return Ok(None);
                    }
                    let stats = physics.publish_detail_scene(scene, prepared)?;
                    self.published_version = Some(version);
                    self.added.clear();
                    self.structural = false;
                    return Ok(Some(stats));
                }
            }
        }
        let Some(prepared) = self
            .controller
            .poll_retaining(scene, |outcome| match outcome {
                // Gate: defer publication while a dynamic body overlaps an
                // added-solid region (or any prepared collider after a structural
                // change). Errors pass through.
                Ok(prepared) => !gate_blocked(structural, added, physics, Some(prepared)),
                Err(_) => true,
            })
        else {
            return Ok(None);
        };
        self.publish_prepared(scene, physics, prepared)
    }

    fn publish_prepared(
        &mut self,
        scene: &DetailScene,
        physics: &mut Physics,
        prepared: Result<crate::PreparedDetailCollision, String>,
    ) -> Result<Option<DetailCollisionStats>, String> {
        match prepared {
            Ok(prepared) => {
                let stats = physics.publish_detail_scene(scene, prepared)?;
                self.published_version = Some(scene.source_version());
                self.added.clear();
                self.structural = false;
                self.staged = None;
                Ok(Some(stats))
            }
            Err(error) => {
                self.added.clear();
                self.structural = false;
                self.staged = None;
                Err(error)
            }
        }
    }

    /// Scene replacement or load: cancels pending and buffered work and
    /// invalidates the job in flight, so nothing prepared for a retired scene
    /// can ever publish. Clears the gate region, the structural latch and any
    /// staged result. A fresh [`on_edit`](Self::on_edit) works immediately.
    /// The next unavailable-worker fallback republishes after a reset, too:
    /// [`Self::forget_publication`] clears the version tracked here.
    ///
    /// The replacement itself must be published synchronously by the owner
    /// with bodies pre-validated (the load path), or queued via
    /// `on_edit(.., None)` for the conservative structural gate: after a
    /// reset the cadence holds no region for the new scene, so an ungated
    /// publish would be unsound.
    pub fn reset(&mut self) {
        self.controller.reset();
        self.added.clear();
        self.structural = false;
        self.staged = None;
        self.forget_publication();
    }

    /// Clears the tracked last-published version, so an unavailable-worker
    /// fallback republishes the current source synchronously on the next
    /// `step` even if its version was published before a reset or reload.
    pub fn forget_publication(&mut self) {
        self.published_version = None;
    }

    /// False after worker startup failure or an unexpected worker exit; edits
    /// then take the synchronous staged fallback in [`on_edit`](Self::on_edit).
    pub fn available(&self) -> bool {
        self.controller.available()
    }

    /// Live queue counts for diagnostics; see [`AsyncDetailStats`].
    pub fn stats(&self) -> AsyncDetailStats {
        self.controller.stats()
    }
}

/// Whether the gate currently defers `prepared`: an added-solid overlap, or
/// any prepared-collider overlap after a structural change. A free function
/// so poll closures can snapshot the inputs without borrowing the cadence
/// while its controller is borrowed mutably.
fn gate_blocked(
    structural: bool,
    added: &[Aabb],
    physics: &Physics,
    prepared: Option<&crate::PreparedDetailCollision>,
) -> bool {
    let Some(prepared) = prepared else {
        return false;
    };
    if structural {
        let blockers = prepared.collider_aabbs();
        return physics
            .dynamic_body_aabbs()
            .iter()
            .any(|body| blockers.iter().any(|region| region.intersects(body)));
    }
    physics.detail_added_blocked(added)
}
