# Generator3 route review — route-review-astra

Frozen target `d58c85e`, integrated as `0d8225b`. Read-only Pi review.

### Finding — missing recorded short waterside itinerary
**Location:** `crates/matterweave-detail/src/showcase.rs:934–936`; `tests/showcase.rs:546`; `docs/SHOWCASE.md:20`.

The documented layout includes a long loop, a **short waterside route**, and an elevated alternate. The generator exposes only the long loop and elevated route; tests validate those two without identifying a short waterside itinerary.

**Consequence:** Passing both traversal gates does not establish this remaining layout target. A waterside portion of the loop might suffice, but its endpoints and independent traversal evidence are not recorded here.

**Action:** Designate an existing loop subsection as the short waterside itinerary, or generate one, and validate its endpoints, water proximity, and traversal. This is a demonstrated acceptance-coverage gap, **not a proven newly introduced regression**.

### Verification limits
- Read both scoped files completely and `docs/SHOWCASE.md`; branch reference matches `d58c85e574023eedd4ade9dd005c68084d7f9ffe`.
- No shell, edits, delegation, or phone access.
- Did not independently execute content/Rapier tests or generation-cost measurements.
- Could not inspect the historical diff against `3af4115` with the permitted tools; regression attribution remains unverified.
- No additional substantiated correctness or serious generation-cost finding.

### Engineering log
- **Actions:** Inspected final grading, connectivity, waypoint simplification, scatter clearance, source assembly, and acceptance tests.
- **Issues:** Short waterside itinerary lacks explicit coverage.
- **Decisions:** Accepted the dirty 0.30 m autostep dependency as instructed; did not treat historical worker failures as current defects.
- **Solutions:** Reuse a designated loop subsection rather than adding unnecessary routing machinery.
- **Insights:** Supplied traversal passes support the two tested routes, not every showcase obligation; independent runtime verification remains outstanding.

Lead disposition: no new source correctness blocker. Astra waterside coverage
finding addressed by `60abc39`/`f887d67`: a recorded21-point prefix with
water-margin/endpoints checks, actual continuous traversal7.1166673simseconds.
Phone routes are a separate open gate.
