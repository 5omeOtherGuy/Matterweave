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
//!   removed wall is solid but not yet visible. That window is bounded by
//!   preparation time, and the publication gate below guarantees it can never
//!   trap a body.
//! - **Publication gate**: a prepared result publishes only when no dynamic
//!   body overlaps any collider it would *add* ([`Physics::detail_publication_blocked`]).
//!   When a body has advanced into the region of a pending new wall, the result
//!   is *retained* in the controller's buffer and retried on later frames; it
//!   is never dropped and never placed through the body. The wall appears only
//!   once the body has left the region. Removals and unchanged colliders never
//!   gate, so the common "remove a wall while walking toward it" edit still
//!   publishes as soon as preparation completes.
//! - [`DetailCollisionCadence::step`] publishes **at most one** result per
//!   call. This is structural, not a convention: the controller buffers at
//!   most one completed result, so one `step` can never publish two. The
//!   once-*per-frame* part is the caller's convention — call `step` once on
//!   the simulation thread before stepping, never concurrently with stepping.
//! - Every result is tagged with the scene's opaque
//!   [`SceneVersion`](matterweave_detail::SceneVersion). A superseded result (a
//!   newer edit arrived), a reset-cancelled result, or a result built before a
//!   scene replacement never publishes; the controller drops or retains them
//!   per its documented bounds and `step` simply finds nothing.
//! - If preparation of the **current** scene fails (deterministic budget
//!   rejection), `step` returns the error with live collision untouched. The
//!   owner must restore consistency: revert the offending edit against the
//!   authoritative scene and queue that reverted source again. The reverted
//!   source matches the still-live collision, so movement stays correct the
//!   whole time.
//! - If the worker is unavailable (startup failure or exit),
//!   [`DetailCollisionCadence::on_edit`] falls back to the synchronous
//!   [`Physics::replace_detail_scene`] path so collision never silently
//!   diverges from the authoritative scene, and `step` applies the same
//!   fallback for any source version that has not yet published. The race
//!   "worker dies right after an edit was queued" is bounded: the very next
//!   `step` observes `available() == false` and republishes that source
//!   synchronously (at most one rebuild per new source version, tracked by
//!   [`DetailCollisionCadence::published_version`]); pending work whose
//!   *current* result already buffered still publishes first.
//! - `pending_edits` bookkeeping (the host's own revert journal) is bounded by
//!   coalescing: at most one entry per (instance, cell), the entry carrying
//!   the material the cell held at the last accepted publication.

use crate::Physics;
use crate::{AsyncDetailCollision, AsyncDetailStats, DetailCollisionStats};
use matterweave_detail::{DetailScene, SceneVersion};

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
        }
    }

    /// Queues preparation of `scene` after an authoritative edit.
    ///
    /// Returns `Ok(true)` when preparation was queued asynchronously,
    /// `Ok(false)` when the worker was unavailable and the scene was instead
    /// published synchronously (collision is already current), and
    /// `Err(_)` when even the synchronous fallback rejected the source; the
    /// caller must then revert the edit. Live collision is unchanged on `Err`.
    pub fn on_edit(&mut self, scene: &DetailScene, physics: &mut Physics) -> Result<bool, String> {
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
        // between the checks): publish synchronously so collision cannot
        // silently lag the authoritative scene. This is the load-time path.
        let version = scene.source_version();
        physics.replace_detail_scene(scene)?;
        self.published_version = Some(version);
        Ok(false)
    }

    /// Per-frame publication. Polls the controller once and, if a result
    /// current for `scene` is buffered *and* safe to publish, publishes it.
    /// Returns `Ok(Some)` with the accepted publication stats, `Ok(None)` when
    /// nothing was published this frame — including a result retained by the
    /// publication gate because a dynamic body overlaps a collider it would
    /// add — and `Err(_)` when preparation of the current scene failed (live
    /// collision preserved; revert the edit).
    pub fn step(
        &mut self,
        scene: &DetailScene,
        physics: &mut Physics,
    ) -> Result<Option<DetailCollisionStats>, String> {
        if !self.controller.available() {
            // Worker gone: nothing will ever complete. Republish the current
            // source synchronously (once per new source version) so an edit
            // cannot stay pending forever; the publication gate does not
            // apply to this load-time path because the owner serializes it
            // with stepping the same way.
            let version = scene.source_version();
            if self.published_version.as_ref() == Some(&version) {
                return Ok(None);
            }
            let stats = physics.replace_detail_scene(scene)?;
            self.published_version = Some(version);
            return Ok(Some(stats));
        }
        let Some(prepared) = self.controller.poll_retaining(scene, |outcome| match outcome {
            // Gate: defer publication while a dynamic body overlaps any
            // collider the preparation would add. Errors pass through.
            Ok(prepared) => !physics.detail_publication_blocked(prepared),
            Err(_) => true,
        }) else {
            return Ok(None);
        };
        match prepared {
            Ok(prepared) => {
                let stats = physics.publish_detail_scene(scene, prepared)?;
                self.published_version = Some(scene.source_version());
                Ok(Some(stats))
            }
            Err(error) => Err(error),
        }
    }

    /// Scene replacement or load: cancels pending and buffered work and
    /// invalidates the job in flight, so nothing prepared for a retired scene
    /// can ever publish. A fresh [`on_edit`](Self::on_edit) works immediately.
    /// The next unavailable-worker fallback republishes after a reset, too:
    /// [`Self::forget_publication`] clears the version tracked here.
    pub fn reset(&mut self) {
        self.controller.reset();
        self.forget_publication();
    }

    /// Clears the tracked last-published version, so an unavailable-worker
    /// fallback republishes the current source synchronously on the next
    /// `step` even if its version was published before a reset or reload.
    pub fn forget_publication(&mut self) {
        self.published_version = None;
    }

    /// False after worker startup failure or an unexpected worker exit; edits
    /// then take the synchronous fallback in [`on_edit`](Self::on_edit).
    pub fn available(&self) -> bool {
        self.controller.available()
    }

    /// Live queue counts for diagnostics; see [`AsyncDetailStats`].
    pub fn stats(&self) -> AsyncDetailStats {
        self.controller.stats()
    }
}
