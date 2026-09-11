# flora-review-muse

Frozen source reviewed independently; candidate findings, not acceptance.

Leaf review — `completion-flora` frozen diff only. Tests NOT RUN. No visual-quality inference; full-map scatter/native checks deferred to lead.

Scope: `crates/matterweave-detail/src/wetland_flora.rs`, `tests/wetland_flora.rs`, `examples/wetland_flora_gallery.rs`, `src/lib.rs:33, 51-53` (re-export only). Existing mesh/storage/serial not re-audited.

Candidate 1 — enforcement-only connectivity
File/line: `src/wetland_flora.rs:205,361,452`
Trigger: builder produces disconnected body in release profile.
Consequence: `debug_assert_eq!(connected_components, 1)` is stripped in release, so `wetland_prototype`/gallery export succeeds silently with a multi-body prototype, violating the one-body requirement.
Evidence: all three builders rely on `debug_assert`; the strict `==1` check lives only in tests (`tests/wetland_flora.rs` bodies test).
Uncertainty: no disconnected output evidenced; construction (`set_path`, crowns seeded inside radii, stems) reads connected. Gap is enforcement, not observed geometry.

Candidate 2 — gallery header overclaims LOD snapshots
File/line: `examples/wetland_flora_gallery.rs:3` vs `:90-93`
Trigger: consumer follows header to fetch per-LOD snapshots.
Consequence: header promises "snapshots for source, half and quarter LODs" but code writes one `{id}.snapshot.json` from the source volume; meshes cover 3 LODs, snapshots do not. Confusion/re-derivation.
Evidence: snapshot loop iterates `prototype_ids` once, `volume.snapshot()` only.
Uncertainty: likely doc-only; source-authoritative + derived LODs may be intentional. No geometry impact.

Candidate 3 — shrub taper asymmetric on z-branches
File/line: `src/wetland_flora.rs:193`
Trigger: branches with `dir=[0, 1]`/`[0, -1]` (`SHRUB_BRANCHES` h=9, 11).
Consequence: taper side cell is fixed `cell[2]+1`; collinear with travel for z-branches (redundant with `set_path`), lateral only for x-branches. Stated "two cells wide at takeoff" holds for x-branches, not z-branches.
Evidence: unconditional `let side = [cell[0], cell[1], cell[2]+1]`.
Uncertainty: connectivity preserved via path; visual significance not assessed here.

Non-findings: 3 archetypes, 12.5/6.25cm scales, wood-Collision/leaves-Decorative reuse with no palette additions, <33288-cell/roundtrip/LOD-mesh coverage present in tests; `bracket_shelf` stays reserved.

