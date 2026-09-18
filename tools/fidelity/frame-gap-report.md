# Frame-gap report — ours (10_after_atmos_device) vs ten reference frames

Toolkit: `frame-gap-metrics.py` (new) + `sweep_all.json` (11 images × 5 regions) +
`sweep_trend.json` (3 older ours frames). Spec: `fidelity-spec.md`.
Example run (DoD #1): `python3 frame-gap-metrics.py IMAGE [...] --region near|mid|distant|water|sky`
e.g. `frame-gap-metrics.py ours/10_after_atmos_device.png --region near` →
`near covw=0.256 g=0.406 bl=0.0651 ... sat=0.21/0.33 edge=0.0588 ... pm=583`
(full row in §6). Regions are semantic bands, not identical pixels: ours source
(3168×1440, HUD rows <440 excluded) and ref source (1600×900) each get bands that
land on sky/horizon/middle/water/foreground; exact rects are printed in every
JSON row and in the script docstring. All metrics are scale-free (fractions,
per-kpx, fixed 800 px analysis width, M2 at 1600-equivalent width).

## 1. Player-chrome audit (DoD #2: excluded, effect quantified)

Reference frames carry: title line (rows 0–40), red progress bar (~836–846, full
width), hotbar (~775–838, x 430–1070), left buttons (x<95, y 650–800), right icons
(x>1400, y 700–800), bottom controls (>855). Handling: title/progress/controls are
outside every region by construction (ref scene = rows 40–825); hotbar/buttons/icons
are masked (dropped from colour stats, median-filled for spatial stats).
`--chrome-test` on `0132` (mask off minus on): only `near` overlaps chrome
(mask_drop 6.9%); deltas are sat +0.007, spreadL +0.09, blade +0.013 (worst, 13%
relative), edge +0.008 (8%), buckets −0.11 (5%), cov_modal −0.0001. Mid, water,
distant, sky: zero overlap, deltas exactly 0. All ≪ the 1.5–37× gaps below.
Non-chrome confounds, handled by exclusion (not masking): `0018` carries a video
title card across the distant/mid bands → excluded from distant+mid medians (kept
for near); `0245` is settings UI + end card → excluded from ALL medians (the spec
itself: "Not a target. Video furniture"); `0227` near includes the canoe (noted,
kept — it reads as smooth dark area and does not flatter us).

## 2. Claim-by-claim verdicts (DoD #4)

Reference medians below use applicable subsets (content must exist in the band);
n and set listed each time. Full 11×5 table: `sweep_all.json` (+ §6 summary).

1. **Ground cover: blades vs cube props on bare plane — CONFIRMED vs 0132.**
   Near smooth-ground fraction ours 0.406 vs 0132 0.246 (1.7× more bare plane);
   blade (thin-vertical) fraction ours 0.065 vs 0132 0.104 (1.6× deficit).
   Caveat: windowed colour coverage does NOT confirm it (ours 0.256 vs 0132
   0.134 — our flat-shaded cube faces fragment into as many //13 buckets as
   reference texture). The bareness shows at structural scale, not colour-bucket
   scale: thresholds must be structural. Cuboid-vs-tapered shape itself has no
   metric yet (named in §5).
2. **Trees everywhere vs none near camera — CONFIRMED by proxy; direct metric missing.**
   HUD reports TREES 157, none visible in frame (M4 observation). Toolkit proxy:
   ours-mid is the emptiest mid-band of all 11 frames by buckets/kpx (0.13 vs
   0132 2.34, 18×; §3 row 6). There is no tree/trunk-presence metric in the
   toolkit — `trunk` was tested and DROPPED (§5). An implementation brief for
   trees needs a new detector, not these numbers.
3. **Terracing + within-material spread — SPLIT: luminance half CLOSED, hue half wide open.**
   Near spreadL ours 10.74 vs vegetated median 11.08 (1.0× — the atmos build fixed
   this; the old ground-level sand was 0.63). Near spreadH (hue) ours 3.6° vs
   vegetated median 41.8° (**11.5×**); fine-terracing (thin-horizontal) ours 0.110
   vs 0132 0.231 (2.1×). Build colour variety in hue, not luminance.
4. **Mid-distance filled vs empty water — CONFIRMED vs daylight refs.**
   Mid edge ours 0.033 vs 0132 0.099 (3×); mid buckets/kpx ours 0.13 vs
   8-frame median 0.52 (**4×**), vs 0132 2.34 (18×). Not separable vs dusk/night
   mids (0150 0.0085, 0114 0.0043 — haze/darkness flattens everything), so the
   threshold is anchored on daylight (§3).
5. **Distant slopes forest+snow vs flat silhouettes — CONFIRMED.**
   Distant buckets/kpx 0.11 vs 0.46 (**4×**), edge 0.009 vs 0.023 (**2.5×**),
   hue-spread 1.5° vs forested median 25° (**17×**). m2 does NOT confirm (ours 8.21
   vs 0132 0.96 — our hard silhouette edges inflate block-std; §5).
6. **Tiling ripple vs smooth water — CONFIRMED, and it is regularity, not amplitude.**
   Detrended-spectral peakmed: water ours 25033 vs water-bearing median 1117
   (**22×**); mid-band 187308 vs 5044 (**37×**). Detrended amplitude is at parity
   (ours rms 14.9 vs 0055 14.7, 0132 40.6) — do not threshold amplitude.
   0132 itself scores 10268 (real surf/depth bands), hence the two-tier threshold.
7. **Desaturated palette — CONFIRMED with refinement: the deficit is saturated accents.**
   Near sat_mean ours 0.21 vs vegetated 0.31 (1.5×), sat_p90 0.33 vs 0.48 (1.5×);
   0132/0150/0209 all put 0.44–0.56 at p90 (flowers-accents) while our base greens
   are comparable. Sky/distant/mid saturation is roughly parity — the palette claim
   holds for the foreground, not globally.

## 3. Ranked gap table (DoD #5): ours, reference median, ratio, threshold

Ratio = larger:smaller, direction shown. "refs" = applicable subset (see §1).

| gap | dir | metric (region) | ours | ref median (n, set) | threshold to build against |
|---|---|---|---|---|---|
| 37× | ours:ref | spectral peakmed (mid) | 187308 | 5044 (8, all excl 0018/0245) | ≤15000, goal ≤5000 |
| 22× | ours:ref | spectral peakmed (water) | 25033 | 1117 (7, water-bearing) | ≤11000 (0132 parity), goal ≤3000 |
| 17× | ref:ours | hue spread ° (distant) | 1.5 | 25.0 (3, forested) | ≥15 |
| 11.5× | ref:ours | hue spread ° (near) | 3.6 | 41.8 (4, vegetated) | ≥12 (0132 parity), stretch ≥25 |
| 4.0× | ref:ours | buckets/kpx (distant) | 0.11 | 0.46 (8) | ≥0.45 |
| 4.0× | ref:ours | buckets/kpx (mid) | 0.13 | 0.52 (8) | ≥0.5 |
| 2.5× | ref:ours | edge density (distant) | 0.009 | 0.023 (8) | ≥0.02 |
| 2.1× | ref:ours | thin-horizontal (near) | 0.110 | 0.231 (0132 only) | ≥0.20 |
| 1.7× | ours:ref | smooth-ground frac (near) | 0.406 | 0.246 (0132 only) | ≤0.27 |
| 1.6× | ref:ours | blade frac (near) | 0.065 | 0.104 (0132 only) | ≥0.10 |
| 1.5× | ref:ours | sat mean (near) | 0.21 | 0.31 (4, vegetated) | ≥0.30 |
| 1.5× | ref:ours | sat p90 (near) | 0.33 | 0.48 (4, vegetated) | ≥0.45 |
| 1.0× | — | lum spread (near) | 10.74 | 11.08 (4) | ≥9 (regression guard — already closed) |
| 1.0× | — | spectral peakmed (sky) | 2084 | 2187 (9) | watch only (cloud-layer tiling suspect, §4) |

## 4. Dominant gaps (DoD #6) — three, in build order

1. **Regular banding + empty middle distance (37×/22× spectral, 4× variety).**
   The water/mid reads as a tiling ripple over an empty plane. Fix order matters:
   content first (A5 stands), then ripple irregularity — thresholds above force both.
2. **Missing hue variety (11–17×).** Luminance spread already matches the reference;
   every remaining colour gap is hue: near cover (3.6° vs 42°) and distant forest
   (1.5° vs 25°). Per-voxel/per-instance hue jitter, not more luminance work.
3. **Ground-cover geometry + accents (1.5–1.7× each, compounding).** More bare plane,
   fewer blades, no saturated accents vs 0132. Small individually; together they are
   the foreground subject of both closest reference frames.

Notes: ours-sky spectral (2084 ≈ parity) still shows a 100 px-wavelength peak worth
one implementer-glance at the cloud layer for tiling. Trend (same rects): water
regularity 07→10 rose 14768→25033 (1.7× worse); near blade 0.054→0.065, sat flat,
hue spread 11.6°→3.6° (worse). Old ground-level sand rects (dist/03: covw 0.53,
spreadL 0.6) vs newest grass (0.26/10.7) is a camera/content change, not a pure
rendering delta. Spectral also fires on smooth nonlinear fog gradients (dist-mid
sand: 760k), so it is a *regularity* detector — always compare against refs, never
against zero.

## 5. Weak/confounded metrics — dropped, not reported as fact (DoD #7)

- **D1 global modal share**: polarity inverted by gradients + codec noise (ours-near
  0.10 below most refs). Replaced by windowed `cov_win`; which itself is ambiguous
  (ours below vegetated median) — coverage thresholds must be structural (`ground`).
- **D2 exact-colour distinct/kpx**: YouTube noise dominates (0132-near 346 vs ours 10).
  Replaced by `buckets_per_kpx`.
- **D3 column runs of non-modal pixels** (as briefed): modal bucket is 4–36% of any
  region, so runs measure blob size (ours 34/100px vs refs 10–32, no separation).
  Replaced by `blade`/`ground` ridge fractions.
- **D4 plain autocorrelation peak** (as briefed): every image peaks at minimum lag —
  measures smoothness, not tiling. Replaced by detrended-FFT `peakmed`/`frachi`
  (tested 5–187× separation).
- **`trunk`** (coarse thin-vertical): fires on our terrace edges as much as on palm
  trunks (ours 0.110 vs 0132 0.102). Reported, not ranked.
- **m2 on distant**: our silhouette edges inflate it past the reference (8.21 vs 0.96).
  Edge density + buckets carry the distant signal instead.
- **Trees**: no presence metric exists in this toolkit (see claim 2).

## 6. Per-region numbers (DoD #3 summary; full precision in sweep_all.json)

Ours newest (`10_after`, sky/distant/mid/water/near):
`sat .13/.13/.13/.20/.21 · edge .009/.009/.033/.005/.059 · m2 9.1/8.2/3.5/6.0/8.8 ·
bpk .13/.11/.13/.07/.33 · peakmed 2084/2363/187308/25033/583 · spreadL 10.9/14.7/15.0/15.5/10.7 ·
spreadH 1.0/1.5/4.2/1.3/3.6 · blade .036/.031/.004/.003/.065 · ground .104/.148/.522/.304/.406 (trunk n/a for sky: band too short for coarse structure)`
Closest refs for the same rows — 0132:
`sat .21/.07/.09/.13/.31 · edge .022/.031/.099/.119/.105 · bpk .98/1.33/2.34/2.51/2.24 ·
peakmed 555/20054/4444/10268/687 · spreadL 5.7/9.2/12.2/13.0/13.9 ·
spreadH 1.3/n-a/n-a/n-a/12.6 · blade .028/.038/.043/.055/.104 · ground .842/.817/.377/.283/.246`
— 0150 (near/mid/distant): `sat .31/.34/.40 · edge .028/.009/.003 · bpk .20/.15/.18 ·
spreadH 23/20/26 · spreadL 12.5/5.5/7.5` — 0209-near: `sat .37/.56 · spreadH 63.6 ·
blade .054 · ground .626` — 0227-distant: `edge .052 · bpk 1.45 · spreadH 16.8`.
All ten refs × five regions with rects, n, mask fractions: `sweep_all.json`
(`sweep_all.txt` holds the same run as text). Older ours: `sweep_trend.json`.
