#!/usr/bin/env python3
"""voxel-step-metric.py -- decide whether distant terrain reads as voxels.

Measurement method for the "voxel-stepped distant terrain" acceptance item
(spec A7.5 / B4 style; follows the measurement protocol of fidelity-spec.md
section 0: compare a named region of our capture against a named reference
region with a stated numeric metric).

Reads an image + an optional crop region, reports metrics that separate
"stepped voxel terrain" (flat tops at quantised heights, vertical walls,
flat shading -> many hard shading breaks a few px apart) from "smooth mesh"
(gradient faces, fog wipe -> almost no hard breaks, few colours).

Kept (they separate; see CALIBRATION below):
  K1 edge_h  - fraction of horizontal neighbour pairs with |dLum| >= 8.
               Voxel walls/shading breaks make hard steps; smooth fogged
               mesh makes ~none.  STEPPED iff edge_h >= 0.020.
  K2 distinct_per_kpx - distinct RGB values per 1000 region pixels.
               Voxel relief carries many flat-shaded facets; smooth mesh is
               a handful of fogged colours.  STEPPED iff >= 18.

Supporting (reported, not part of the verdict):
  S1 m2       - median std of Rec.709 luminance over 8x8 blocks (spec M2 on
               the region, full-res; spec acceptance for 2 km terrain is
               M2 >= 2.0, see A7.2/B4).
  The gradient-histogram bands, run-length stats, modal shares,
  gradient-orientation shares and largest-region share are the
  tested-and-DROPPED candidates. They are still computed so a future change
  can re-check them, but they do NOT feed the verdict, for reasons in
  DROPPED below.

DROPPED (computed, reported, rejected for acceptance):
  D1 exact run-length: separates with INVERTED polarity (ours mean run 9.2
     vs reference 1.7-2.2). Our fog wipe is *more* exactly-constant than the
     reference, and YouTube compression breaks the reference's exact runs,
     so "long runs = voxels" is false on real data. Worse, flat-shaded
     voxels rendered without compression *would* make long exact runs, so
     this metric would punish the rebuild for succeeding.
  D2 gradient-orientation axis/diag share: inverted. Our few hard edges are
     horizontal fog strata (axis_share 0.69-0.90); reference voxel relief
     throws hard edges at all orientations (axis 0.35-0.40, diag 0.39-0.46).
     At this scale "axis-aligned" describes the smooth defect, not voxels.
  D3 modal-colour / largest-single-colour-region share: inverted. Blown snow
     and sky in the reference connect into larger flat regions (0.14-0.21
     of region) than anything in our frames (0.03-0.06).
  D4 skyline staircase (risers/treads along the terrain-sky boundary): the
     direct measure of A7.5 in principle, but segmentation is fragile --
     clouds behind the ridge, HUD boxes, and title text all confound an
     automatic sky/terrain split. Covered by --skyline as advisory output
     (no threshold, "n/a" when degenerate); interior edge density (K1)
     carries the acceptance signal instead.

CALIBRATION (terrain-only regions, luminance = Rec.709):
  region                                         edge_h   /kpx    m2
  ours/dist-zoom.png  crop 0,60,1800,240          0.0023    6.8  0.59
  ours/dist.png  crop 1400,480,3168,660           0.0026    9.3  3.62*
  ours/03_clouds_low_device.png (same crop)       0.0020   10.8  3.25*
  ref/0001_mountains_snow_conifers 200,180,1400,450  0.0402 34.8 1.24
  ref/0018_snow_detail_terracing 600,250,1400,550 0.0975   52.9  5.31
  *m2 inflated by cloud tops/waterline inside the looser full-frame crop;
   dist-zoom (the 1:1 ridge crop) is the authoritative ours sample at 0.59,
   matching the spec's far-hill M2 = 0.57. Thresholds rest on K1/K2 only.
Thresholds: EDGE_H >= 0.020 (~8x above best ours 0.0026, 2.0x below worst
  reference 0.0402); DISTINCT_PER_KPX >= 18 (1.7x above best ours 10.8, 1.9x
  below worst reference 34.8). Overall verdict STEPPED iff both pass.
Caveats: verdict is valid ONLY for terrain-bearing regions (exclude sky,
  water, HUD/UI by crop -- reference sky alone scores edge_h ~0.035 from
  codec noise + overlay text). Ours frames are 2x reference linear
  resolution; edge fractions and per-kpx counts are scale-free, run lengths
  are reported in px and per 100 px width -- compare like-for-like crops.

Usage:
  voxel-step-metric.py IMAGE [--crop X0,Y0,X1,Y1] [--json-only]
                             [--skyline [--sky R,G,B]] [--check stepped]
Exit code is 0 on successful measurement, 1 if --check names a verdict that
is not met, 2 on usage/IO errors.
Needs Pillow; numpy is optional (pure-python fallback is slower and leaves
m2/orientation fields null).
"""

import argparse
import json
import math
import sys
from collections import Counter, deque

EDGE_T = 8            # hard-step threshold on Rec.709 luminance
EDGE_H_PASS = 0.020   # K1 threshold
DPK_PASS = 18.0       # K2 threshold

try:
    from PIL import Image
except ImportError:
    sys.stderr.write("error: Pillow (PIL) is required\n")
    sys.exit(2)

try:
    import numpy as np
    HAVE_NP = True
except ImportError:
    HAVE_NP = False


# ---------------------------------------------------------------- numpy path

def lum_np(a):
    return 0.2126 * a[:, :, 0] + 0.7152 * a[:, :, 1] + 0.0722 * a[:, :, 2]


def measure_np(a):
    H, W, _ = a.shape
    L = lum_np(a.astype(float))
    gx = np.abs(np.diff(L, axis=1))          # H x (W-1)
    gy = np.abs(np.diff(L, axis=0))          # (H-1) x W
    edge_h = float((gx >= EDGE_T).mean())
    edge_v = float((gy >= EDGE_T).mean())
    edge = float((gx.sum() * 0 + (gx >= EDGE_T).sum() + (gy >= EDGE_T).sum())
                 / (gx.size + gy.size))
    # gradient-magnitude histogram bands (horizontal pairs)
    tot = gx.size
    flat0 = float((gx == 0).mean())
    small = float(((gx >= 1) & (gx <= 2)).mean())
    mid = float(((gx >= 3) & (gx <= 7)).mean())
    hard = edge_h
    # run lengths of exactly-equal colours along rows
    same = (np.abs(np.diff(a.astype(np.int16), axis=1)).max(axis=2) == 0)
    n_same = int(same.sum())
    mean_run = float(H * W / (tot - n_same + H))
    runs = []
    for y in range(0, H, max(1, H // 25)):
        row = same[y]
        ln = 1
        for s in row:
            if s:
                ln += 1
            else:
                runs.append(ln)
                ln = 1
        runs.append(ln)
    runs = np.array(runs, dtype=float)
    # orientation of hard edges (interior cells)
    gxc = gx[:-1, :]
    gyc = gy[:, :-1]
    hardm = np.maximum(gxc, gyc) >= EDGE_T
    nh = int(hardm.sum())
    axis = float(((np.minimum(gxc, gyc) <= 2) & hardm).sum() / nh) if nh else None
    diag = float(((gxc >= 4) & (gyc >= 4) & hardm).sum() / nh) if nh else None
    # distinct colours
    flat = a.reshape(-1, 3)
    distinct = len(np.unique(flat, axis=0))
    # modal shares
    c = Counter(map(tuple, flat.tolist()))
    modal_exact = c.most_common(1)[0][1] / len(flat)
    q = (flat // 13).tolist()
    modal_q6 = Counter(map(tuple, q)).most_common(1)[0][1] / len(flat)
    # M2: median 8x8 block std
    H2, W2 = H // 8 * 8, W // 8 * 8
    b = L[:H2, :W2].reshape(H2 // 8, 8, W2 // 8, 8)
    m2 = float(np.median(b.std(axis=(1, 3))))
    # largest exact-colour 4-connected region (downsampled for speed)
    big = biggest_region_np(a)
    return dict(edge_h=edge_h, edge_v=edge_v, edge=edge,
                grad_flat0=flat0, grad_small12=small, grad_mid37=mid,
                run_mean=mean_run,
                run_frac_len1=float((runs == 1).mean()),
                run_frac_len_ge8=float((runs >= 8).mean()),
                run_max=int(runs.max()), run_n=int(len(runs)),
                axis_share=axis, diag_share=diag, hard_cells=nh,
                distinct=distinct, modal_exact=modal_exact,
                modal_q6=modal_q6, m2=m2, biggest_region=big)


def biggest_region_np(a, maxw=320):
    h, w, _ = a.shape
    if w > maxw:
        import PIL.Image as I  # noqa
        im = Image.fromarray(a)
        im = im.resize((maxw, max(1, round(h * maxw / w))), Image.NEAREST)
        s = np.asarray(im)
    else:
        s = a
    H, W, _ = s.shape
    lab = np.full((H, W), -1, dtype=np.int32)
    best = 0
    cur = 0
    sv = s.reshape(-1, 3)
    for y in range(H):
        for x in range(W):
            if lab[y, x] >= 0:
                continue
            col0, col1, col2 = (int(sv[y * W + x, 0]), int(sv[y * W + x, 1]),
                                int(sv[y * W + x, 2]))
            q = deque([(y, x)])
            lab[y, x] = cur
            n = 0
            while q:
                cy, cx = q.popleft()
                n += 1
                for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                    ny, nx = cy + dy, cx + dx
                    if 0 <= ny < H and 0 <= nx < W and lab[ny, nx] < 0:
                        o = (ny * W + nx) * 1
                        if (int(s[ny, nx, 0]) == col0 and
                                int(s[ny, nx, 1]) == col1 and
                                int(s[ny, nx, 2]) == col2):
                            lab[ny, nx] = cur
                            q.append((ny, nx))
            best = max(best, n)
            cur += 1
    return best / (H * W)


# ---------------------------------------------------------- pure-python path

def measure_pure(px, W, H):
    def rl(r, g, b):
        return 0.2126 * r + 0.7152 * g + 0.0722 * b
    lum = [rl(*p) for p in px]
    tot = H * (W - 1)
    he = sum(1 for y in range(H) for x in range(W - 1)
             if abs(lum[y * W + x + 1] - lum[y * W + x]) >= EDGE_T)
    ve = sum(1 for y in range(H - 1) for x in range(W)
             if abs(lum[(y + 1) * W + x] - lum[y * W + x]) >= EDGE_T)
    edge_h = he / tot
    edge_v = ve / (H - 1) / W
    f0 = sum(1 for y in range(H) for x in range(W - 1)
             if lum[y * W + x + 1] == lum[y * W + x]) / tot
    distinct = len(set(px))
    c = Counter(px)
    modal_exact = c.most_common(1)[0][1] / len(px)
    q = Counter((r // 13, g // 13, b // 13) for r, g, b in px)
    modal_q6 = q.most_common(1)[0][1] / len(px)
    # exact runs, sampled rows
    step = max(1, H // 25)
    runs = []
    for y in range(0, H, step):
        ln = 1
        for x in range(W - 1):
            if px[y * W + x + 1] == px[y * W + x]:
                ln += 1
            else:
                runs.append(ln)
                ln = 1
        runs.append(ln)
    return dict(edge_h=edge_h, edge_v=edge_v,
                edge=(he + ve) / (tot + (H - 1) * W),
                grad_flat0=f0, grad_small12=None, grad_mid37=None,
                run_mean=(H * W) / (tot - sum(
                    1 for y in range(H) for x in range(W - 1)
                    if px[y * W + x + 1] == px[y * W + x]) + H),
                run_frac_len1=sum(1 for r in runs if r == 1) / len(runs),
                run_frac_len_ge8=sum(1 for r in runs if r >= 8) / len(runs),
                run_max=max(runs), run_n=len(runs),
                axis_share=None, diag_share=None, hard_cells=None,
                distinct=distinct, modal_exact=modal_exact,
                modal_q6=modal_q6, m2=None, biggest_region=None)


# ------------------------------------------------------------------ skyline

def skyline_np(a, sky=None, tol=14, min_run=4):
    """Advisory staircase analysis. Returns dict or {'degenerate': reason}."""
    H, W, _ = a.shape
    arr = a.astype(int)
    if sky is None:
        sky = np.median(arr[:max(1, min(8, H))].reshape(-1, 3), axis=0)
    else:
        sky = np.array(sky, dtype=float)
    nonsky = (np.abs(arr - sky).max(axis=2) > tol)
    cs = np.concatenate([np.zeros((1, W), int), nonsky.cumsum(axis=0)], axis=0)
    win = cs[min_run:, :] - cs[:-min_run, :]
    ys = np.full(W, H - 1)
    for x in range(W):
        hit = np.nonzero(win[:, x] == min_run)[0]
        if len(hit):
            ys[x] = hit[0]
    pinned = float(((ys == 0) | (ys == H - 1)).mean())
    if pinned > 0.90:
        return {"degenerate": "skyline pinned to region edge on "
                "%.0f%% of columns; sky/terrain split failed" % (pinned * 100),
                "pinned_frac": pinned}
    step = max(1, round(W / 400))
    y = ys[::step].astype(float)
    dy = np.diff(y)
    n = len(dy)
    tl = []
    ln = 1
    for i in range(1, len(y)):
        if y[i] == y[i - 1]:
            ln += 1
        else:
            tl.append(ln)
            ln = 1
    tl.append(ln)
    tl = np.array(tl, dtype=float) * step
    return {"sky": [round(float(v), 1) for v in sky],
            "subsample_step": step,
            "horiz_frac": float((dy == 0).mean()),
            "diag_frac": float((np.abs(dy) == 1).mean()),
            "riser_frac": float((np.abs(dy) >= 2).mean()),
            "risers_per_100px": float((np.abs(dy) >= 2).sum() / (len(y) * step) * 100),
            "tread_median_px": float(np.median(tl)),
            "tread_mean_px": float(tl.mean()),
            "pinned_frac": pinned}


# ---------------------------------------------------------------------- main

def parse_crop(s, W, H):
    try:
        x0, y0, x1, y1 = [int(v) for v in s.split(",")]
    except ValueError:
        raise SystemExit("error: --crop must be X0,Y0,X1,Y1")
    x0 = max(0, min(W, x0))
    x1 = max(0, min(W, x1))
    y0 = max(0, min(H, y0))
    y1 = max(0, min(H, y1))
    if x1 <= x0 or y1 <= y0:
        raise SystemExit("error: empty --crop region")
    return x0, y0, x1, y1


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Voxel-step metric: decide whether distant terrain reads "
                    "as stepped voxels (STEPPED) or smooth mesh (SMOOTH). "
                    "Verdict is valid only for terrain-bearing crop regions.",
        epilog="Calibration crops (input-image pixels): "
               "ours/dist-zoom.png --crop 0,60,1800,240; "
               "ours/dist.png and ours/03_clouds_low_device.png "
               "--crop 1400,480,3168,660; "
               "reference/0001_mountains_snow_conifers.png "
               "--crop 200,180,1400,450; "
               "reference/0018_snow_detail_terracing.png "
               "--crop 600,250,1400,550.")
    ap.add_argument("image", help="input image path")
    ap.add_argument("--crop", default=None, metavar="X0,Y0,X1,Y1",
                    help="region in input-image pixels (default: whole image)")
    ap.add_argument("--json-only", action="store_true",
                    help="print only the JSON result object")
    ap.add_argument("--skyline", action="store_true",
                    help="also run advisory skyline staircase analysis")
    ap.add_argument("--sky", default=None, metavar="R,G,B",
                    help="sky colour for --skyline (default: median of top rows)")
    ap.add_argument("--check", default=None, metavar="STEPPED|SMOOTH",
                    help="exit 1 unless the verdict equals this value")
    args = ap.parse_args(argv)

    try:
        im = Image.open(args.image).convert("RGB")
    except FileNotFoundError:
        sys.stderr.write("error: no such image: %s\n" % args.image)
        return 2
    except Exception as e:
        sys.stderr.write("error: cannot read %s: %s\n" % (args.image, e))
        return 2
    W, H = im.size
    if args.crop:
        x0, y0, x1, y1 = parse_crop(args.crop, W, H)
        im = im.crop((x0, y0, x1, y1))
    else:
        x0, y0, x1, y1 = 0, 0, W, H
    w, h = im.size
    area = w * h

    if HAVE_NP:
        a = np.asarray(im)
        m = measure_np(a)
        backend = "numpy"
    else:
        px = list(im.getdata())
        m = measure_pure(px, w, h)
        backend = "pure-python"
        sys.stderr.write("warning: numpy not found, "
                         "m2/orientation/biggest_region unavailable\n")

    dpk = m["distinct"] / (area / 1000)
    k1 = m["edge_h"] is not None and m["edge_h"] >= EDGE_H_PASS
    k2 = dpk >= DPK_PASS
    verdict = "STEPPED" if (k1 and k2) else "SMOOTH"

    out = {
        "image": args.image,
        "region": [x0, y0, x1, y1],
        "region_size": [w, h],
        "backend": backend,
        "K1_edge_h": m["edge_h"],
        "K1_pass_ge": EDGE_H_PASS,
        "K2_distinct_per_kpx": dpk,
        "K2_pass_ge": DPK_PASS,
        "verdict": verdict,
        "supporting": {
            "edge_v": m["edge_v"],
            "edge_combined": m["edge"],
            "grad_flat0": m["grad_flat0"],
            "grad_small1_2": m["grad_small12"],
            "grad_mid3_7": m["grad_mid37"],
            "run_mean_px": m["run_mean"],
            "run_mean_per_100px": (m["run_mean"] / w * 100) if w else None,
            "run_frac_len1": m["run_frac_len1"],
            "run_frac_len_ge8": m["run_frac_len_ge8"],
            "run_max_px": m["run_max"],
            "axis_share": m["axis_share"],
            "diag_share": m["diag_share"],
            "hard_cells": m["hard_cells"],
            "distinct": m["distinct"],
            "modal_exact": m["modal_exact"],
            "modal_q6": m["modal_q6"],
            "m2_block8": m["m2"],
            "biggest_region_frac": m["biggest_region"],
        },
    }

    if args.skyline:
        if HAVE_NP:
            sky = None
            if args.sky:
                try:
                    sky = [float(v) for v in args.sky.split(",")]
                    assert len(sky) == 3
                except (ValueError, AssertionError):
                    sys.stderr.write("error: --sky must be R,G,B\n")
                    return 2
            out["skyline"] = skyline_np(np.asarray(im), sky=sky)
        else:
            out["skyline"] = {"degenerate": "needs numpy"}

    if args.json_only:
        print(json.dumps(out, indent=2))
    else:
        r = out["region"]
        print("image   : %s  region %d,%d,%d,%d  (%dx%d, %s)" %
              (out["image"], r[0], r[1], r[2], r[3], w, h, backend))
        print("K1 edge_h            : %.4f  (pass >= %.3f)  %s" %
              (out["K1_edge_h"], EDGE_H_PASS, "PASS" if k1 else "FAIL"))
        print("K2 distinct/kpx      : %.1f  (pass >= %.0f)  %s" %
              (dpk, DPK_PASS, "PASS" if k2 else "FAIL"))
        print("verdict              : %s" % verdict)
        s = out["supporting"]
        fmt = lambda v, f="%s": "n/a" if v is None else f % v
        print("support: edge_v=%s edge=%s m2=%s modal=%.3f/%.3f distinct=%d "
              "big=%s runs(mean=%.1fpx, len1=%.2f, len>=8=%.2f) "
              "axis=%s diag=%s skyline=%s" % (
                  fmt(s["edge_v"], "%.4f"), fmt(s["edge_combined"], "%.4f"),
                  fmt(s["m2_block8"], "%.2f"), s["modal_exact"], s["modal_q6"],
                  s["distinct"], fmt(s["biggest_region_frac"], "%.3f"),
                  s["run_mean_px"], s["run_frac_len1"], s["run_frac_len_ge8"],
                  fmt(s["axis_share"], "%.2f"), fmt(s["diag_share"], "%.2f"),
                  "n/a" if "skyline" not in out else
                  ("degenerate" if "degenerate" in out["skyline"]
                   else "risers/100px=%.1f tread_med=%.0fpx" % (
                       out["skyline"]["risers_per_100px"],
                       out["skyline"]["tread_median_px"]))))
        print("JSON: " + json.dumps(out))

    if args.check:
        want = args.check.upper()
        if want not in ("STEPPED", "SMOOTH"):
            sys.stderr.write("error: --check must be STEPPED or SMOOTH\n")
            return 2
        if verdict != want:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
