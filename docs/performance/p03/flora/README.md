# P03 flora catalogue and dense test tile

This is a host-verified source-data increment for `matterweave-detail`. Native
rendering, mobile cost and the complete showcase remain separate acceptance
gates. The existing sparse `gallery_scene(2026)` and its public foundation stay
unchanged. See [verification](verification.md) for the final artifact counts and
checks; [execution log](logs/execution_log.md) records failed attempts and repairs.

## Content and API

`dense_tile(FLORA_CANONICAL_SEED)` builds a separate 16 m terrain fixture with
reused flora prototypes, deterministic habitat placement and quarter-turn yaw.
The canonical seed is **20260908**. `FLORA_GENERATOR_VERSION` and the existing
terrain generator version are exported separately. `flora_prototype(id)` exposes
the individual assets; the named builder functions accept a caller-selected ID.

| Prototype | Source scale | Intended anatomy | Source policy |
| --- | --- | --- | --- |
| Existing parasol mushroom | 6.25 cm | Broad cap, rim, stipe and gills | Collision |
| Funnel mushroom | 6.25 cm | Depressed open centre, rolled rim, underside gills, stipe | Collision |
| Clustered mushroom | 6.25 cm | Four distinct caps and stems joined through roots | Collision |
| Fan frond | 12.5 cm | Broad tapering blades, central ribs and curved stems | Decorative |
| Reed cluster | 12.5 cm | Upright culms, lateral leaves and contrasting plumes | Decorative |
| Rosette groundcover | 12.5 cm | Low radial leaves, contrasting heart and patterned tips | Decorative |

These are original procedural assets. Named palette entries 30–48 are additive;
existing material IDs retain their meaning. The two luminous-looking accent
materials have separate collision policies for fungi and leaves. Their colours
are **not an implemented emissive-lighting system**. Water retains the existing
Liquid policy and opaque host mesh representation.

The tile has bounded placement searches and returns an error if the declared
minimums cannot be met: 64 plants, 40,000 instance-expanded flora cells, five
types and four placements per planned species. Terrain cells do not count toward
the flora threshold. Unique stored cells and expanded counts are reported
separately. This representative test does not replace the full showcase targets
in [SHOWCASE](../../../SHOWCASE.md).

Placement uses the terrain source, dry moss root contacts and water proximity
for reeds. A central two-metre strip excludes plant origins; canopy overhang is
possible. It is a placement rule, not proof of a collision-free walking route.
Neither native traversal nor whole-map habitat quality is established here.
The reserved Hy4 bracket mushroom is not implemented by this lane.

## Integration boundaries

Use the existing scene draw list and one cached mesh per prototype/LOD. The
source remains authoritative for policy and queries. The previous conservative
33,288-cell mesh preflight remains intact; large worlds require bounded
prototypes and instances, not one giant accepted volume. No renderer, physics,
save schema, source storage or mesh-budget implementation changes belong here.

Start native visual evaluation with **Source LOD**. Half and Quarter produce
valid meshes and preserve source data, but the coarse shapes lose important
gills, funnel cavities and distinct stipes. Their close-view appearance and
transitions are not accepted. Source connectivity does not establish adequate
screen-space thickness at distance.

The terrain footprint is 16 m square; foliage can overhang its edges. There is
no full 128 m showcase, streaming integration, dynamic vegetation, transparent
water pass or device performance claim in this increment. All exported byte
counts exclude allocator/map overhead, transient build scratch, instance records
and GPU allocations unless explicitly stated otherwise.

## Reproduction

From a checkout containing this increment, with build and artifact paths under
`/mnt/bench`:

```bash
export CARGO_TARGET_DIR=/mnt/bench/matterweave-flora-check/target
cargo test -p matterweave-detail --locked
cargo clippy -p matterweave-detail --all-targets --locked -- -D warnings
cargo fmt -p matterweave-detail --check
cargo run -p matterweave-detail --release --example flora_gallery -- /mnt/bench/matterweave-flora-check/gallery
python3 docs/performance/p03/validate_gallery.py /mnt/bench/matterweave-flora-check/gallery
python3 docs/performance/p03/flora/inspect_geometry.py /mnt/bench/matterweave-flora-check/gallery
python3 tools/check_docs.py
```

The existing validator checks exported counts, mesh indices/winding and source
surface area. The additional inspector reconstructs source voxels independently
of the placement helper, checks six-neighbour connectivity, and compares actual
transformed root/voxel centres with the terrain. Its scope is this trusted,
identity-terrain fixture, not arbitrary imported files or general collision
detection. The manifest and little-endian buffers are reproducible content;
`timing.json` contains separate host-only measurements.
