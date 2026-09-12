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

### 5.2 `world.wgsl` + oracle — sub-cell mesh surfaces (reflection), not applied

The shipped `specular_reflection` treats a solid start cell as self-intersection and
returns a miss. That is exactly right for world voxels, whose *exposed* faces always
have air in front, so the rule never fires for them. A mesh-only surface placed at
fractional metres lies strictly inside a proxy cell: the mirror lookup finds the
material (`reflection_mirror`), the grid carries it, but the reflected ray's first
cell is the surface's own cell, so the fragment renders the background/fog target
instead of the reflection (asserted in G1). Fixing it needs a one-branch shader
change plus the matching oracle change, which is why both are proposed together;
the CPU oracle at `reflection.rs` deliberately still mirrors the *shipped* shader.

```diff
--- a/crates/matterweave-render/src/world.wgsl
+++ b/crates/matterweave-render/src/world.wgsl
@@ fn specular_reflection(world_pos: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> ReflectionSample
     let origin = world_pos + normal * lighting.reflection_params.y;
+    // The cell this fragment's surface belongs to, by the same rule
+    // reflection_mirror uses. A mesh-only surface can lie strictly inside it.
+    let self_cell = vec3<i32>(floor(world_pos - normal * lighting.reflection_params.y)
+        - vec3<f32>(lighting.reflection_origin.xyz));
     let lower = vec3<f32>(lighting.reflection_origin.xyz);
@@ for (var iteration = 0u; iteration < bound; iteration = iteration + 1u) {
         let material = reflection_materials[index];
-        if material != 0u {
-            // Self-intersection: a solid start cell is the surface being shaded.
-            if iteration == 0u { return out; }
+        // Self-intersection: the fragment's own cell is not a hit. Skipping it
+        // instead of terminating lets a mesh-only surface reflect the scene;
+        // world voxel faces always have air in front, so their rays never start
+        // in a solid cell and this branch is unchanged for them.
+        if material != 0u && !(iteration == 0u && cell == self_cell) {
```

Matching oracle change (same file set I own, to be applied *with* the shader, never
alone): in `reflect_in_scene`, when the trace's first hit is the start cell at
distance `0.` and `start == surface_cell(point, normal)`, advance the origin past
that cell's exit plane (min over axes of `(boundary - origin) / direction`, then
`+ 1e-6` along the direction) and re-trace the remaining distance, adding the
advance to the reported distance. The 1e-6 is the oracle's stand-in for the
shader's exact integer stepping; `reflection_validation.rs` compares GPU against
this oracle, so the two must land in the same commit.

Estimated blast radius: one branch in the fragment shader, reachable only when a
fragment's own cell is solid (mesh-only geometry, or a mesh fragment embedded in
world voxels), plus the oracle. No uniform, binding, pipeline or registration
change; `build.rs`/naga validates the WGSL in-tree.

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
   triangle's surface passes through, plus for degenerate axes the cell behind an
   integer plane; a slanted triangle fills its AABB. So the proxy can be up to one
   cell thicker than the source along an axis and, for slanted geometry, is an AABB
   stand-in. Exact for axis-aligned faces on the world cell grid (asserted for the
   unit cube = 1 cell and the two-voxel stack = exactly 2 cells).
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
