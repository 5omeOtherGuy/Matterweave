# Matterweave visual-fidelity specification

Target: the look of *"Solving the hardest problem in my Micro Voxel Engine"* (MishMash).
Baseline: `ours/01_spawn_shore_fields.png`, `ours/02_look_around.png` (OnePlus 13, 3168x1440).
Scope: what must change and how it will be judged. No implementation, no schedule.

---

## 0. Measurement protocol

Every numeric criterion below is settled by comparing one of our captures against one named
reference frame using one of four metrics. All were computed on the supplied frames; anyone can
recompute them in about ten lines of numpy.

- **M1 — flatness (modal-colour share).** In a square window of side `frame_width / 25`, bucket
  pixels into ±6 RGB cells, take the largest bucket, report the fraction of window pixels within
  ±6 of that bucket centre. Report the **median over all windows** in the named region. High = flat.
- **M2 — local detail (block-std).** Median standard deviation of Rec.709 luminance over 8x8
  pixel blocks, **after downsampling the capture to 1600 px wide** so our 3168 px frames are
  comparable with the 1600 px reference. High = surface carries structure.
- **M3 — tonal range.** Rec.709 luminance percentiles and the fraction of scene pixels below
  luminance 30, over the scene area only (HUD and letterboxing excluded).
- **M4 — by eye, stated crop.** Used where no honest number exists. The criterion names the crop
  and the yes/no question.

**Compression floor.** The reference is YouTube-compressed. Measured M2 on genuinely flat
reference regions (clear sky in 0132, 0018, 0245): **0.00–0.28**. Any reference M2 above ~1.0 is
real signal, not codec noise. Do not treat reference per-pixel noise as material detail.

**Scale caveat.** Our captures are 2x the reference's linear resolution. M1 window and M2 block
sizes above are defined to cancel that. Where a raw pixel count is quoted it is labelled with its
frame width.

---

## 1. What each reference frame shows, and what it implies

| Frame | What it shows | What it demands of us |
|---|---|---|
| `0001_mountains_snow_conifers` | Snow ridges receding into haze, conifers in silhouette, blocky voxel relief still legible on the far ridge (M2 = 3.95 at 1600 px), sat falls 0.03 → 0.09 with distance | Distant terrain must keep **structure and silhouette**, not dissolve to a flat colour. Aerial perspective is a gradient, not a wipe. |
| `0018_snow_detail_terracing` | Same terrain close: sub-metre terracing, thousands of distinct snow blocks, M2 = 6.54 / p90 19.2, M1 = 13% | Near terrain must be **many-valued per surface**, with readable step geometry and occlusion between steps. |
| `0037_sunset_clouds_rays` | Cream sunset sky, thick cloud deck, god rays through a canyon, a whole valley in shadow | Sky is a **lit volume**, not a backdrop. Terrain self-shadowing at landscape scale is part of the read. |
| `0055_water_closeup_reflections` | Water surface: dark mirror reflections of trunks, bright specular glints, lily pads on top, submerged plants below, bank grass meeting water with no hard seam | Water needs **fresnel + specular + depth + reflected darkness**. Dynamic range across the water alone: 64 luminance (p1–p99). |
| `0114_night_fog_mood` | Night, heavy mist filling the valley, a mountain reading only as a silhouette against fog | Fog must be **volumetric and depth-layered**, and the scene must survive having almost no light in it. |
| `0132_oasis_grass_cumulus` | **Closest analogue to our scene.** Sand, dune grass, palms, blue water, cumulus. Sky gradient zenith (191,210,240) → horizon (232,231,238). Cloud coverage 16% (25% at a loose threshold). Sand M1 = 16%, M2 = 3.33. Scene lum mean 159, sd 64, p1 29, 2.1% of pixels below lum 30 | The single frame our capture is judged against. Everything in §2 is calibrated to it unless stated. |
| `0150_valley_dusk_trees` | Green valley at dusk, layered forest stands, hills stacked in value bands, whole frame lum mean 41 sd 12 | Mid-ground is **made of trees**. Layered stands are what gives depth, not fog alone. |
| `0209_night_forest_flowers_water` | **Second closest.** Meadow of short, dense, multi-coloured grass with purple/red/yellow/white flowers and mushrooms; ground completely hidden; lake; conifer wall; fog between the trunks. Meadow hue sd 37, sat mean 0.43 | Ground cover is **continuous and polychrome**. The flower layer is a distinct species set, not a recoloured tuft. |
| `0227_g8` | River from a canoe: reflections of trunks and sky on water (p1–p99 range 101 lum), forest stands at four depths, sunlit bank grass against shadow, mountain in haze | The combined target: water + forest + aerial perspective in one frame. |
| `0245_g9` | Snow valley under a player-built roof, with the video's end-card and settings UI on screen | **Not a target.** Video furniture. Useful only as further evidence that distant snow keeps blocky relief. |

---

## 2. The gap, itemised

Each item: **Observed** (what a viewer would name, with the number) → **Accept** (the screenshot
test) → **Subsystem** → **Tier**.

---

### A1. There is no sky

**Observed.** Our sky is the single RGB value `(203,219,234)` everywhere. Sampled at five heights
across 2200 px of `02_look_around`, every row returns `203 219 234`; p5 and p95 of sky luminance
are both 216.7, i.e. variance zero. No gradient, no clouds, no sun, no horizon warming.
Reference `0132`: zenith `(191,210,240)` to horizon `(232,231,238)` — +41 luminance and a
17-point blue-to-neutral shift — plus cumulus over 16% of the sky.

**Accept.**
1. Looking at the zenith and at the horizon in one capture, luminance rises by **≥ 30** from
   zenith to horizon and the horizon is measurably less blue (B−R falls by **≥ 15**). (M3)
2. In a daytime capture framed like `0132`, clouds occupy **14–30%** of the sky area above the
   horizon, measured as pixels whose red channel exceeds the per-row clear-sky median by >6.
   (Derived: 15.9% measured in `0132` at that threshold, 24.8% at a looser one; the band gives
   room for weather variation without permitting an empty or an overcast sky.)
3. Cloud edges are soft: M2 across a cloud boundary is **≥ 2.0** and no cloud silhouette is a
   straight axis-aligned pixel run longer than 1% of frame width. (`0132` cumulus have no hard
   edges anywhere.)

**Subsystem.** Sky/cloud pass (the existing PR), lit by the same sun as lighting and shadows.

**Tier A.** It is the largest single area of our frame and it is currently blank. Nothing else
closes as much perceived gap per unit of work.

---

### A2. Our frame has no dark end

**Observed.** This is the deepest problem in the capture and it is not a content problem.
Over the whole scene area of `01_spawn_shore_fields`: lum mean 186, sd 38, **p1 = 91**, and
**0.00% of pixels below luminance 30**. Reference `0132` — also a bright daylight frame — is mean
159, sd 64, **p1 = 29**, **2.09% below luminance 30**. Our entire image lives in the top 55% of the
value range. Consequences visible in the crops:

- Grass sits on ground at the same value as the ground: darkest 5% inside our grass patch is 102,
  which is exactly our flat side-face palette colour. Reference dune grass p5 = 56, bank grass
  p5 = 41 — the base of every clump goes dark.
- No cast shadow from any flora onto the ground is visible anywhere in the capture, though a
  shadow map is shipped.
- Lit-to-shaded ratio on our sand is exactly **1.00** (one colour for every face orientation).
  On grass terrain it is 1.77 (180.8 top / 102.2 side), applied as a flat palette step with no
  gradient and no corner darkening.

**Accept.**
1. In a daylight capture, **≥ 1.5%** of scene pixels fall below luminance 30 and scene luminance
   sd is **≥ 55**. (Derived from `0132`: 2.09% and 64. The threshold is set below the reference so
   we are not forced to match its exposure, only to have a shadow end at all.)
2. In a 200 px crop of near grass, the darkest 5% of pixels are **≤ 0.45x** the median of that
   crop. (Derived: `0132` dune grass 56/150 = 0.37; `0227` bank grass 41/80 = 0.51; ours 102/181 = 0.56.)
3. At least one flora cast shadow with a readable, tracking silhouette is visible on open ground
   in a capture taken with the sun below 40° elevation. (M4, crop named at review.)
4. Concave corners darken: at a terrain step, the inside corner is **≥ 20%** darker than the flat
   face 8 px away, on both the top and the riser. (M4 + M3 on a named crop; the reference shows
   this everywhere in `0018`.)

**Subsystem.** Lighting and shadows (ambient occlusion baked at meshing, sky light, contact
shadow), with post carrying the curve (see B3). Not a content fix — adding more grass to a frame
with no dark end makes it busier, not better.

**Tier A.** Every other item is judged against a value range we do not currently have. Do this
before or alongside A3/A4 or their effect will not read.

---

### A3. Our surfaces are single flat colours

**Observed.** Our sand region (120,000 px) contains **3 distinct RGB values**, 89.8% of them one
value; M2 = **0.00**, M1 = **100%**. That is not "low detail", it is a constant. Our water region
contains 25 values in 320,000 px, M2 = 0.29, M1 = 82%. Reference: sand M2 = 3.33 / p90 12.8,
M1 = 16%; snow slope M2 = 6.54, M1 = 13%; river water M2 varies 1.0–5.8, M1 = 39–49%. The
compression floor is 0.28, so the reference's surfaces carry an order of magnitude more real
variation than ours carry any.

**Accept.**
1. No ground, rock, sand, snow or water surface in a capture has **M1 > 45%** or **M2 < 1.5** in
   any window. (Derived: worst reference ground region measured is M1 43% / M2 1.76, in a *night*
   frame where everything compresses toward black; daylight reference regions sit at M1 11–16%,
   M2 3.3–6.5. 45%/1.5 is the floor, not the goal.)
2. The variation is **material**, not noise: at 40 m the same surface must still show the same
   family of tones (it must not average out to the flat colour). Test: M2 on the same material at
   3 m and at 40 m in one capture differ by **< 2.5x**.
3. Face-orientation shading is not the only variation present: with the sun disabled, M1 on the
   sand region is still **≤ 60%**.

**Subsystem.** Material palette (per-voxel colour jitter from the deterministic hash, per-material
tone sets) and terrain LOD/tile filtering (the jitter must survive into the coarse rings).

**Tier A.** Combined with A2 this is what makes our frame read as flat-shaded diagram rather than
landscape. It is also the cheapest of the tier-A items: the variation is a per-voxel palette
lookup at meshing time with no per-frame cost.

---

### A4. Ground cover is sparse, monochrome and gridded

**Observed.** In the near grass band, **55.1% of pixels are one of four flat ground/face colours**
— more than half of our "grass field" is bare ground. Colour: saturation mean 0.23, hue sd 20–33.
Reference: `0132` dune grass sat 0.40, hue sd 44; `0209` meadow sat 0.43, hue sd 37; in `0209`
the ground is not visible at all. In the crop (`ours/01` region 300–1500 x 1050–1440) our tufts
sit on a **visible regular lattice** aligned to the terrain grid, all blades the same height, no
taper, and flowers are single coloured cubes.

**Correction to the brief's framing.** Apparent blade *width* is not our problem. Measured median
blade run width: ours 0.66% of frame width, reference `0132` 1.19%. Our blades are, if anything,
thinner on screen. What makes ours read chunky is that each blade is a **uniformly-coloured
hard-edged cuboid with no base occlusion**, and that bare ground shows between them. Do not spend
work shrinking the voxel; spend it on colour, coverage and occlusion.

**Accept.**
1. In a 400 px crop of near ground (0–8 m), **≤ 15%** of pixels are bare ground. (Derived from
   `0209`, where it is effectively 0%, relaxed to allow paths, rock and shoreline.)
2. Within that crop, hue sd **≥ 30** and saturation mean **≥ 0.35**. (Derived: `0209` 37/0.43,
   `0132` 44/0.40; ours 20–33/0.23.)
3. Blade aspect is **≥ 8:1** (height:width) measured on one foreground blade. (Hand-measured on
   the crops, ±20%: reference 9–12:1, ours ~5:1. Reachable at 6.25 cm detail voxels — see §3.)
4. No lattice: in a top-down or shallow-angle capture of open grass, no row or column alignment of
   tuft origins is visible over a 10 m span. (M4)
5. Each tuft carries a colour ramp — the blade tip differs from the blade base by **≥ 15**
   luminance within a single tuft, and neighbouring tufts differ from each other by **≥ 10**
   luminance and **≥ 8°** hue.
6. Flowers read as **≥ 4 distinguishable species** (silhouette, not just colour) in a single
   meadow capture, plus at least one non-flower ground item (mushroom, stone, fern). (`0209`
   shows purple spires, red clusters, yellow heads, white heads and mushrooms in one frame.)

**Subsystem.** Flora placement tiers (density, jittered/blue-noise placement), detail volumes
(blade shape and taper at 6.25 cm), material palette (per-instance and per-voxel colour ramp),
lighting and shadows (base occlusion — see A2).

**Tier A.** `0209` and `0132` are the frames we are being compared to, and in both of them the
ground cover is the foreground subject.

---

### A5. There is no mid-ground

**Observed.** Every reference frame has tree stands as its dominant silhouette between roughly
50 m and 800 m — conifer walls in `0227` and `0209`, palm and pine bands in `0132`, layered forest
in `0150`. Our capture reports `TREES 157` in the HUD and shows **none** in frame: our scene goes
straight from near grass to hazy hills, with nothing between. The distant headland at frame left
is bare.

**Accept.**
1. In a capture framed like `0132`, tree silhouettes are readable at **≥ 400 m** and occupy a
   continuous band along at least **50%** of the horizon where the biome supports forest. (Derived
   from `0132`, where the far shore is continuously treed across the whole frame width.)
2. Trees do not pop: over a 200 m dolly, no tree appears or changes silhouette class within 120 m
   of the camera. (M4 on a two-frame pair.)
3. Stands read at **≥ 3 depth layers** distinguishable by value in one capture (`0150` and `0227`
   both show four).

**Subsystem.** Flora placement tiers (tree density and distance tiering), terrain LOD and tile
filtering (trees must exist in the coarse rings, as impostors if necessary), fog/atmosphere (A7
governs whether they stay separable).

**Tier A.** Without it the scene is a beach with a lawn, not the reference's world.

---

### A6. Water does not read as water

**Observed.** Our water is a flat teal plane, `(122,167,183)` ± a smooth vertical fog gradient:
M2 = 0.29, M1 = 82%, 25 distinct colours in 320,000 px. No specular, no ripple, no depth tint, no
reflection, no foam. At the shoreline, four constant-colour terraces meet along hard stepped
edges: open water `(122,167,183)`, then `(191,198,200)` spanning rows 920–1011 across 2200 px,
then `(203,219,234)` — **exactly our clear/sky colour** — then sand `(229,220,190)`. Each band is
internally flat with zero gradient. (The source of the two shore bands must be localised: they are
geometry, not HUD — they are absent from `02_look_around`, which is the same HUD on empty sky.)

Reference `0227` river water: p1–p99 luminance range **101**, M1 39%, tree reflections dark
against a bright sky reflection, specular glints, ripple structure, and the bank darkens
continuously into the water with no seam. `0055`: reflections plus visible submerged plants.
`0132`: open water shows a ripple field at M2 = 5.83.

**Accept.**
1. Water dynamic range (p1–p99 luminance) across an open-water region is **≥ 55**. (Derived:
   `0055` 64, `0227` 101; ours 39, and our 39 is a fog gradient, not water.)
2. **M1 on water ≤ 55%** and **M2 ≥ 1.5**. (Derived: `0055` M2 1.27 at range, `0132` 5.83 near;
   `0227` M1 39%.)
3. A depth ramp exists: from the waterline to 30 m out, water luminance changes **monotonically**
   by **≥ 25** with no constant-colour band wider than 1% of frame height. This single criterion
   kills the shore terracing.
4. Grazing angles brighten: the water at the far horizon is **≥ 20** luminance brighter than the
   water directly below the camera in the same capture, and the brightening is continuous.
5. No pixel of clear/sky colour appears between the beach and the water at any camera angle.
6. Sun specular is present: in a capture with the sun low behind the water, **≥ 0.1%** of water
   pixels exceed luminance 230.

**Subsystem.** Water pass (depth tint, fresnel, specular, ripple normal), fog/atmosphere (the
shore bands may be a fog or a blend-order defect and must be identified before anything is added
on top).

**Tier A** for 1–5 (depth, shoreline, ripple, fresnel); reflections are **B1**.

---

### A7. Distance is a wipe, not an atmosphere

**Observed.** **43.0% of our far-hill band is exactly `(203,219,234)`** — the clear colour, pixel
for pixel — and 45.1% is within ±4 of it. M2 on our far hills is **0.57**, M1 **62%**: the distant
terrain has been replaced by the fog colour. The surviving silhouettes are smooth low-poly humps
with no voxel stepping at all. Measured contrast decay in our capture is near / mid / far
= sd 35.5 / 18.9 / 9.1 with saturation 0.25 / 0.12 / 0.12.

Reference `0001` decays sd 22.9 / 15.5 / 7.6 with saturation **0.032 / 0.041 / 0.091** — note that
saturation *rises* with distance as terrain takes on the sky's blue, which is aerial perspective,
not a lerp to grey. `0227` decays 35.3 / 20.2 / 6.6 with saturation 0.359 / 0.064 / 0.109. The far
ridge in `0001` still measures M2 = 3.95, six times ours.

**Accept.**
1. No more than **5%** of any capture is within ±4 RGB of the clear/horizon colour outside the sky
   itself. (Ours: 43% of the far band.)
2. Distant terrain keeps structure: **M2 ≥ 2.0** on terrain at 2 km in clear weather. (Derived:
   `0001` far ridge 3.95, compression floor 0.28. Set at 2.0 because our 1 m grid cannot reach
   3.95 honestly — see §3.)
3. Ridges stay separable: two overlapping ridgelines at 1.5 km and 3 km differ by **≥ 10**
   luminance at their shared edge.
4. Fog takes its colour from the sky in the view direction, not from a constant: with the sun low,
   fog toward the sun is **≥ 12** warmer in R−B than fog away from it. (`0037` and `0227` both
   show this strongly.)
5. Distant silhouettes are stepped, not smooth: on a 2 km ridgeline, at least one visible step
   per 3% of frame width. (`0001`, `0245` both show blocky far ridges.)

**Subsystem.** Fog/atmosphere (curve and colour source), terrain LOD and tile filtering (what
survives into the 32 m and 64 m cells), sky/cloud pass (fog colour must sample the sky, so A1 is
a prerequisite for criterion 4).

**Tier A** for criteria 1, 3 and 4 — they are a fog-curve and fog-colour change and cost nothing
per frame. Criteria 2 and 5 are **B4** and are partly bounded by §3.

---

### B1. No reflections

**Observed.** Ours: none. `0055` and `0227` both take most of their character from dark trunk
reflections over a bright sky reflection; in `0227` the reflections alone span the water's 101
luminance range.

**Accept.** Under a sky with cloud and a treed bank, the water shows (a) a fresnel-weighted sky
term that brightens toward grazing angles, and (b) a **darkening beneath vertical geometry that
tracks it as the camera moves** — a mast or trunk near the shore produces a dark column on the
water below it, elongated along the view direction, within 15% of the object's screen width.
Criterion is met by any technique; no per-pixel accuracy required. (M4 on a two-frame pair.)

**Subsystem.** Water pass; post if screen-space.

**Tier B.** Large payoff for anyone who looks at the water, but the scene reads correctly without
it once A6 lands. See §3 for what is actually affordable.

---

### B2. No volumetric light

**Observed.** Ours: none. `0037` god rays through a canyon; `0209` fog banked between conifer
trunks at three depths; `0114` mist filling a valley at night; `0227` haze layered along a river.
Ours has a single linear distance fog with no height term and no scattering.

**Accept.**
1. Fog is height-banded: standing at 3 m, fog density at eye level is **≥ 2x** its density 60 m
   above, visible as a fog layer with a readable top edge against a hillside. (M4)
2. Fog is depth-layered between objects: in a forest capture, trunks at 30 m, 60 m and 120 m sit
   at three distinguishable value levels differing by **≥ 12** luminance each. (Derived from
   `0209` and `0227`.)
3. God rays: with the sun within 20° of the view direction and geometry occluding it, radial
   brightening is visible against the sky. (M4 — `0037`.)

**Subsystem.** Fog/atmosphere; sky/cloud pass (shares the raymarch and the light budget).

**Tier B.** Criterion 2 is most of the value and is the cheapest of the three.

---

### B3. No tone curve, no bloom, no warm/cool light split

**Observed.** Ours clips: **0.89% of scene pixels above luminance 250** against the reference's
**0.17%**, while simultaneously having no shadows (A2) — that is the signature of a linear
pipeline with no shoulder. Our lighting is one neutral ramp: sun-lit and ambient faces differ only
in value, never in hue. Reference frames show warm sunlight against cool blue shadow throughout
(`0132`, `0227`, `0037`).

**Accept.**
1. Highlight clipping (fraction above luminance 250) is **≤ 0.3%** in a daylight capture while
   criterion A2.1 still holds. (Derived: reference 0.17% across three frames.)
2. Sun-lit and shadowed faces of the same material differ in hue, not only value: **≥ 10°** hue
   separation, with shadow biased toward the sky colour.
3. Bloom is present but bounded: a specular glint or the sun disc produces a halo whose radius is
   **≤ 3%** of frame height. (`0227` and `0037` both bloom modestly; nothing in the reference is
   blown out.)

**Subsystem.** Post (tone map, bloom, grade); lighting and shadows (sky-coloured ambient).

**Tier B.** It converts "correct" into "good", and it is the cheapest way to buy back the contrast
that A2 needs — but a tone curve over a scene with no occlusion just makes flat things darker, so
it must land after or with A2.

---

### B4. Distant terrain has no micro-relief; material boundaries are hard lines

**Observed.** Two symptoms of one cause — nothing varies inside a tile and nothing blends between
tiles or biomes. Far hills M2 = 0.57 against the reference's 3.95. The sand/grass boundary in our
foreground crop is a single hard palette edge with no interleaving; the reference's dune in `0132`
has grass thinning into sand over several metres with sand showing between clumps.

**Accept.**
1. A7.2 and A7.5 (M2 ≥ 2.0 and visible stepping on a 2 km ridge).
2. No material boundary on open ground is a continuous straight or tile-aligned edge longer than
   **2%** of frame width; boundaries interleave over **≥ 2 m** of ground. (M4 + M1 on the crop.)

**Subsystem.** Terrain LOD and tile filtering, material palette, flora placement tiers.

**Tier B.** Real, but only visible once A3 and A7 have landed.

---

### C1. Water has no surface or subsurface population

**Observed.** `0055` shows lily pads floating, submerged plants legible through the surface, and
bright float debris; `0227` shows shore rocks and reeds standing in water. Ours: an empty plane.

**Accept.** In a lake capture, floating flora is present at **≥ 0.05 instances/m²** within 20 m of
shore, and at least one submerged item is legible through the surface at 2 m depth. Foam or a
tone change of **≥ 15** luminance exists within 1 m of the waterline.

**Subsystem.** Water pass, flora placement tiers, detail volumes.

**Tier C.** Charming, bounded, and irrelevant until A6 makes the water a surface at all.

---

### C2. Sky states, sun disc, and living detail

**Observed.** The reference carries a full day cycle (`0037` sunset, `0114` night, `0150` dusk,
`0209` night), a visible sun, and birds (`0209`). Ours has one lighting state.

**Accept.** At least dawn/day/dusk/night sky and fog states exist and each satisfies A1.1 with
state-appropriate numbers; the sun disc is visible and bloomed per B3.3.

**Subsystem.** Sky/cloud pass, lighting and shadows, post.

**Tier C.** Each state multiplies the work of validating every other item. Do it once the daylight
frame is settled.

---

## 3. Honest feasibility

Four of the items above cannot be met the way the reference meets them. Named here with the
cheapest substitute, so nothing is dropped silently.

**1. Metre-scale micro-voxel detail at kilometres (A7.2, A7.5, B4).**
*Out of reach.* The reference is sub-10 cm voxels everywhere with an LOD that preserves blockiness;
our authoritative grid is 1 m and our outer rings are 32 m and 64 m cells at 4–8 km. At 8 km, one
64 m cell is a few pixels; there is no representation of a 6.25 cm feature to preserve. Matching
the reference's far-ridge M2 of 3.95 by carrying real geometry is not available in 16.6 ms with
332 resident tiles and 57 MB of tile geometry.
*Cheapest approximation:* keep the **statistical signature**, not the geometry. (a) Carry the
per-voxel material jitter of A3 into the coarse-ring shader as a screen-stable hash so far terrain
measures M2 ≥ 2.0 instead of 0.57 — a shader term, no new geometry, no new memory. (b) Displace
the top silhouette edge of coarse tiles by the same deterministic hash so 2 km ridgelines step
rather than curve (A7.5) — a vertex-stage offset on the ridge row only. Accept M2 ≥ 2.0 rather
than the reference's 3.95, and say so.

**2. Mirror reflections (B1).**
*Out of reach as mirrors.* No reflection pass exists; a planar re-render doubles the terrain pass
that already dominates the 23.8 ms flight frame, and full screen-space tracing is not affordable
alongside a cloud raymarch that is itself unmeasured.
*Cheapest approximation, in order of value per cost:* (i) **fresnel + sky-colour + sun specular +
depth tint** — no reflection pass at all, and it delivers A6.1, A6.2, A6.4, A6.6 and most of what
a viewer names as "the water looks right"; (ii) a **single vertical screen-space ray, quarter
resolution, blurred** — enough to produce the tracking dark column under trunks that B1 actually
asks for, at roughly the cost of one extra quarter-res pass; (iii) planar reflection only if a
frame budget appears. Ship (i) in tier A, evaluate (ii) as tier B with a hard ms cap.

**3. Volumetric clouds and god rays at full resolution (A1, B2.3).**
*Out of reach at full res.* The cloud pass in the open PR is unmeasured on the device, and a
full-resolution raymarch plus our existing 23.8 ms flight frame will not fit.
*Cheapest approximation:* quarter-resolution raymarch with temporal reprojection and a **declared
millisecond cap**, plus a static cloud-layer fallback (a lit, scrolling 2D layer on the dome) that
still satisfies A1.2 and A1.3. Measure on the OnePlus 13 before either is enabled by default; A1
must not be the reason the standing frame leaves 16.6 ms.

**4. Reference-width grass blades (A4.3).**
*Partly out of reach.* Our finest detail volume is 6.25 cm. Reference blades are roughly 2–4x
finer than the 12.5 cm we ship, and 6.25 cm gets us halfway. Going to 3.125 cm doubles flora voxel
counts against a frame that is already CPU/streaming-bound at 42 fps in flight.
*Cheapest approximation:* 6.25 cm blades with **taper** (narrow to one voxel at the tip) and a
per-blade colour ramp. Aspect ratio, not absolute width, is what makes a blade read as thin — the
measurement in A4 shows our apparent blade width is already narrower than the reference's in the
comparable frame. Accept ≥ 8:1 aspect at 6.25 cm; do not pursue 3.125 cm.

**One thing that is *not* a feasibility problem.** A2 and A3 — the missing dark end and the flat
surfaces — are the two largest contributors to the gap and are both nearly free: ambient occlusion
baked into vertex colours at meshing time, and a per-voxel palette lookup. Neither costs per-frame
time. If the frame budget forces a choice, these are the items that survive it.

---

## 4. Priority

**Tier A — required for the scene to read as the same kind of thing.**
Each of these is something a viewer names as *absent*, not as *worse*. Ordered by perceived gap
closed per unit of work.

| | Item | Why it is first |
|---|---|---|
| 1 | **A2** value range / AO / shadow | Largest measured divergence (p1 91 vs 29; 0% vs 2.1% dark), near-zero per-frame cost, and every other item is judged against it |
| 2 | **A3** material variation | Our sand is literally one colour; fixed at meshing time, no runtime cost |
| 3 | **A1** sky and clouds | Largest blank area of the frame; code already exists in a PR |
| 4 | **A4** ground cover density and colour | The subject of both closest reference frames |
| 5 | **A7** (1,3,4) fog curve and fog colour | 43% of our distance is currently the clear colour; a curve change |
| 6 | **A6** (1–5) water depth, shoreline, ripple, fresnel | Removes the four-band shoreline artefact, which is the most obviously wrong thing in the capture |
| 7 | **A5** mid-ground tree stands | Highest work of the tier — placement, LOD and streaming — but the scene has no middle distance without it |

**Tier B — makes it look good rather than merely correct.**
These are things a viewer names as *worse*, not absent. B3 (tone/bloom/warm-cool) first, because it
multiplies the value of everything in tier A for the least work; then B2.2 (depth-layered fog),
B1 approximation (i)/(ii), B4, B2.1, B2.3.

**Tier C — polish that can wait.**
C1 (water population and foam), C2 (sky states, sun disc, birds), extended flower and ground-item
species beyond A4.6's minimum. Deferred because each depends on a tier-A subsystem being settled
first, and each multiplies validation cost across every other item.

---

## 5. What not to chase

- **The reference's frame rate.** Its counter reads 16–70 fps and drops to the mid-teens in the
  night scenes. We hold 60 fps standing and 42 fps flying, on a phone, with no thermal throttling
  at 27 °C. Do not trade our frame stability for its detail. Where the two conflict, §3 names the
  substitute.
- **The night frames as a fidelity target.** `0114`, `0150` and `0209` are the reference at its
  slowest and darkest, and darkness is hiding a lot of its own LOD. Take from them what they show
  clearly — ground-cover density and colour variety in `0209`, layered stands in `0150`, fog
  banking in `0114` — and validate every criterion against the **daylight** frames `0132` and
  `0227`.
- **`0227` and `0245` as scenes.** They are the tail of the video: a settings menu, a render-
  distance comparison, an end card and a player-built roof. `0227`'s water and forest are useful
  evidence; its UI, its canoe, and `0245` entirely are video furniture.
- **YouTube compression as detail.** Measured codec floor on flat reference regions is M2 ≤ 0.28.
  Per-pixel shimmer in the reference frames is not a material to reproduce.
- **The reference's exact palette.** It is warmer and more saturated than ours. Every criterion
  above is written on *structure* — value range, variance, coverage, separation — not on hue
  targets, so that our palette can stay ours.
- **Digging, carving, the hotbar, the canoe.** Gameplay surface, not visual fidelity.
- **`0037`'s god-ray intensity.** It is the video's showpiece shot, taken under a player-built
  structure at sunset with the sun almost directly in frame. B2.3 asks only that rays exist under
  those conditions; do not tune the general case to that frame.
- **Chasing 3.125 cm flora, planar reflections, or full-resolution cloud raymarching** before the
  free items (A2, A3) have landed and been measured on the device. They are the expensive answers
  to a gap whose largest components are cheap.
