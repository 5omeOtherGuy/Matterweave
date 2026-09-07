#!/usr/bin/env python3
"""Refresh upstream Cargo provenance from locked metadata (Python 3.11+)."""
import hashlib
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
    "note": "Locked Cargo dependencies including host/build/other-platform packages. Upstream licenses do not select a Matterweave license. Vendored patches are listed separately.",
    "vendored": [{
        "name": "winit", "version": "0.30.12",
        "source": "https://crates.io/api/v1/crates/winit/0.30.12/download",
        "upstream_archive_sha256": "c66d4b9ed69c4009f6321f762d6e61ad8a2389cd431b97cb1e146812e9e6c732",
        "license": "Apache-2.0", "path": "vendor/winit",
        "patch_sha256": hashlib.sha256((ROOT / "vendor/winit/MATTERWEAVE.patch").read_bytes()).hexdigest(),
    }],
    "packages": packages,
}
(ROOT / "docs/dependencies.json").write_text(json.dumps(report, indent=2) + "\n")
print(f"Recorded {len(packages)} locked upstream packages in docs/dependencies.json")
