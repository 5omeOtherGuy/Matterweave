# Showcase full-map completion (leaf worker log)

Scope: `crates/matterweave-detail/src/showcase.rs`, `tests/showcase.rs`,
`examples/showcase_manifest.rs` only. App, renderer and phone validation are
lead-owned and untouched here.

Status legend: PASS = executed and green, FAIL = executed and red,
NOT RUN = not executed in this session.

## 01 — Completing the ten-archetype map and repairing lead-verified review findings

- **Actions Taken:** Read the frozen map generator, the six-species catalogue,
  the attached `bracket_fungus` source and the three wetland prototypes
  (`twisted_shrub`, `horsetail`, `marsh_lily`), then implemented: open-route
  terminal waypoint, dual-route scatter exclusion with per-species source radius
  plus player clearance, carve-aware `Terrain::surface_at_metres`, honest unique
  overhang-column accounting, and placement of all ten archetypes including
  shallow-water lily pads.
- **Issues & Friction:** (filled in below as work was executed)
- **Decisions & Rationale:** (see section 02)
- **Solutions Applied:** (see section 02)
- **Insights:** (see section 02)
