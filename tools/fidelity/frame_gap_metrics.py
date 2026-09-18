#!/usr/bin/env python3
"""frame-gap-metrics.py -- quantify the visual gap between our capture and refs.

Second script in the fidelity toolkit (see voxel-step-metric.py and
fidelity-spec.md). It measures the properties named in the analyst brief
so implementation briefs can be written against thresholds instead of
opinions. Prescribed-but-weak measures were tested and either fixed or
dropped -- see DROPPED below.

Kept metrics (all scale-free: fractions, per-kpx, or computed at a fixed
analysis width, so our 3168 px frames compare fairly with 1600 px refs):

  cov_win    windowed coverage: median over tiles (side ~= width/25, as in
             spec M1) of the dominant //13-bucket share. High = bare flat
             plane. (Global modal share is reported as cov_modal but
             DROPPED -- see below.)
  ground     fraction of near-region pixels smooth in BOTH directions
             (ridge filter, analysis width 800). High = bare ground visible.
  blade      fraction of pixels thin-vertical (differ across, continue
             along). Large for tall thin blades, small for cube props.
  trunk      same filter at coarse offset: thick vertical masses (palm
             trunks, near trees).
  block      corner/edge fraction (differs both ways): chunky cuboid edges.
  horiz      thin-horizontal fraction: terrace risers, strata, surf bands.
  spreadL/H  colour spread inside the material neighbourhood (buckets
             within Chebyshev <= 2 of the modal bucket): luminance std and
             circular hue std (hue over sat>0.15 px only).
  sat_mean/p90  HSV saturation per region.
  edge/m2    mid-band content density: |dLum|>=8 neighbour fraction and
             median 8x8 block std at 1600-equivalent width (spec M2 scale),
             plus buckets_per_kpx (noise-robust variety; exact-colour
             distinct counts are DROPPED).
  spectral   water tiling detector: plane-detrended 2D FFT, 12-160 px band.
             peakmed  = peak/median power (regularity, NOT amplitude);
             frachi   = fraction of band bins above 50x median;
             wl/ang   = peak wavelength/direction; rms = detrended amplitude.
  lum_*      luminance mean/sd/p1/p99 per region (palette context).

DROPPED (computed, reported, rejected for ranking):
  D1 cov_modal (global dominant-bucket share): confounded by depth/fog
     gradients across large crops (one material spans many buckets) and by
     codec noise fragmenting reference regions. Ours-near 0.10 sits BELOW
     most refs (0.04-0.36) -- inverted polarity. Windowed cov_win carries
     the coverage signal instead.
  D2 distinct_per_kpx (exact colours): dominated by YouTube noise
     (0132-near: 346/kpx vs ours 10/kpx). buckets_per_kpx replaces it.
  D3 vert_run (mean column run of non-modal pixels, as briefed): the modal
     bucket is 4-36% of any region, so "non-ground" is 64-96% of pixels and
     runs measure blob size, not blade height. Ours-near 34/100px vs refs
     10-32/100px -- no separation. blade/trunk ridge fractions replace it.
  D4 plain autocorrelation peak (as briefed): every image peaks at the
     minimum lag (broadband + gradient decay), ours and refs alike -- it
     measures smoothness, not tiling. The detrended spectral metric is its
     tested replacement: ours-water peakmed ~49k-187k vs water-refs
     ~0.5k-10k.

Regions (input-image pixels; exact rect in every output row). Semantic,
not pixel-identical -- the framings differ, so each source gets bands
that land on sky / horizon / middle / water / foreground:

  ours (3168x1440, top 440 rows are debug HUD):
    sky     (0,445,3168,545)      distant (0,460,3168,590)
    mid     (0,590,3168,845)      water (1100,790,3168,1145)
    near    (650,1080,1750,1410)  (x>=650 avoids the MOVE overlay box)
  ref (1600x900 YouTube; title rows <40 and control rows >825 are outside
  every region; hotbar/side-button/icon rects in CHROME_MASK are dropped
  from colour stats and median-filled for spatial stats):
    sky     (0,45,1600,250)       distant (0,300,1600,430)
    mid     (0,420,1600,560)      water (0,430,1600,600)
    near    (0,540,1600,770)      (stops above the hotbar, y~775+)

Usage:
  frame-gap-metrics.py IMAGE [IMAGE ...]
      [--region all|near|mid|distant|water|sky] [--crop X0,Y0,X1,Y1]
      [--source auto|ours|ref] [--no-mask] [--json-only] [--chrome-test]
Exit code 0 on success, 2 on usage/IO errors. Needs Pillow and numpy.
"""

import argparse
import json
import sys

try:
    from PIL import Image
except ImportError:
    sys.stderr.write("error: Pillow (PIL) is required\n")
    sys.exit(2)

try:
    import numpy as np
except ImportError:
    sys.stderr.write("error: numpy is required\n")
    sys.exit(2)

PRESETS = {
    "ours": {
        "sky": (0, 445, 3168, 545),
        "distant": (0, 460, 3168, 590),
        "mid": (0, 590, 3168, 845),
        "water": (1100, 790, 3168, 1145),
        "near": (650, 1080, 1750, 1410),
    },
    "ref": {
        "sky": (0, 45, 1600, 250),
        "distant": (0, 300, 1600, 430),
        "mid": (0, 420, 1600, 560),
        "water": (0, 430, 1600, 600),
        "near": (0, 540, 1600, 770),
    },
}
REGION_ORDER = ["sky", "distant", "mid", "water", "near"]

CHROME_MASK = [
    (430, 772, 1070, 838),    # hotbar slots + progress-bar centre overlap
    (0, 650, 95, 800),        # left side buttons
    (1400, 700, 1600, 800),   # right-side icons
]

QSTEP = 13            # coarse bucket (~ +/-6 RGB, matches spec M1)
EDGE_T = 8            # hard-step threshold on Rec.709 luminance
SAT_HUE_MIN = 0.15
ANALYSIS_W = 800      # fixed width for structural metrics (scale-free)
M2EQ_W = 1600         # spec M2 reference width
SPEC_LO, SPEC_HI = 12.0, 160.0   # ripple wavelength band (px)
SPEC_K = 50.0


# ---------------------------------------------------------------- basics

def detect_source(im):
    w, h = im.size
    if (w, h) == (3168, 1440):
        return "ours"
    if (w, h) == (1600, 900):
        return "ref"
    return "ours" if h > 1000 else "ref"


def clip_rect(rect, w, h):
    x0, y0, x1, y1 = rect
    return (max(0, min(w, x0)), max(0, min(h, y0)),
            max(0, min(w, x1)), max(0, min(h, y1)))


def luminance(a):
    a = np.asarray(a, dtype=np.float64)
    return 0.2126 * a[..., 0] + 0.7152 * a[..., 1] + 0.0722 * a[..., 2]


def sat_hue(px):
    px = np.asarray(px, dtype=np.float64)
    mx = px.max(axis=1)
    mn = px.min(axis=1)
    d = mx - mn
    sat = np.zeros_like(mx)
    nz = mx > 0
    sat[nz] = d[nz] / mx[nz]
    hue = np.zeros_like(mx)
    m = d > 0
    r, g, b = px[:, 0], px[:, 1], px[:, 2]
    i = m & (mx == r)
    hue[i] = ((g[i] - b[i]) / d[i]) % 6.0
    i = m & (mx == g)
    hue[i] = (b[i] - r[i]) / d[i] + 2.0
    i = m & (mx == b)
    hue[i] = (r[i] - g[i]) / d[i] + 4.0
    return sat, hue * 60.0


def circ_std_deg(h):
    h = np.asarray(h, dtype=np.float64)
    if len(h) == 0:
        return None
    R = abs(np.exp(1j * np.deg2rad(h)).mean())
    if R < 1e-9:
        return 104.3
    return float(np.sqrt(-2.0 * np.log(min(1.0, R))) * 180.0 / np.pi)


def bucket_keys(px):
    q = (np.asarray(px) // QSTEP).astype(np.int64)
    if q.ndim == 2:
        return (q[:, 0] * 64 + q[:, 1]) * 64 + q[:, 2]
    return (q[:, :, 0] * 64 + q[:, :, 1]) * 64 + q[:, :, 2]


# ---------------------------------------------------------------- chrome

def split_chrome(a, rect, source, use_mask):
    """Return (kept_pixels (N,3), filled_crop (H,W,3), drop_frac).

    Colour stats use kept_pixels (chrome dropped). Spatial stats use
    filled_crop (chrome median-filled so no fake edges enter)."""
    x0, y0, x1, y1 = rect
    sub = a[y0:y1, x0:x1]
    H, W, _ = sub.shape
    if source != "ref" or not use_mask:
        return sub.reshape(-1, 3), sub, 0.0
    drop = np.zeros((H, W), dtype=bool)
    for (cx0, cy0, cx1, cy1) in CHROME_MASK:
        ox0, oy0 = max(cx0, x0), max(cy0, y0)
        ox1, oy1 = min(cx1, x1), min(cy1, y1)
        if ox1 > ox0 and oy1 > oy0:
            drop[oy0 - y0:oy1 - y0, ox0 - x0:ox1 - x0] = True
    kept = sub[~drop]
    filled = sub.copy()
    if drop.any() and kept.size:
        filled[drop] = np.median(kept.reshape(-1, 3), axis=0).astype(sub.dtype)
    return kept, filled, float(drop.mean())


# ---------------------------------------------------------------- colour

def colour_metrics(px):
    n = len(px)
    out = {"n": int(n)}
    if n == 0:
        return out
    keys = bucket_keys(px)
    vals, counts = np.unique(keys, return_counts=True)
    o = np.argsort(-counts)
    out["cov_modal"] = float(counts[o[0]] / n)          # D1 (dropped)
    out["modal_rgb"] = [int(v) for v in px[keys == vals[o[0]]][0]]
    out["distinct_buckets"] = int(len(vals))
    out["buckets_per_kpx"] = float(len(vals) / (n / 1000.0))
    pu = np.asarray(px).astype(np.int64)
    out["distinct"] = int(len(np.unique(
        pu[:, 0] * 65536 + pu[:, 1] * 256 + pu[:, 2])))  # D2 (dropped)
    out["distinct_per_kpx"] = out["distinct"] / (n / 1000.0)
    # material neighbourhood: buckets within Chebyshev <= 2 of modal
    modal = int(vals[o[0]])
    mb = np.array([modal // 4096, (modal // 64) % 64, modal % 64])
    qb = np.stack([keys // 4096, (keys // 64) % 64, keys % 64], axis=1)
    mat = np.abs(qb - mb).max(axis=1) <= 2
    out["mat_frac"] = float(mat.mean())
    lum = luminance(px)
    out["spreadL"] = float(lum[mat].std()) if mat.any() else None
    sat_m, hue_m = sat_hue(np.asarray(px)[mat]) if mat.any() else ([], [])
    use = np.asarray(hue_m)[np.asarray(sat_m) >= SAT_HUE_MIN]
    out["spreadH"] = circ_std_deg(use)
    out["spreadH_used"] = float(len(use) / max(1, len(np.asarray(hue_m))))
    sat, _ = sat_hue(px)
    out["sat_mean"] = float(sat.mean())
    out["sat_p90"] = float(np.percentile(sat, 90))
    out["lum_mean"] = float(lum.mean())
    out["lum_sd"] = float(lum.std())
    out["lum_p1"] = float(np.percentile(lum, 1))
    out["lum_p99"] = float(np.percentile(lum, 99))
    return out


def windowed_coverage(sub):
    H, W, _ = sub.shape
    ws = max(24, round(W / 25))
    shares = []
    for yy in range(0, H, ws):
        for xx in range(0, W, ws):
            w = bucket_keys(sub[yy:yy + ws, xx:xx + ws].reshape(-1, 3))
            _, c = np.unique(w, return_counts=True)
            shares.append(c.max() / c.sum())
    return float(np.median(shares))


def vertical_runs(sub, modal_key):
    """D3 (dropped): mean column run of non-modal pixels, per 100px height."""
    H, W, _ = sub.shape
    nongr = bucket_keys(sub) != modal_key
    step = max(1, W // 400)
    total, cnt = 0, 0
    for x in range(0, W, step):
        ln = 0
        for v in nongr[:, x]:
            if v:
                ln += 1
            elif ln:
                total += ln
                cnt += 1
                ln = 0
        if ln:
            total += ln
            cnt += 1
    mean = total / cnt if cnt else 0.0
    return float(mean / H * 100.0) if H else 0.0, float(nongr.mean())


# ---------------------------------------------------------------- spatial

def analysis_crop(sub):
    H, W, _ = sub.shape
    nh = max(1, round(H * ANALYSIS_W / W))
    im = Image.fromarray(np.asarray(sub, dtype=np.uint8))
    return np.asarray(im.resize((ANALYSIS_W, nh), Image.LANCZOS)).astype(float)


def ridge_fractions(s, offsets=(3, 12), t_lo=12.0, t_hi=25.0):
    """Thin-structure detector. For offset d: H = colour distance to
    horizontal neighbours at +-d, V likewise vertical. Returns dict with
    ground/blade/block/horiz at d=offsets[0] and trunk at d=offsets[1]."""
    h, w, _ = s.shape
    out = {}
    for tag, dd in (("fine", offsets[0]), ("coarse", offsets[1])):
        dd = max(1, min(dd, w // 6, h // 4))
        if tag == "coarse" and dd < 8:
            out["trunk"] = None  # region too short for coarse structure
            continue
        L = np.pad(s, ((dd, dd), (dd, dd), (0, 0)), mode="edge")
        C = L[dd:-dd, dd:-dd]
        cd = lambda A, B: np.sqrt(((A - B) ** 2).sum(-1))
        Hm = (cd(C, L[dd:-dd, :-2 * dd]) + cd(C, L[dd:-dd, 2 * dd:])) / 2
        Vm = (cd(C, L[:-2 * dd, dd:-dd]) + cd(C, L[2 * dd:, dd:-dd])) / 2
        if tag == "fine":
            out["ground"] = float(((Hm < t_lo) & (Vm < t_lo)).mean())
            out["blade"] = float(((Hm > t_hi) & (Vm < Hm / 2)).mean())
            out["block"] = float(((Hm > t_hi) & (Vm > t_hi)).mean())
            out["horiz"] = float(((Vm > t_hi) & (Hm < Vm / 2)).mean())
        else:
            out["trunk"] = float(((Hm > t_hi) & (Vm < Hm / 2)).mean())
    return out


def edge_m2(sub, full_w):
    L = luminance(sub)
    gx = np.abs(np.diff(L, axis=1))
    gy = np.abs(np.diff(L, axis=0))
    edge = {"edge_h": float((gx >= EDGE_T).mean()),
            "edge_v": float((gy >= EDGE_T).mean())}
    edge["edge"] = float(((gx >= EDGE_T).sum() + (gy >= EDGE_T).sum())
                         / (gx.size + gy.size))
    # spec-M2 scale: region at 1600/source-width
    H, W, _ = sub.shape
    nw = max(8, round(W * M2EQ_W / full_w))
    nh = max(8, round(H * M2EQ_W / full_w))
    im = Image.fromarray(np.asarray(sub, dtype=np.uint8))
    r = np.asarray(im.resize((nw, nh), Image.LANCZOS)).astype(float)
    L2 = luminance(r)
    H2, W2 = (L2.shape[0] // 8 * 8, L2.shape[1] // 8 * 8)
    if H2 >= 8 and W2 >= 8:
        b = L2[:H2, :W2].reshape(H2 // 8, 8, W2 // 8, 8)
        edge["m2"] = float(np.median(b.std(axis=(1, 3))))
    else:
        edge["m2"] = None
    return edge


def spectral(sub):
    """Detrended-FFT tiling detector (tested replacement for plain AC)."""
    L = luminance(sub)
    H, W = L.shape
    yy, xx = np.mgrid[0:H, 0:W].astype(float)
    A = np.stack([xx.ravel() / W, yy.ravel() / H, np.ones(H * W)], axis=1)
    coef, _, _, _ = np.linalg.lstsq(A, L.ravel(), rcond=None)
    D = L - (coef[0] * xx / W + coef[1] * yy / H + coef[2])
    rms = float(np.sqrt((D ** 2).mean()))
    F = np.fft.rfft2(D * np.hanning(H)[:, None] * np.hanning(W)[None, :])
    P = np.abs(F) ** 2
    fy = np.broadcast_to(np.fft.fftfreq(H)[:, None], P.shape)
    fx = np.broadcast_to(np.fft.rfftfreq(W)[None, :], P.shape)
    fr = np.sqrt(fy ** 2 + fx ** 2)
    band = (fr >= 1.0 / SPEC_HI) & (fr <= 1.0 / SPEC_LO) & (fr > 0)
    Pb = P[band]
    if Pb.size == 0:
        return {"peakmed": None, "frachi": None, "wl": None,
                "ang": None, "rms": rms}
    med = float(np.median(Pb))
    ip = int(np.argmax(Pb))
    fyb, fxb = float(fy[band][ip]), float(fx[band][ip])
    return {"peakmed": float(Pb.max() / med) if med > 0 else None,
            "frachi": float((Pb > SPEC_K * med).mean()),
            "wl": float(1.0 / fr[band][ip]),
            "ang": float(np.degrees(np.arctan2(fyb, fxb))),
            "rms": rms}


# ---------------------------------------------------------------- main

def parse_crop(s, w, h):
    try:
        vals = [int(v) for v in s.split(",")]
        assert len(vals) == 4
    except (ValueError, AssertionError):
        raise SystemExit("error: --crop must be X0,Y0,X1,Y1")
    return clip_rect(tuple(vals), w, h)


HEADLINE = ("cov_win ground blade trunk block horiz spreadL spreadH "
            "sat satp90 edge m2 bpk peakmed frachi wl rms lum_sd").split()


def measure_region(a, rect, source, use_mask, full_w, chrome_test):
    px, filled, drop = split_chrome(a, rect, source, use_mask)
    m = colour_metrics(px)
    if m.get("n", 0) == 0:
        return {"rect": list(rect), "error": "empty"}
    m["cov_win"] = windowed_coverage(filled)
    keys = bucket_keys(np.asarray(px))
    vals, counts = np.unique(keys, return_counts=True)
    modal_key = int(vals[np.argmax(counts)])
    vr, ng = vertical_runs(filled, modal_key)   # D3, reported only
    m["vert_run_D3"] = vr
    m["nonground_D3"] = ng
    s = analysis_crop(filled)
    m.update(ridge_fractions(s))
    m.update(edge_m2(filled, full_w))
    sp = spectral(filled)
    m["period_peakmed"] = sp["peakmed"]
    m["period_frachi"] = sp["frachi"]
    m["period_wl"] = sp["wl"]
    m["period_ang"] = sp["ang"]
    m["ripple_rms"] = sp["rms"]
    m["rect"] = list(rect)
    m["region_size"] = [rect[2] - rect[0], rect[3] - rect[1]]
    m["mask_drop_frac"] = drop
    if chrome_test and source == "ref":
        px2, filled2, _ = split_chrome(a, rect, source, False)
        m2 = colour_metrics(px2)
        s2 = analysis_crop(filled2)
        r2 = ridge_fractions(s2)
        e2 = edge_m2(filled2, full_w)
        delta = {}
        for k in ("cov_modal", "sat_mean", "spreadL", "spreadH",
                  "buckets_per_kpx"):
            v_on, v_off = m.get(k), m2.get(k)
            delta[k] = ((v_off - v_on)
                          if isinstance(v_on, float)
                          and isinstance(v_off, float) else None)
        for k in ("ground", "blade"):
            delta[k] = r2.get(k, 0) - m.get(k, 0)
        delta["edge_check"] = e2["edge"] - m["edge"]
        m["chrome_delta_off_minus_on"] = delta
    return m


def fmt_row(img, name, m):
    if "error" in m or m.get("n", 0) == 0:
        return "%-40s %-7s EMPTY" % (img, name)
    f = lambda v, s="%s": "n/a" if v is None else s % v
    return (
        "%-40s %-7s covw=%.3f g=%.3f bl=%.4f tr=%s bk=%.4f hz=%.4f "
        "spL=%5.2f spH=%s sat=%.2f/%.2f edge=%.4f m2=%s bpk=%.1f "
        "pm=%s fhi=%.3f wl=%s rms=%.1f" % (
            img, name, m["cov_win"], m["ground"], m["blade"],
            f(m["trunk"], "%.4f"),
            m["block"], m["horiz"], m["spreadL"], f(m["spreadH"], "%5.1f"),
            m["sat_mean"], m["sat_p90"], m["edge"],
            f(m["m2"], "%.2f"), m["buckets_per_kpx"],
            f(m["period_peakmed"], "%.0f"), m["period_frachi"],
            f(m["period_wl"], "%.0f"), m["ripple_rms"]))


def main(argv=None):
    ap = argparse.ArgumentParser(
        description="Frame-gap metrics per region (see module docstring).")
    ap.add_argument("image", nargs="+", help="input image(s)")
    ap.add_argument("--region", default="all",
                    choices=["all"] + REGION_ORDER,
                    help="which region to measure (default: all)")
    ap.add_argument("--crop", default=None, metavar="X0,Y0,X1,Y1",
                    help="explicit rect overriding the region preset "
                         "(only with a single --region)")
    ap.add_argument("--source", default="auto", choices=["auto", "ours", "ref"])
    ap.add_argument("--no-mask", action="store_true",
                    help="disable the reference player-chrome mask")
    ap.add_argument("--json-only", action="store_true")
    ap.add_argument("--chrome-test", action="store_true",
                    help="report mask-off-minus-mask-on deltas (quantifies "
                         "chrome confounding)")
    args = ap.parse_args(argv)

    if args.crop and args.region == "all":
        sys.stderr.write("error: --crop needs a single --region\n")
        return 2

    regions = REGION_ORDER if args.region == "all" else [args.region]
    results = []
    rc = 0
    for path in args.image:
        try:
            im = Image.open(path).convert("RGB")
        except FileNotFoundError:
            sys.stderr.write("error: no such image: %s\n" % path)
            rc = 2
            continue
        except Exception as e:
            sys.stderr.write("error: cannot read %s: %s\n" % (path, e))
            rc = 2
            continue
        w, h = im.size
        source = args.source
        if source == "auto":
            source = detect_source(im)
        if args.crop:
            PRESETS[source][args.region] = parse_crop(args.crop, w, h)
        a = np.asarray(im)
        res = {"image": path, "size": [w, h], "source": source, "regions": {}}
        for name in regions:
            rect = clip_rect(PRESETS[source][name], w, h)
            x0, y0, x1, y1 = rect
            if x1 <= x0 or y1 <= y0:
                res["regions"][name] = {"rect": list(rect), "error": "empty"}
                continue
            res["regions"][name] = measure_region(
                a, rect, source, not args.no_mask, w, args.chrome_test)
        results.append(res)

    out = {"results": results}
    if args.json_only:
        print(json.dumps(out, indent=2))
    else:
        for res in results:
            for name in regions:
                print(fmt_row(res["image"], name, res["regions"][name]))
        print("JSON: " + json.dumps(out))
    return rc


if __name__ == "__main__":
    sys.exit(main())
