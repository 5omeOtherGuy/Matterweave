# Required showcase — alien fungal wetland

Status: specification, not generated content. Owner requirement, 2026-09-08: a
small, highly detailed, dense, lush explorable map with a very high voxel count,
dense vegetation, water, a complex height map, a Morrowind-esque semi-alien
atmosphere and carefully modeled, instantly recognizable mushrooms. Required output
of the [optimization campaign](PERFORMANCE_PLAN.md).

## Visual direction and exploration

Create an original wetland enclosed by ash-colored ridges, with a sheltered fungal
grove, a water basin and creek, eroded gullies, shelves/overhangs and a high viewpoint.
Use strange silhouettes, muted mineral colors, warm ochre caps, mossy teal foliage
and restrained luminous accents. The reference is an atmospheric direction, not
permission to copy Bethesda maps, models, names, textures, interface or music.
Lushness must come from overlapping plant layers and deliberate composition, not
uniform random scattering or a few enormous objects hiding empty ground.

Initial layout target: roughly 128 m × 128 m of traversable landscape with up to
64 m of vertical extent, a loop taking about 3–5 minutes to walk, a short waterside
route and an elevated alternate route. These dimensions are proposed implementation
budgets, not newly imposed owner constraints. Traversal time must be measured.
Provide near, middle and distant detail: roots/ground cover; clustered small fungi
and fronds; tall mushroom canopies and twisted woody vegetation; distinctive ridge
silhouettes. Create sheltered interiors and open vistas that expose different
occlusion, shadow and streaming costs. Retain a bounded 64-piece destruction area
in a clearing as an engine demonstration, not the only interesting place to visit.

The terrain must have coherent drainage, slopes, terraces, gullies and local relief;
combine authored masks/landmarks with a deterministic height function. Add selected
voxel overhangs/cavities where a heightfield alone cannot represent the shape.
Water levels and banks must fit the terrain. Avoid featureless noise hills, abrupt
square map edges, inaccessible attractive paths and vegetation that obscures all
navigation. Record fixed seeds, generator versions, landmarks and camera routes.

## Voxel detail and honest counts

Build the map and flora from meaningful authoritative voxel data. Render geometry,
visibility/LOD, lighting and collision are derived representations. Repeated plants
may share sparse voxel prototypes plus authoritative transforms/material variants;
that is valid reuse, not a requirement to duplicate all bytes in memory. Do not
replace the requirement with triangle-only decorative models.

Initial content targets, subject to documented feasibility experiments:

- At least 20 million occupied voxel cells represented in the fully detailed
  authored scene, including placed instances; separately report unique stored
  prototype cells, instance-expanded occupied cells, resident cells, meshes/triangles
  and bytes.
- At least 2 million of those cells in above-ground flora/surface detail, with
  visible close-range detail. Buried terrain alone must not satisfy the density
  objective.
- At least 4,000 placed vegetation instances across at least 4 distinctive mushroom
  archetypes and 6 other flora archetypes, distributed in convincing habitat
  clusters.
- Fine flora/detail voxel sizes around 6.25–12.5 cm; test 25 cm terrain/detail
  volumes where useful. These are source-resolution candidates, not a global coordinate-scale
  change to apply blindly to existing physics, saves and chunk math.

Counts are engineering targets for 'very high', not achieved facts. Validate them
from the generated authoritative data; do not count air, an allocated bounding box,
LOD duplicates, render triangles or hypothetical maximum capacity as occupied cells.
Shared prototypes and instance-expanded counts must never be conflated. If a target
proves infeasible, provide the measured bottleneck and proposed change; do not quietly
lower density and declare the original showcase satisfied.

Start with one representative 16 m × 16 m tile, then expand using measured allocation,
load-time and render budgets. Choose sparse storage, prototype instancing, surface
extraction and explicit LOD based on this tile. Keep finest source data/edits intact
when showing coarser distant detail. Pin generation parameters and hash artifacts.
Avoid attaching every grass tuft to a dynamic rigid body. Static collision/query
policy must be explicit for trunks, large caps, rocks and walkable structures;
small nonblocking leaves/grass can be decorative. LOD must not remove physical walls.

## Mushroom modeling gate

Each mushroom needs a designed silhouette and coherent anatomy before bulk placement.
Include umbrella/parasol, funnel and clustered cap-and-stem forms; a shelf/bracket
species can provide a fourth contrasting form. For gilled species, model cap crown,
rim thickness/underside, radial gills, a distinct supporting stipe and believable
cap/stem connection. Give the bracket species its appropriate attachment and pore
or ridge underside rather than forcing an unrelated stem.

Do not accept stacked cylinders with an undifferentiated blob on top. Shape caps
with intentional profiles, slight asymmetry, color zoning and a readable underside.
Use deterministic bounded variation in size/lean/cap shape rather than arbitrary
noise that destroys identity. Root bases into appropriate ground/wood without
floating, severe intersections or mechanically repeated rows.

Before accepting each prototype, render front/side/underside views, a solid silhouette
and close/medium/distant in-engine views at recorded distances. Check cap/stem/rim
readability, underside structure, exposed holes/winding, LOD silhouette loss and
shadow artifacts. Lead visual judgment is required: a voxel-count test cannot prove
recognizability. Image critique requires verified image input; findings remain
candidates. The mushroom gate precedes large-scale scatter.

## Water, vegetation and materials

Deliver visibly recognizable water: coherent basin/creek surfaces and banks, readable
color/depth cues, restrained motion/ripples and appropriate highlights. Store the
water region as meaningful world/material data; its visible surface may be derived.
Define whether the player walks, wades, floats or is blocked, and test that behavior.
A full fluid solver is not required. Evaluate transparency, reflections and animated
vegetation as explicit quality/cost choices; do not promise GI or refraction merely
because water exists. Use opaque/masked alternatives when equivalent appearance is
validated, with no hidden downgrade of the delivered scene.

Show intentional material variation on fungus, wet rock, ash, moss and woody plants.
Avoid flat placeholder colors as the final showcase. Optional gentle foliage motion
must preserve stable gameplay collision and have an explicit animation budget.
Use original procedural/art-authored content and record adopted component licenses.

## Showcase definition of done

1. A deterministic fresh build/load produces the full map with validated source
   voxel/instance counts, generator/artifact hashes and bounded resource manifests.
2. All mandatory terrain, vegetation, water and mushroom features are present and
   pass the prototype/gallery and integrated visual checks. No placeholder cube forest.
3. Ground and elevated routes are traversable on the phone; vegetation placement,
   water interaction, collision boundaries, edit/save/reload and lifecycle recovery work.
4. Near/far views and camera movement are reviewed for LOD popping, thin-feature loss,
   shimmer, shadow/contact artifacts and occlusion. Captures include worst-case vistas.
5. Performance evidence includes stationary, moving, water/vegetation-heavy and
   destruction workloads on this map. Quality settings and source counts accompany
   every comparison. An empty-scene win does not substitute for this gate.
6. A normal player-facing presentation hides most developer counters by default,
   retains an opt-in diagnostic view, and makes the route and interactions usable.
7. The tested Android APK, representative screenshots/video, build/scene manifests
   and known limitations ship in a GitHub prerelease with merged source.
