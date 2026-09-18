#!/usr/bin/env python3
"""Near-field vegetation metrics for the landscape captures.

Metrics, both computed over one fixed pixel crop:

- bare-ground fraction: share of crop pixels that are terrain ground, i.e.
  neither vegetation nor sky/water. Ground is classified as the modal colour
  family of the crop's lower band (the flat plane), vegetation as blade-green
  (green channel strongly above red and blue, or dark green) or flower colour.

- mean vertical run length: over every image column in the crop, the mean
  length in pixels of contiguous vegetation runs. Blades are tall, so a
  column through a blade is one long run; cube props give short runs.

Usage: veg_metrics.py IMAGE [--crop x0,y0,x1,y1] [--plot out.png]
"""
import argparse
import sys

import numpy as np
from PIL import Image


def load(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(np.float32)


def hue_like(rgb):
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    return r, g, b


def classify(image, crop):
    x0, y0, x1, y1 = crop
    sub = image[y0:y1, x0:x1]
    r, g, b = hue_like(sub)
    # Vegetation: green-dominant and either saturated or dark (shaded blades are
    # darker and less saturated than the lit ground plane), or a flower colour.
    green_dom = (g - np.maximum(r, b)) > 8.0
    dark_green = (g > r) & (g >= b) & (g < 150.0)
    flower = ((r - np.maximum(g, b) > 40.0) |         # red/yellow-ish petals
              ((r > 200) & (g > 200) & (b > 200)) |   # white petals
              ((r > 170) & (g > 150) & (b > 170) & (r - b < 40) & (g - b < 30)))
    veg = (green_dom | dark_green | flower)
    ground = ~veg
    return sub, veg, ground


def vertical_runs(veg):
    h, w = veg.shape
    total = 0
    count = 0
    for x in range(w):
        col = veg[:, x]
        y = 0
        while y < h:
            if col[y]:
                start = y
                while y < h and col[y]:
                    y += 1
                total += y - start
                count += 1
            else:
                y += 1
    return total, count


def report(path, crop, plot=None):
    image = load(path)
    h, w = image.shape[:2]
    if crop is None:
        x0, y0 = 0, int(h * 0.55)
        x1, y1 = int(w * 0.42), h
    else:
        x0, y0, x1, y1 = crop
    sub, veg, ground = classify(image, (x0, y0, x1, y1))
    bare = ground.mean()
    total, count = vertical_runs(veg)
    mean_run = total / count if count else 0.0
    print(f"{path}")
    print(f"  crop x[{x0}:{x1}] y[{y0}:{y1}] = {x1-x0}x{y1-y0} px")
    print(f"  bare-ground fraction  {bare*100:.1f}%")
    print(f"  vegetation fraction   {veg.mean()*100:.1f}%")
    print(f"  vertical runs         {count}")
    print(f"  mean vertical run     {mean_run:.1f} px")
    print(f"  p90 vertical run      {np.percentile([r for r in runs(veg)], 90):.1f} px"
          if count else "  p90 vertical run      n/a")
    if plot:
        out = np.zeros_like(sub, dtype=np.uint8)
        out[..., 1] = np.where(veg, 255, 0)
        out[..., 0] = np.where(ground, 200, 0)
        Image.fromarray(np.concatenate([out, sub.astype(np.uint8)], axis=1)).save(plot)


def runs(veg):
    h, w = veg.shape
    out = []
    for x in range(w):
        col = veg[:, x]
        y = 0
        while y < h:
            if col[y]:
                start = y
                while y < h and col[y]:
                    y += 1
                out.append(y - start)
            else:
                y += 1
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("image")
    ap.add_argument("--crop", default=None, help="x0,y0,x1,y1")
    ap.add_argument("--plot", default=None)
    args = ap.parse_args()
    crop = tuple(int(v) for v in args.crop.split(",")) if args.crop else None
    report(args.image, crop, args.plot)


if __name__ == "__main__":
    sys.exit(main())
