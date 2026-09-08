#!/usr/bin/env python3
"""Opt-in native Vulkan acceptance check; requires built explorer and Xvfb."""
import argparse
import csv
import json
from pathlib import Path
import subprocess

from validate_frame_profile import validate_profile


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("artifacts", type=Path, help="fresh directory; retained on failure")
    args = parser.parse_args()
    args.artifacts.mkdir(parents=True, exist_ok=False)
    sentinel = b"user save sentinel: deliberately invalid JSON\n"
    world = args.artifacts / "world.json"
    world.write_bytes(sentinel)
    (args.artifacts / "detail-gallery.txt").write_text("tile source\n")
    (args.artifacts / "profile-frames.txt").write_text("20\n")
    with (args.artifacts / "run.log").open("wb") as log:
        subprocess.run(
            ["xvfb-run", "-a", str(args.binary.resolve()), "--smoke-frames", "30",
             "--save", str(world.resolve())],
            stdout=log, stderr=subprocess.STDOUT, timeout=90, check=True,
        )
    assert world.read_bytes() == sentinel, "gallery changed user data"
    assert list(args.artifacts.glob("world*")) == [world], "unexpected recovery/save file"
    captures = list(args.artifacts.glob("frame-profile-v2-*.csv"))
    assert len(captures) == 1, captures
    summary = validate_profile(captures[0])
    (args.artifacts / "validation.json").write_text(json.dumps(summary, indent=2) + "\n")
    assert summary["row_count"] == 20, summary
    # Producer has exited; this is the owned, validated acceptance fixture.
    with captures[0].open(newline="") as source:
        next(source)  # Version header, already strictly validated above.
        rows = list(csv.DictReader(source))
    for row in rows:
        for name in ("dynamic_mesh_builds", "dynamic_mesh_uploads", "physics_fixed_steps",
                     "voxel_bodies_total", "chunk_mesh_uploads", "save_attempts", "save_failures"):
            assert row[name] == "0", (name, row[name])
        for name in ("dynamic_mesh_build_wall_ms", "dynamic_upload_wall_ms"):
            assert row[name] == "", (name, row[name], "no dynamic pipeline in gallery")
    assert any(float(row["mesh_sync_wall_ms"]) > 0 for row in rows)
    print("PASS:20 valid gallery rows, no borrowed dynamic work, sentinel unchanged")


if __name__ == "__main__":
    main()
