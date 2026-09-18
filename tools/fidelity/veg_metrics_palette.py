#!/usr/bin/env python3
"""Near-field ground-cover metrics, following the fidelity spec's A4 method.

The spec measures bare ground as "pixels that are one of four flat ground/face
colours" (55.1% of our old near-grass crop). This reproduces that measurement
without hand-picking colours:

1. Take the same camera rendered with the flora layer disabled
   (MATTERWEAVE_LANDSCAPE_FLORA=off). Its near-field crop is pure ground and
   water.
2. The four most common 8-bit-quantised colours in that crop are the ground's
   flat palette; pixels within `--tol` of one of them are bare ground.
3. Everything in the crop that is not bare ground and not water (water is
   classified from the reference's own colour families) is vegetation.

Reported for each image:
- bare-ground fraction over land pixels,
- mean / p90 / max vertical run length of vegetation along image columns.

Usage: veg_metrics_palette.py IMAGE REFERENCE --crop x0,y0,x1,y1 [--tol N]
"""
import argparse
from collections import Counter

import numpy as np
from PIL import Image


def load(path, crop):
    x0, y0, x1, y1 = crop
    return np.asarray(Image.open(path).convert("RGB")).astype(int)[y0:y1, x0:x1]


def palette(reference, count=4, quant=8):
    q = (reference // quant * quant).reshape(-1, 3)
    common = Counter(map(tuple, q)).most_common(count)
    return [np.array(colour, dtype=float) + quant / 2 for colour, _ in common]


def families(reference, count=6, quant=8):
    """The reference's own colour families, used only to exclude water."""
    q = (reference // quant * quant).reshape(-1, 3)
    return [np.array(colour, dtype=float) + quant / 2 for colour, _ in Counter(map(tuple, q)).most_common(count)]


def water_mask(reference):
    """Water in the reference: blue-dominant pixels. The terrain never is."""
    r, g, b = reference[..., 0], reference[..., 1], reference[..., 2]
    return (b > r + 6) & (b > g - 12)


def classify(image, reference, ground, tol):
    is_water = water_mask(reference)
    d_ground = np.min(
        np.stack([np.abs(image - colour).max(axis=2) for colour in ground]), axis=0
    )
    is_ground = (d_ground <= tol) & ~is_water
    land = ~is_water
    return is_ground, land


def runs_of(mask):
    """All maximal vertical run lengths, plus per-column max and span."""
    runs, max_runs, spans = [], [], []
    for x in range(mask.shape[1]):
        col = mask[:, x]
        y = 0
        longest = 0
        while y < len(col):
            if col[y]:
                start = y
                while y < len(col) and col[y]:
                    y += 1
                runs.append(y - start)
                longest = max(longest, y - start)
            else:
                y += 1
        max_runs.append(longest)
        idx = np.nonzero(col)[0]
        spans.append(idx[-1] - idx[0] + 1 if len(idx) else 0)
    return runs, max_runs, spans


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("image")
    ap.add_argument("reference")
    ap.add_argument("--crop", default="121,525,606,720")
    ap.add_argument("--tol", type=int, default=14)
    ap.add_argument("--plot", default=None)
    args = ap.parse_args()
    crop = tuple(int(v) for v in args.crop.split(","))

    ref = load(args.reference, crop)
    img = load(args.image, crop)
    ground = palette(ref, 4)
    is_ground, land = classify(img, ref, ground, args.tol)
    veg = land & ~is_ground

    runs, max_runs, spans = runs_of(veg)
    print(f"{args.image} vs {args.reference}")
    print(f"  crop {crop} = {crop[2]-crop[0]}x{crop[3]-crop[1]} px, tol {args.tol}")
    print(f"  ground palette      {[list(map(int, c)) for c in ground]}")
    print(f"  land fraction       {land.mean()*100:.1f}%")
    print(f"  bare-ground / land  {is_ground.sum() / max(land.sum(),1) * 100:.1f}%")
    print(f"  vegetation / land   {veg.sum() / max(land.sum(),1) * 100:.1f}%")
    print(f"  vertical runs       {len(runs)}")
    print(f"  mean run            {np.mean(runs) if runs else 0:.1f} px")
    print(f"  p90 run             {np.percentile(runs, 90) if runs else 0:.1f} px")
    print(f"  mean per-col max    {np.mean(max_runs):.1f} px")
    print(f"  mean per-col span   {np.mean(spans):.1f} px")
    if args.plot:
        out = np.zeros_like(img, dtype=np.uint8)
        out[..., 1] = np.where(veg, 255, 0)
        out[..., 0] = np.where(is_ground, 60, out[..., 0])
        out[..., 2] = np.where(~land, 255, out[..., 2])
        Image.fromarray(np.concatenate([out, img.astype(np.uint8)], axis=1)).save(args.plot)


if __name__ == "__main__":
    main()
