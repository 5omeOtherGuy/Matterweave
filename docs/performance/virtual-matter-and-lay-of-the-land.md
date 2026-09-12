# On-device lessons: Virtual Matter and Lay of the Land

Assessment requested by the owner, 2026-09-12. **All Matterweave runtime rendering,
simulation, generation, editing and persistence must run on the device.** Cloud
rendering, remote simulation and a required online voxel store are not acceptable
substitutes. This assessment adopts no SDK and adds no full-game feature commitment.

## Atomontage / Virtual Matter

The [vendor site](https://www.atomontage.com/) describes progressive microvoxel
streaming, persistent world editing and conversion from several asset forms, alongside
browser collaboration and optional native applications. These are vendor claims;
we did not run its engine. Native delivery alone does not establish offline operation.

Public documentation provides concrete API-level concepts:

- [VoxelData](https://docs.atomontage.com/api/VoxelData/) separates voxel data from
  its render component, exposes a content-version invalidation counter and LOD
  prioritization, and distinguishes a persistence flag from actually writing data.
- [Vox](https://docs.atomontage.com/api/Vox/) builds add/remove/paint/copy/shape
  operations. Run schedules an asynchronous edit; OnFinished reports completion.
  Explicitly naming an object avoids repeated overlap discovery when the target
  is already known. The documented API does not disclose the storage algorithms.
- The [edit tutorial](https://docs.atomontage.com/manual/scripting/examples/Voxel-Edits/)
  still says server-only, whereas current Vox is labelled Client/Server. This is
  insufficient evidence for a supported fully local deployment. Deprecated VoxelDB
  and VoxelEdit pages must not be treated as current integration contracts.
- [RigidBody](https://docs.atomontage.com/api/RigidBody/) exposes mass, inertia and
  force/impulse controls. That alone does not establish per-voxel material simulation,
  fracture algorithms or a solver we can reuse.

**Disposition:** adapt useful architectural concepts; do not integrate the cloud
platform. Progressive streaming can instead mean local storage → RAM → GPU in
Matterweave; this is our design inference, not a claim about Atomontage internals.
Versioned authoritative data and derived rendering/collision already fit our design.
Make edit submission, publication and durable save distinguishable to callers;
exercise delayed/stale completion and save/reload through the same contracts.
Asset voxelization is a later reuse assessment against documented formats and licenses.
No inspected source establishes an obtainable offline Android engine SDK, Rust bindings,
redistribution terms or usable engine source. Public documentation source is not engine
source. Any future dependency needs those facts before a bounded integration trial.

## Lay of the Land

The [developer's store description](https://store.steampowered.com/app/2776090/Lay_of_the_Land/)
presents spreading fire, flowing water, collapsing sand, environmental gas, destructible
terrain and trees, procedural landscapes, and shape-based building with reusable
prefabs. These are useful outcome references; we have not independently played or
profiled the game. Its listed Windows/discrete-GPU requirements provide no Android
performance evidence.

In a [direct developer reply](https://steamcommunity.com/app/2776090/discussions/0/595158831570535221/),
Tooley1998 confirms using C++ and Blueprints and reusing Unreal systems where available.
The same thread contains another user's attributed Discord quotation about a custom
voxel system feeding Unreal components; that quotation is secondary and is not used
here as verified architectural evidence. No inspected primary source exposes a
reusable voxel engine library or its exact renderer, fluid or fracture implementation.

**Disposition:** use as an interaction and authoring reference. Use a small engine
validation scene in which dynamic debris collides with edited terrain and a
constructed opening remains open after save/reload. Automatic detachment after
support removal is a separate bounded trial, not a new S4 requirement. Existing
D2 destruction and 64-piece acceptance remain required. Reuse our
physics adapter and world-version contracts. Do not copy game assets or mechanics,
switch to Unreal, or infer that every microvoxel needs an independent rigid body.
Fire, liquids, granular matter and gas should each be bounded later capability trials
if selected; this request to assess a reference does not make all four prerequisites
for technical completion. Existing fluid/soft-body non-goals remain intact.

## Consequences for the roadmap

| Stage | Useful input | Functional evidence |
| --- | --- | --- |
| S1/S2 | Fine geometry plus local progressive detail | Near/far transitions retain edited openings; authoritative edits survive cache eviction and reload from local storage; derived LOD never overwrites fine state |
| S2 | Explicit asynchronous edit and save contracts | Stale jobs rejected; completion means published versions; save/reload matches committed state |
| S3 | Lighting follows changed matter | Open/close an enclosure and move debris; geometry and lighting converge coherently |
| S4 | Material interaction and shape edits | Dynamic-body collision against edited terrain and persistent reconstruction in a shared-engine scene |
| S5 | Entire runtime on device | Network-disabled cold start, local world load/generation, rendering, simulation, edit, save, process restart and reload on Android |

These are proposed validation interpretations of accepted goals, not completed
features. The network-disabled gate is owner-directed and **NOT RUN**. The other
session retains the phone lease. Preserve its D1/D2/D4 assignments; no new overlapping
code dispatch is created by this assessment. Development and functional completion
remain first; large performance/thermal campaigns remain afterwards.
