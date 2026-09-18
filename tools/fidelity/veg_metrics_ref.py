#!/usr/bin/env python3
"""Near-field vegetation metrics against a flora-off ground reference.

The ground reference is the same camera rendered with the flora layer disabled
(MATTERWEAVE_LANDSCAPE_FLORA=off). The terrain, water and sky passes are
identical in both runs, so any crop pixel that differs from the reference is
vegetation (or its shadow on the ground).

- bare-ground fraction: share of crop pixels with no vegetation.
- mean vertical run: mean length in pixels of maximal vertical runs of
  vegetation, over every column of the crop.

Usage: veg_metrics_ref.py IMAGE REFERENCE --crop x0,y0,x1,y1 [--tolerance N]
"""
import argparse

import numpy as np
from PIL import Image


def mask_of(image, reference, crop, tolerance):
    x0, y0, x1, y1 = crop
    a = np.asarray(Image.open(image).convert("RGB")).astype(np.int16)[y0:y1, x0:x1]
    b = np.asarray(Image.open(reference).convert("RGB")).astype(np.int16)[y0:y1, x0:x1]
    return np.abs(a - b).max(axis=2) > tolerance, a


def runs(veg):
    h, w = veg.shape
    lengths = []
    for x in range(w):
        y = 0
        while y < h:
            if veg[y, x]:
                start = y
                while y < h and veg[y, x]:
                    y += 1
                lengths.append(y - start)
            else:
                y += 1
    return lengths


def report(image, reference, crop, tolerance, plot=None):
    veg, rgb = mask_of(image, reference, crop, tolerance)
    lengths = runs(veg)
    print(f"{image} vs {reference}")
    print(
        f"  crop x[{crop[0]}:{crop[2]}] y[{crop[1]}:{crop[3]}] "
        f"= {crop[2]-crop[0]}x{crop[3]-crop[1]} px, tolerance {tolerance}"
    )
    print(f"  bare-ground fraction  {(1.0 - veg.mean()) * 100:.1f}%")
    print(f"  vegetation fraction   {veg.mean() * 100:.1f}%")
    print(f"  vertical runs         {len(lengths)}")
    print(
        f"  mean vertical run     {np.mean(lengths) if lengths else 0.0:.1f} px"
    )
    print(
        f"  p90 vertical run      {np.percentile(lengths, 90) if lengths else 0.0:.1f} px"
    )
    if plot:
        out = np.zeros_like(rgb, dtype=np.uint8)
        out[..., 1] = np.where(veg, 255, 0)
        Image.fromarray(np.concatenate([out, rgb.astype(np.uint8)], axis=1)).save(plot)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("image")
    ap.add_argument("reference")
    ap.add_argument("--crop", default="60,610,440,720")
    ap.add_argument("--tolerance", type=int, default=12)
    ap.add_argument("--plot", default=None)
    args = ap.parse_args()
    crop = tuple(int(v) for v in args.crop.split(","))
    report(args.image, args.reference, crop, args.tolerance, args.plot)


if __name__ == "__main__":
    main()
