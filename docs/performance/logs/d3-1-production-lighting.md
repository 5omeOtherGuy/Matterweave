# Engineering log d3-1-production-lighting

Worker slice D3.1. Branch `phase-a/d3-1-production-lighting`, base `c095047`
(current `main` at dispatch). No push, no PR: integration is the lead's row.

Owned writes used: `crates/matterweave-render/src/indirect.rs`,
`crates/matterweave-render/src/reflection.rs`,
`crates/matterweave-render/src/indirect_tests.rs` (indirect's focused test module),
`crates/matterweave-render/src/reflection.rs` inline tests, this log.
`lighting.rs` was read and left unchanged (no sun/shadow contract change is needed).
`lib.rs`, `world.wgsl`, `ray_reference.rs`, `static_scene.rs`, `shadow.rs`,
`async_indirect.rs`, `apps/explorer/**`, `Cargo.lock` and CI were not edited.

## 1. Outcome

One representative piece of production geometry — a detail prototype placed by a
`StaticInstance` record (and, through the same input shape, a moving world-space
mesh-only object) — now **participates** in the bounded indirect and reflection
representation instead of being absent from it:

- its surface cells are **sampled** as exposed faces of the indirect volume
  (`IndirectVolume::sample`), and the volume's gather/sun rays **hit** it;
- its cells enter the packed reflection material grid, so the mirror strength at
  its surface is the caller-assigned material's strength instead of air
  (`ReflectionVolume::mirror_at`, group-0 binding 4/5 content), and traced
  reflected rays hit it.

Receiving-shading and participation are kept separate, as the dispatch requires:
the slice changes what is *represented* (sampled cells, traced geometry, packed
material grid). The shader's per-fragment indirect lookup still requires an
axis-aligned fragment normal; that is a pre-existing property of the
piecewise-constant face cache, not something this slice claims to have fixed
(section 7).

## 2. What was lifted, per seam quote

`indirect.rs`, `IndirectVolume` doc: *"Unit voxel geometry only: detail meshes and
dynamic mesh-only objects are not represented by World and are not supported."*

Lifted by `MeshProxy` (`indirect.rs:117`) plus `MeshGeometry` (`indirect.rs:93`):
the resident mesh pool, the placement records and one material id per prototype are
transformed to world space (translation + quarter-turn yaw, matching
`static_scene::rotate_xz` and `world.wgsl` `quarter_rotation`) and rasterized into a
bounded cell stand-in clipped to a caller-chosen coverage box. The authoritative
`World` stays authoritative: `scene_material` (`indirect.rs:129`) reports world
material wherever the world is solid, and `trace_scene` (`indirect.rs:138`) takes
the nearest of the authoritative DDA and the proxy's DDA, keeping the world on an
exact tie. `IndirectVolume::set_mesh_proxy` (`indirect.rs:568`) attaches it; the
proxy digest is part of the cache key, so moving the object invalidates the cached
radiance and republishes from the first face.

`reflection.rs` doc: *"surfaces not represented by a covered voxel (dynamic meshes,
static instances, detail geometry) are nonreflective."*

Lifted by `ReflectionVolume::pack_with_mesh` (`reflection.rs:226`): the proxy's air
cells are merged into a copy of the authoritative world and the existing
`RayVolume::pack` is reused unchanged, so the grid, indexing order and cell cap are
not re-implemented. The volume keeps the authority's own epoch/revision/seed
(`valid_for` semantics unchanged, verified by the existing stale-publication test)
and exposes `source_mesh_digest`/`valid_for_scene`. `reflect_sample_with_mesh`
(`reflection.rs:438`) extends the CPU oracle to the same union, and
`footprint_digest` moved to `indirect.rs` (re-exported at `reflection.rs:385`) so
the proxy and the authoritative footprint use one hash.

## 3. Declared criterion (asserted, not eyeballed)

Fixtures: one unit-cube prototype produced by the engine's own mesher for a single
voxel (`World::mesh()`), a sunlit red floor (`[0.9, 0.05, 0.02]`), a unit-voxel
receiver column, a second identical mesh-only object as static control, volume
`origin [-5,-2,-5]`, `dims [11,6,11]`, 64 samples, distance 16, sun `[0,1,0]`,
intensity 1. Reflection fixture: `origin [-4,-2,-4]`, `dims [8,7,8]`, one world
mirror cell, one far wall, one mesh-only cube.

| # | Asserted criterion | Measured (this build) |
| --- | --- | --- |
| C1 | presence + direction: every exposed side face of the object is exactly zero with no mesh geometry attached, and strictly positive with the floor albedo channel dominant afterwards | before all faces `[0,0,0]`; after `+X [0.4078124, 0.0226562, 0.0090625]`, `-X [0.3937499, 0.0218750, 0.0087500]`, `+Z [0.3796874, 0.0210937, 0.0084375]`, `-Z [0.3937499, 0.0218750, 0.0087500]` (red ≈ 18× green); asserted `r > 10g`, `r > 10b` |
| C1b | top/bottom faces stay dark: one bounce with black misses cannot light the top of an object standing on the only bounce source | `+Y [0,0,0]`, `-Y [0,0,0]` at rest |
| C2 | trace participation: the receiver face's gather loses sunlit-floor red and gains the object's green albedo | `[0.3374999, 0.0187500, 0.0075000]` → `[0.3242187, 0.0304687, 0.0087500]`: red −3.9 %, green +62.5 % |
| C3 | movement + static control: lifting the object strictly lowers every side-face response, the vacated cell returns to exactly zero, and a control face that cannot see the object is bit-identical | side faces `0.4078/0.3937/0.3797/0.3937` → `0.1828/0.1836/0.1828/0.1969`; vacated all `[0,0,0]`; control `-X [0.30937493, 0.0171875, 0.006875001]` identical in both volumes; proxy digests differ (`0x46576975be0e4193` vs `0x956dbb6ee036cbb`); the control's `+X` face, whose hemisphere does contain the object, moves `0.3937499 → 0.4078124` (asserted as participation, not drift) |
| C4 | representation identity: attaching a different proxy clears published output immediately, `valid_for` is false until recomputed; a proxy that marks no cell leaves unit-voxel output bit-identical | asserted over 4 cells × 6 faces plus `resident_bytes` growth |
| C5 | rasterization: a two-voxel stacked mesh marks exactly its two cells; a unit cube marks exactly one; clipping and ascending local-index emission | `occupied_cells() == 3` for cube `[1,0,1]` + block `[3,0,0..1]`, cells `[([3,0,0],2), ([3,1,0],2), ([1,0,1],3)]` |
| C6 | validation + world precedence: material zero, per-prototype material count, dims/cell/axis/coordinate caps, prototype range, non-finite translation, yaw range, non-triangle indices, out-of-range index all rejected; merge touches only air cells | merge into a world with one solid cell writes 0 cells and keeps the world's material |
| C7 | reflection: the packed mirror strength at the mesh cell is `0.0` before and the assigned `1.0` after, the packed u32 grid and vec4 palette the shader reads carry the mesh material, world precedence holds | before `material 0 / mirror 0.0`; after `material 4 / mirror 1.0`, `materials()[local] == 4`, `palette()[4].w == 1.0`, wall cell keeps `material 2 / mirror 0.0` |
| C8 | reflection trace participation, both directions: a world mirror's reflection now hits the mesh-only object instead of the far wall, and the mesh surface's own reflection lands on world geometry | before `cell [3,4,0] material 2`; after `cell [0,1,0] material 4` at a shorter distance; mesh top face reflects onto `cell [3,3,0] material 2` |
| G1 | pinned gap: a *fractionally placed* mesh surface has a nonzero packed mirror but its reflected ray starts inside its own cell and terminates (shipped shader rule), while the same geometry on a cell boundary reflects | asserted in `g1_fractional_mesh_surfaces_still_start_inside_their_own_cell` |

## 4. Definition of done

| Criterion | Verification | Result |
| --- | --- | --- |
| One representative detail volume or moving mesh-only object participates in indirect lighting where it previously could not | C1/C2/C3 probes (`indirect_tests.rs:283`, `:326`, `:358`), before/after in one test body | PASS (representation + sampling + trace, CPU) |
| The same geometry is reflective where it was previously nonreflective | C7/C8 (`reflection.rs:986`, `:1038`) | PASS for grid-aligned geometry; sub-cell (fractional) placements stated precisely and pinned in G1 (`reflection.rs:1099`), fix proposed in section 5.2 |
| The observable criterion is declared explicitly and actually asserted | section 3; assertions read the values and compare, no screenshot/eyeball step | PASS |
| Moving the object changes its lighting response in the declared direction; a static control does not drift | C3 | PASS |
| No regression in existing indirect or reflection tests | `cargo test -p matterweave-render --lib --locked` → 95 passed (86 before this slice, +9 new); `cargo test --workspace --locked` → all suites ok | PASS |
| Any required `lib.rs`/shader-registration change is proposed, not applied | section 5 | PASS (nothing applied to `lib.rs` or WGSL) |
| `cargo test --workspace --locked` | run (section 6) | PASS |
| `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | run (section 6) | PASS (one pre-existing `winit` vendor warning, exit 0) |
| Integration of the proposed entry-point diff and the Android functional lighting run | device run | **NOT RUN** — belongs to the lead; no device access from this session |

## 5. Proposed diffs (not applied)

### 5.1 `lib.rs` — indirect entry point only (~lines 1522–1546)

`upload_reflection`, `shadow.rs` and the descriptor/shader registration need **no
change**: `shadow.rs` already uploads `IndirectVolume::values` (binding 3) and
`ReflectionVolume::materials()`/`palette()` (bindings 4/5) wholesale, so proxy cells
and face values ride along with the existing buffers, same sizes and layouts. No
WGSL change is required for the indirect half or for the reflection grid/trace.

```diff
--- a/crates/matterweave-render/src/lib.rs
+++ b/crates/matterweave-render/src/lib.rs
@@ pub fn upload_indirect(
-    /// All geometry uploads and sun changes disable GI until republished; shadow
-    /// resource replacement also disables it. Unit World voxels only, opt-in.
+    /// All geometry uploads and sun changes disable GI until republished; shadow
+    /// resource replacement also disables it. Opt-in.
+    ///
+    /// The volume represents unit World voxels plus the mesh-only geometry of the
+    /// [`indirect::IndirectVolume::set_mesh_proxy`] proxy attached to it. The
+    /// renderer cannot verify that a proxy covers every resident instance, so a
+    /// volume without a proxy is still rejected while mesh-only geometry is
+    /// resident: the caller must attach a proxy built from the same mesh pool and
+    /// instances it uploaded, over the volume's footprint.
     pub fn upload_indirect(
         &mut self,
         volume: &indirect::IndirectVolume,
         world: &matterweave_core::World,
         source_epoch: u64,
     ) -> Result<()> {
         self.shadow.disable_indirect();
-        if self.dynamic.as_ref().is_some_and(|m| m.index_count != 0)
-            || self.static_scene.as_ref().is_some_and(|s| s.has_geometry)
+        if !volume.has_mesh_proxy()
+            && (self.dynamic.as_ref().is_some_and(|m| m.index_count != 0)
+                || self.static_scene.as_ref().is_some_and(|s| s.has_geometry))
         {
             return Err("Indirect World cache does not cover mesh-only objects/instances".into());
         }
```

Optional hardening the lead may prefer instead (not implemented; it needs state the
renderer does not keep today): remember the pool/placement digest at
`replace_static_scene`/`update_static_instances`/`upload_dynamic` time and require
`volume.mesh_digest() == Some(resident_digest)`, which would turn "the caller says it
covered the geometry" into "the renderer checked that the proxy came from the same
pool and placements". The proxy must be built over a *coverage box*, which is chosen
by the volume owner (the app), so this is a caller-visible contract either way.

**Correction (review finding 3): `upload_reflection` does need a guard and an
invalidation change.** The first revision of this log said it needed none, which
left two holes: a unit-voxel pack (`source_mesh_digest() == None`) could be
published while mesh-only geometry was resident, and a pack built for an older
proxy stayed acceptable because the entry point only called `valid_for`, which does
not check mesh identity. The proposed diff below closes both, mirroring
`upload_indirect`'s guard and using the scene-aware validity check. It adds one
parameter, so the two call sites in `apps/explorer/src/reflection_check.rs:283` and
`crates/matterweave-render/examples/reflection_smoke.rs:139, 202, 221` pass the
digest of the proxy they built for that volume's footprint (`None` when they build
none).

```diff
--- a/crates/matterweave-render/src/lib.rs
+++ b/crates/matterweave-render/src/lib.rs
@@ pub fn upload_reflection(
-    /// The volume covers unit World voxels only: dynamic meshes, static instances
-    /// and detail geometry are neither reflective nor reflected. Any later geometry
-    /// upload or shadow-resource replacement disables reflection until republished.
+    /// The volume covers unit World voxels plus the mesh-only geometry of the
+    /// [`reflection::ReflectionVolume::pack_with_mesh`] proxy it was packed with.
+    /// `mesh_digest` is the identity of the proxy the caller built for this
+    /// volume's footprint, exactly as `source_epoch` identifies the World; a pack
+    /// whose proxy identity no longer matches is rejected, so moving a mesh-only
+    /// object cannot silently keep an old reflection. Any later geometry upload or
+    /// shadow-resource replacement disables reflection until republished.
     pub fn upload_reflection(
         &mut self,
         volume: &reflection::ReflectionVolume,
         world: &matterweave_core::World,
         source_epoch: u64,
+        mesh_digest: Option<u64>,
     ) -> Result<ReflectionUploadStats> {
         self.disable_reflection();
-        if !volume.valid_for(world, source_epoch) {
-            return Err("Stale reflection source revision/epoch".into());
+        if !volume.valid_for_scene(world, source_epoch, mesh_digest) {
+            return Err("Stale reflection source revision/epoch or mesh placement".into());
+        }
+        if volume.source_mesh_digest().is_none()
+            && (self.dynamic.as_ref().is_some_and(|m| m.index_count != 0)
+                || self.static_scene.as_ref().is_some_and(|s| s.has_geometry))
+        {
+            return Err("Reflection source does not cover mesh-only objects/instances".into());
         }
```

The two guards are complementary, exactly as for `upload_indirect`: the first is a
*correctness* check the renderer can make (the caller's current proxy identity
against the pack's), the second is a *coverage* declaration it cannot verify
against resident instances. Applying the digest parameter to `upload_indirect` too
(`mesh_digest` compared with `volume.mesh_digest()`) is the same one-line check and
would close the mirror-image hole there: today a volume whose proxy was never
re-attached after a move still satisfies `source_valid` when the World revision is
unchanged. Listed as an option, not part of the minimal diff above, because
`upload_indirect` has no explicit mesh guard to correct.

### 5.2 `world.wgsl` + oracle — sub-cell mesh surfaces (reflection), not applied

**Withdrawn and replaced (review finding 2).** The first revision proposed
`!(iteration == 0u && cell == self_cell)`. That fix is unsound and must not be
applied: it covers only the ray's *first* cell, so for a moved (sub-cell) mesh whose
proxy marks several cells, the ray leaves its own first cell and enters a second
cell of the same instance at `iteration == 1`, where the guard no longer applies.
The result would be a false self-reflection at 0-1 cell distance, shaded with the
object's own albedo. The shipped shader has no such artefact; its artefact is the
opposite one (below), and a half-fix would trade a missing reflection for a wrong
one.

What the shipped shader does today (asserted in G1): `specular_reflection` treats a
solid start cell as self-intersection and returns a miss. That is exactly right for
world voxels, whose *exposed* faces always have air in front. A mesh-only surface at
fractional metres lies strictly inside a proxy cell, so its reflected ray starts in
that cell and terminates: the fragment renders `REFLECTION_BACKGROUND` (the fog
target) instead of the scene. Grid-aligned mesh faces are unaffected and reflect
correctly (C8). The artefact is therefore: *sub-cell mesh surfaces are reflective in
the packed data and block/are hit by other reflections, but their own reflection
shows background/fog rather than the scene.* No false geometry is shown.

**Why it cannot be fixed correctly inside `world.wgsl` alone.** The guard has to
skip the whole proxy footprint of the *originating instance*, not one cell, and the
packed grid carries no instance identity: `reflection_materials[index]` is a
material id shared by every instance of a prototype and by every object mapped to
that material, so a material-level rule would also skip *other* objects that happen
to share it. Applying a start-cell-only skip would introduce the false
self-reflection above. The smallest correct fix adds per-cell instance identity to
the data the shader already reads, which means the packer, the shader and the CPU
oracle change together:

1. `indirect.rs` — `MeshProxy` keeps a per-instance tag next to each cell
   (1-based instance index, so it never collides with the authoritative world's 0):
   `cells: Vec<(u32, u8, u32)>` plus `pub fn tagged_cells(&self) -> impl Iterator<Item = ([i32; 3], u8, u32)>`.
2. `reflection.rs` — `pack_with_mesh` stops merging into a cloned `World` (so no
   clone, no `World::set` rejection path, and the pack's own provenance matches the
   authority again) and instead overlays the proxy into a local tagged grid:
   `tagged: Option<Vec<u32>>` copied from `pack.materials()`, each proxy cell written
   as `u32::from(material) | (tag << 8)`; `materials()` returns
   `self.tagged.as_deref().unwrap_or(self.pack.materials())` and `material_at()`
   masks with `& 0xFF`.
3. `world.wgsl` — mask the material at both palette lookups
   (`reflection_mirror` and the hit branch of `specular_reflection`) and replace the
   start-cell rule with a same-tag run skip:

```diff
--- a/crates/matterweave-render/src/world.wgsl
+++ b/crates/matterweave-render/src/world.wgsl
@@ fn reflection_mirror(world: vec3<f32>, normal: vec3<f32>) -> f32
-    return reflection_palette[reflection_materials[index]].w;
+    // High bits carry the instance tag; only the low byte indexes the palette.
+    return reflection_palette[reflection_materials[index] & 0xFFu].w;
@@ fn specular_reflection(...) -> ReflectionSample
+    // Instance tag of the surface being shaded, 0 for authoritative World voxels.
+    let self_tag = ... // reflection_materials[mirror_cell_index] >> 8u
     for (var iteration = 0u; iteration < bound; iteration = iteration + 1u) {
         if any(cell < vec3(0)) || any(cell >= vec3<i32>(dims)) { return out; }
         let index = u32(cell.x) + dims.x * (u32(cell.y) + dims.y * u32(cell.z));
-        let material = reflection_materials[index];
+        let tagged = reflection_materials[index];
+        let material = tagged & 0xFFu;
+        // A mesh-only surface can start strictly inside its own proxy cell. Skip
+        // the contiguous run of the originating instance's cells instead of
+        // either self-hitting them or terminating; the run ends at the first air
+        // cell, so a later, genuinely different part of the same object is still a
+        // hit. World voxels have tag 0, which never matches, so their path is
+        // unchanged (their exposed faces always have air in front).
+        if self_tag != 0u && tagged >> 8u == self_tag { continue; }
         if material != 0u {
-            // Self-intersection: a solid start cell is the surface being shaded.
-            if iteration == 0u { return out; }
+            if iteration == 0u { return out; }   // unchanged for world voxels
```

Hmm — the `continue` above must still advance the DDA. In the shipped loop the
advance is the code after the hit branch, so the applied version needs the skip as
`if self_tag != 0u && tagged >> 8u == self_tag { } else if material != 0u { ... }`
with the stepping code kept last; the exact restructuring is mechanical and must be
written against the loop when applied. `continue` is spelled out here to state the
rule, not as a literal hunk. The oracle applies the same rule: advance the ray
origin past the contiguous same-tag run (min over axes of
`(boundary - origin) / direction`, +1e-6 along the direction) and add the advance to
the reported distance, before tracing the remaining distance with the authoritative
DDA. `reflection_validation.rs` compares GPU against this oracle, so packer, shader
and oracle must land in one commit.

Verified vs argued: the rule's need is measured (G1, and the false-self-reflection
case is the reason the first proposal is withdrawn); the tag plumbing is not
implemented here and its cost is bounded but unmeasured (+4 bytes per cell only for
mesh packs, `pack_with_mesh` becomes allocation-free in the derived-world sense).
The change is confined to the specular path: the CPU indirect volume reads the proxy
directly, has no self-intersection guard and needs no tag. **Do not apply the
tagging without the shader mask**: an unmasked tagged value indexes the palette out
of range.

If the lead prefers not to add tags in this phase, the honest interim is to leave
`world.wgsl` unchanged: sub-cell mesh surfaces show background/fog in their own
reflection (stated artefact above) while everything else in this slice holds. The
withdrawn one-branch guard is worse than both options.

### 5.3 `async_indirect.rs` — mesh participation on the async path (not applied)

`AsyncIndirectLight` builds its own `IndirectVolume` inside the worker from
`AsyncIndirectConfig`, so a proxy attached on the render side cannot reach the
background job; the async path stays unit-voxel-only until `AsyncIndirectConfig`
carries the proxy (or its geometry + coverage box, `MeshProxy` is not `Copy`) into
`build()` and the digest joins `SourceKey` so a moved object retires in-flight work.
Not attempted: it changes the snapshot/dedup semantics of a module outside this
slice's write grant.

### 5.4 One-line reuse opportunity (not applied)

`static_scene::rotate_xz` is private, so the proxy carries a 4-arm copy of the same
rotation with a cross-reference comment and the rasterization tests pin the mapping.
Making it `pub(crate)` and calling it from `MeshProxy::build` would remove the copy.

## 6. Verification run (exact commands)

```
cargo fmt --all -- --check          -> clean
cargo clippy --workspace --all-targets --locked -- -D warnings
                                    -> exit 0; one pre-existing vendor warning
                                       (vendor/winit/.../x11/ime/context.rs:161,
                                       function_casts_as_integer), untouched by
                                       this slice, not promoted to an error
cargo test -p matterweave-render --lib --locked
                                    -> 95 passed, 0 failed (86 before + 9 new)
cargo test --workspace --locked     -> 45 suites ok, 0 failed, exit 0
```
No GPU or Android step was executed from this session; nothing here is a device
result, a frame time or a visual claim.

## 7. Source-coverage limitations (exact)

1. **Sub-cell mesh surfaces and the specular ray start.** Section 5.2. Grid-aligned
   mesh faces reflect today; fractional placements (the normal case for detail
   geometry, whose translations are world metres) currently terminate as
   background/fog. Pinned by G1.
2. **Fragment-normal alignment for indirect reception.** `world.wgsl`
   `indirect_diffuse` returns zero unless the fragment normal is axis-aligned
   (`abs(n) > 0.999`); this slice changes what is *represented* (the face cache and
   the reflection grid), not that lookup rule. A mesh-only fragment therefore
   receives indirect shading only where its normal is axis-aligned and its cell
   face has a sampled value. This matches the dispatch's separation between
   receiving and participating.
3. **One-cell resolution, conservative rasterization.** The proxy marks the cells a
   triangle's surface actually intersects, using a separating-axis triangle/cell test
   (see section 10.1; the first revision marked the triangle's whole AABB, which
   turned slanted surfaces into solid volumes). Contacts count as intersections, so
   a cell the surface only grazes at a corner is marked on purpose and rays cannot
   slip through. A triangle that is flat in an axis additionally marks the cell on
   the side it faces into when that plane is an integer boundary, which is the cell
   `world.wgsl`'s mirror lookup reads. Exact for axis-aligned faces on the world
   cell grid (asserted for the unit cube = 1 cell and the two-voxel stack = exactly
   2 cells). A slanted surface is not an AABB stand-in; measured diagonal fixtures
   mark 18–21 of the 64 cells of a 4³ box that the old rule filled completely,
   with every sampled surface point still covered.
4. **Coverage box is the caller's choice.** Geometry outside it does not
   participate; the indirect gather reaches up to `distance` beyond the volume, but
   the proxy itself is bounded by its own box. `IndirectVolume`'s exposure loop and
   the reflection pack only read inside their own footprints.
5. **`MeshProxy::build` is a load/edit-time operation, not per frame.** It allocates
   a box-sized occupancy scratch, a cell list and one `World` chunk per touched
   16³ region, and it rejects work above `MAX_MESH_PROXY_TESTS = 4_194_304`
   triangle/cell overlap tests and boxes above `MAX_MESH_PROXY_CELLS = 64³` cells
   rather than silently dropping geometry.
6. **Dynamic meshes** arrive world-space (`Renderer::upload_dynamic(&Mesh)`) with no
   placement record; they are expressible as a one-prototype pool with one identity
   placement, which is what the C3 probe uses. No new engine-wide abstraction was
   added for them.
7. **Async path** (5.3) and **coverage proof** (5.1) as stated above.

## 8. Costs (bounded, not measured — Phase B owns ranking)

- Volume build: one pass over the box, one cell-list and one `World` allocation for
  the proxy; `resident_bytes()` now includes it (a one-cell proxy reports one
  resident world chunk).
- Volume update: per ray, one extra bounded proxy DDA, and only when the segment
  can enter the coverage box (`MeshProxy::segment_exit` rejects in O(1) otherwise);
  the proxy raycast is additionally capped at the current best hit distance. The
  indirect gather reserves two rays per sample as before.
- Reflection pack: with a proxy, one `World` clone plus one `set` per air cell
  (world-solid cells are skipped, and a rejected merge is a hard error, not a silent
  drop). No per-frame cost: packing is publication-time.
- No fence, upload or frame-path change; the single 7.974 ms fence observation was
  not investigated (deferred, per the dispatch).

## 9. Integration contract / next actions

1. Apply 5.1 (one entry point) and, if the device run is to show reflective mesh
   detail away from the grid, 5.2 (shader + oracle in one commit).
2. App/lead side: build `MeshProxy::build(&MeshGeometry { meshes, instances,
   materials }, volume_origin, volume_dimensions)` from the *same* pool and
   instances passed to `replace_static_scene`, with one nonzero palette/material id
   per prototype; `set_mesh_proxy(Some(proxy))` before `update`/`upload_indirect`;
   rebuild and re-attach when the placements or meshes change (compare
   `proxy.digest()` with `volume.mesh_digest()`), then republish — instance updates
   already retire the GPU publication via `update_counters`.
3. Reflection: use `ReflectionVolume::pack_with_mesh(...)` and
   `valid_for_scene(world, epoch, Some(proxy.digest()))` when deciding whether an
   existing pack can be reused.
4. Device evidence to collect (lead): one detail volume and one moving mesh-only
   object, before/after with the proxy absent/present, with the scene, seed, build
   and commit recorded; the indirect probe result is expected to be a wall/floor
   face losing red and gaining the object's albedo, and the mesh object's own side
   faces lit by the floor.

## 10. Correction pass (independent review findings 1–4)

Same branch, same owned paths, committed on top of the slice (`lib.rs` and the WGSL
still untouched). The four findings were checked against this source before acting;
all four were real.

### 10.1 Slanted triangles filled their whole bounding box (finding 1, code)

`MeshProxy::build` marked every cell in a triangle's AABB, because the per-axis range
only collapsed when the triangle was exactly flat on that axis. A slanted triangle
spans all three axes, so a 2D surface became a solid volume: false occlusion in the
indirect volume and false reflection hits on exactly the wetland's dominant slanted
geometry (flora, branches, foliage).

Fixed by `Triangle::intersects_cell` (`indirect.rs`), the classic 13-axis
separating-axis test between one triangle and one closed cell (three cell axes, the
triangle normal, nine cell-axis × triangle-edge cross products), run per candidate
cell inside the existing AABB range. `Triangle` is built once per triangle (origin,
three edges, normal) and reused for every candidate. `MAX_MESH_PROXY_TESTS` now
counts these real intersection tests, so the budget stays meaningful.

Evidence (4³ coverage box, all fixtures fail on the previous commit by marking 64 of
64 cells):

| Fixture | Cells marked | Surface points covered |
| --- | --- | --- |
| Sheet in the plane `z = y` spanning the box | 21 of 64 (was 64) | all 153 sampled points marked (`c11`) |
| Thin diagonal ribbon along `x = y = z` | 18 of 64 (was 64) | corner-touching cells are marked by design |
| Axis-aligned unit cube (regression) | 1 (unchanged) | — |
| Two-voxel stack (regression) | 2 (unchanged) | — |

New tests, all passing: `c9_slanted_triangles_mark_only_cells_they_intersect`
(fails on the pre-correction commit: `cell [0, 3, 0] is more than one cell from the
z = y plane`, marked 3), `c10_proxy_triangle_budget_is_enforced` (17 box-spanning
triangles exceed the budget with an explicit error; the same box with one triangle is
accepted, so cost is bounded and memory stays box-bounded),
`c11_slant_rasterization_covers_sampled_surface_points` (independent check in the
other direction: sample the surface itself, require every sampled point's cell to be
marked, so the intersection test cannot open a gap),
`c12_degenerate_and_planar_triangles_stay_bounded` (zero-area triangle away from a
boundary = exactly 1 cell; on a cell corner = at most 8; a quad flat in the integer
plane `y = 2` marks the cell it faces into and nothing on the far side, for both
winding directions).

### 10.2 The proposed self-hit fix was unsound (finding 2, log)

Corrected in place in section 5.2: the `!(iteration == 0u && cell == self_cell)`
variant is withdrawn as unsound (it would introduce a false self-reflection at
iteration ≥ 1 for a sub-cell mesh whose proxy spans several cells), the precise
reason it cannot be fixed in `world.wgsl` alone is stated (the packed grid carries no
instance identity; material ids are shared across instances and prototypes), and the
smallest correct design is proposed instead: a per-cell instance tag in the high bits
of the existing u32 grid (no new binding), masked at both palette lookups, with a
contiguous same-tag run skip from the ray origin, applied atomically with the packer
and the oracle. The shipped shader's actual artefact is stated exactly: sub-cell mesh
surfaces show `REFLECTION_BACKGROUND`/fog in their own reflection (no false geometry),
grid-aligned mesh faces reflect correctly, and the unrepaired case is preferred over
the half-fix. The section also warns that tags must never reach an unmasked shader
(they would index the palette out of range).

### 10.3 `upload_reflection` invalidation (finding 3, log) — see section 5.1

The claim that `upload_reflection` needed no change was wrong. The corrected
proposal adds `mesh_digest: Option<u64>` to the entry point, checks the pack with
`valid_for_scene`, and mirrors `upload_indirect`'s coverage guard
(`source_mesh_digest().is_none()` while mesh-only geometry is resident → reject),
listing the call sites that need the new argument. Still proposed, not applied.

### 10.4 `MeshProxy::digest()` contract (finding 4, code)

Documented on the method: the digest is taken from the rasterised cells, so it
detects any change that moves a surface across a cell boundary and every edit that
adds or removes a cell, but a sub-cell translation that keeps every triangle inside
its current cells leaves it unchanged; the representation is identical there, so
cached radiance stays valid for it. A caller that uses this three-line contract to
skip a rebuild is skipping work for a scene that is unchanged at proxy resolution.

### 10.5 Definition of done

| Criterion | Verification | Result |
| --- | --- | --- |
| A slanted triangle marks only cells it actually intersects; untouched interior cells stay empty | `c9` (ran first against the pre-correction commit: FAIL, `cell [0, 3, 0]` marked; then PASS after the fix) | PASS |
| Axis-aligned behaviour and every existing lighting test are unchanged | `cargo test -p matterweave-render --lib --locked` → 99 passed (95 before + 4 new); unit cube and two-voxel stack counts unchanged | PASS |
| The triangle/cell test budget is still enforced; a degenerate mesh cannot exhaust memory or time | `c10` (explicit budget error), `c12` (degenerate triangles bounded at 1 and ≤ 8 cells), box caps and allocation checks unchanged in `c6` | PASS |
| The self-hit guard covers the originating instance's whole proxy footprint, or the log states why it cannot and what the artefact is | section 5.2 (rewritten): tag design for the whole footprint, unsound variant withdrawn, artefact stated | PASS |
| The proposed `upload_reflection` diff uses the scene-aware check and stays proposed | section 5.1 diff; `git diff --stat` shows no `lib.rs` change | PASS |
| `MeshProxy::digest()` documents what it does not detect | doc comment on `MeshProxy::digest` | PASS |
| `cargo test --workspace --locked` | run (10.6) | PASS |
| `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | run (10.6) | PASS |
| Correction accepted against the source rather than the summary | lead review and re-run | NOT RUN |
| Android functional lighting run | device run | NOT RUN |

### 10.6 Verification run (exact commands, correction commit)

```
cargo fmt --all -- --check          -> clean
cargo clippy --workspace --all-targets --locked -- -D warnings
                                    -> exit 0 (same pre-existing vendor/winit
                                       warning, untouched)
cargo test -p matterweave-render --lib --locked
                                    -> 99 passed, 0 failed
cargo test --workspace --locked     -> all suites ok, 0 failed, exit 0
python3 tools/check_docs.py         -> PASS
```
`c9` was run before the fix and failed as quoted in 10.1, so the new test is
demonstrably sensitive to the defect it covers. No GPU or Android step was executed
from this session; nothing here is a device result.
