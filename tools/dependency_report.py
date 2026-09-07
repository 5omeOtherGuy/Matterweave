#!/usr/bin/env python3
"""Refresh upstream Cargo provenance from locked metadata (Python 3.11+)."""
import json
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT
))
lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
checksums = {(p["name"], p["version"]): p.get("checksum") for p in lock["package"]}
packages = []
for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
    if package["source"]:
        record = {key: package[key] for key in ("name", "version", "source", "license", "repository")}
        record["checksum"] = checksums[(package["name"], package["version"])]
        packages.append(record)
report = {
    "note": "All locked Cargo packages, including host, target-specific and build dependencies. No local dependency patches. Package licenses are upstream metadata, not the Matterweave project license.",
    "packages": packages,
}
(ROOT / "docs/dependencies.json").write_text(json.dumps(report, indent=2) + "\n")
print(f"Recorded {len(packages)} locked upstream packages in docs/dependencies.json")
