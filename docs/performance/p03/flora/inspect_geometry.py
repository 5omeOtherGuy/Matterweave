#!/usr/bin/env python3
"""Independent inspection of trusted flora_gallery source exports, not native QA.

Run validate_gallery.py first for source/buffer/schema integrity. This script
checks source connectivity, root contact and terrain intersections at voxel
centres. It assumes the fixture's identity-transformed terrain tile; it is not
a general collision detector or an untrusted snapshot loader.
"""

import argparse
from collections import deque
import json
import math
from pathlib import Path


def rotate(x, z, yaw):
    return {
        "Deg0": (x, z), "Deg90": (z, -x),
        "Deg180": (-x, -z), "Deg270": (-z, x),
    }[yaw]


def components(cells):
    unseen = set(cells)
    sizes = []
    while unseen:
        queue = deque([unseen.pop()])
        size = 0
        while queue:
            cell = queue.popleft()
            size += 1
            for axis in range(3):
                for sign in (-1, 1):
                    neighbour = list(cell)
                    neighbour[axis] += sign
                    neighbour = tuple(neighbour)
                    if neighbour in unseen:
                        unseen.remove(neighbour)
                        queue.append(neighbour)
        sizes.append(size)
    return sorted(sizes, reverse=True)


def inspect(root):
    manifest = json.loads((root / "manifest.json").read_text())
    sources = {}
    for record in manifest["prototypes"]:
        snapshot = json.loads((root / (record["id"] + ".snapshot.json")).read_text())
        cells = {(x + dx, y, z): material
                 for x, y, z, count, material in snapshot["runs"]
                 for dx in range(count)}
        sources[record["id"]] = cells, snapshot["scale_m"]
    terrain_id = "terrain_tile_16m"
    terrain_draws = [i for i in manifest["instances"] if i["prototype"] == terrain_id]
    if (len(terrain_draws) != 1 or terrain_draws[0]["yaw"] != "Deg0"
            or terrain_draws[0]["translation_m"] != [0, 0, 0]):
        raise ValueError("inspection requires one identity-transformed terrain tile")
    terrain, tile_scale = sources[terrain_id]
    tops = {}
    for (x, y, z), material in terrain.items():
        if (x, z) not in tops or tops[x, z][0] < y:
            tops[x, z] = y, material
    connectivity = {name: components(cells) for name, (cells, _) in sources.items()
                    if name != terrain_id}
    reports = []
    for instance in manifest["instances"]:
        if instance["prototype"] == terrain_id:
            continue
        cells, scale = sources[instance["prototype"]]
        bottom = min(c[1] for c in cells)
        tx, ty, tz = instance["translation_m"]
        contacts = 0
        bad_contacts = 0
        buried = 0
        for x, y, z in cells:
            px, pz = rotate((x + 0.5) * scale, (z + 0.5) * scale, instance["yaw"])
            column = math.floor((px + tx) / tile_scale), math.floor((pz + tz) / tile_scale)
            if y == bottom:
                contacts += 1
                top = tops.get(column)
                # Palette 11 is the existing moss turf; no water or air roots.
                if (top is None or top[1] != 11
                        or abs(ty + bottom * scale - (top[0] + 1) * tile_scale) > 1e-6):
                    bad_contacts += 1
            world_y = math.floor((ty + (y + 0.5) * scale) / tile_scale)
            material = terrain.get((column[0], world_y, column[1]), 0)
            if material not in (0, 13):
                buried += 1
        reports.append({"instance": instance["instance"], "prototype": instance["prototype"],
                        "contact_cells": contacts, "bad_contacts": bad_contacts,
                        "terrain_intersecting_cell_centres": buried})
    passed = (bool(reports) and all(len(v) == 1 for v in connectivity.values())
              and all(r["contact_cells"] and not r["bad_contacts"]
                      and not r["terrain_intersecting_cell_centres"] for r in reports))
    return {"result": "PASS" if passed else "FAIL", "host_only": True,
            "method": "six-neighbour source connectivity; actual transformed voxel centres",
            "instance_count": len(reports), "source_component_sizes": connectivity,
            "instances": reports}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gallery", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = inspect(args.gallery)
    encoded = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    print(json.dumps({k: v for k, v in result.items() if k != "instances"}))
    return 0 if result["result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
