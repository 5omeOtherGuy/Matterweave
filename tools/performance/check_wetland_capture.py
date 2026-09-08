#!/usr/bin/env python3
"""Run the full wetland through Vulkan and validate its real frame capture and save."""
import argparse
import csv
import json
import os
from pathlib import Path
import signal
import subprocess

from validate_frame_profile import validate_profile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("artifacts", type=Path, help="fresh output directory")
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args()
    args.artifacts.mkdir(parents=True, exist_ok=False)
    world = args.artifacts / "world.json"
    sentinel = b"legacy world sentinel\n"
    world.write_bytes(sentinel)
    (args.artifacts / "profile-frames.txt").write_text("20\n")
    with (args.artifacts / "run.log").open("wb") as log:
        process = subprocess.Popen(
            ["xvfb-run", "-a", str(args.binary.resolve()), "--showcase",
             "--smoke-frames", "25", "--save", str(world.resolve())],
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        try:
            code = process.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            raise
        assert code == 0, f"native wetland exited {code}; see run.log"
    output = (args.artifacts / "run.log").read_text()
    assert "WETLAND SMOKE PASS: 25 frames" in output, output
    assert "Validation Error" not in output and "Vulkan validation ERROR" not in output, output
    assert world.read_bytes() == sentinel, "wetland modified legacy save"
    session = json.loads((args.artifacts / "wetland-session.json").read_text())
    assert len(session["physics"]["bodies"]) == 6, session
    captures = list(args.artifacts.glob("frame-profile-v2-*.csv"))
    assert len(captures) == 1, captures
    summary = validate_profile(captures[0])
    (args.artifacts / "validation.json").write_text(json.dumps(summary, indent=2) + "\n")
    assert summary["row_count"] == 20, summary
    with captures[0].open(newline="") as source:
        next(source)
        rows = list(csv.DictReader(source))
    assert all(row["voxel_bodies_total"] == "6" for row in rows), rows
    assert any(int(row["physics_fixed_steps"]) > 0 for row in rows), "simulation did not advance"
    assert all(row["save_failures"] == "0" for row in rows)
    print("PASS: full wetland rendered25 frames;20 valid rows;6 bodies;separate save;no Vulkan errors")


if __name__ == "__main__":
    main()
