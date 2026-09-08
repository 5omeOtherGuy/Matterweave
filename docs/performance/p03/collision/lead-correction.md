# Collision correction handoff

- **Actions Taken:** Opus replaced per-cell BTreeSet scratch with bounded sparse 4096-bit masks, checks filtered aggregate cells and boxes before shape construction, and counts actual colliders. Lead added decorative >16k instances, >1M liquid-cell and narrow-clearance/chunk-seam regression cases.
- **Issues & Friction:** Opus correction runner ended Interrupted without final handoff. Muse follow-up hit upstream429 before edits. Initial ceiling test relied on exact-touch teleport behavior, which changed under equivalent box partitioning.
- **Decisions & Rationale:** Keep conservative intersection rejection; test positive gap and penetration explicitly across partition seams. 16M unique collision admission permits the actual8.8M map without dropping walls;262144 box limit remains.
- **Solutions Applied:** Fixed filtered-budget checks; sparse scratch payload bounded32MiB per prototype plus allocator/tree overhead. Resident box memory estimates in code are estimates, not device measurements.
- **Insights:** Model completion and failure are separate from partial-code usefulness. Final tests and full-scene collision load remain lead acceptance gates.
