#!/usr/bin/env python3
"""Install the pinned Linux Android build tools into an explicitly chosen SDK."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import subprocess
import tempfile
import urllib.request
import zipfile

ARCHIVE = "https://dl.google.com/android/repository/commandlinetools-linux-13114758_latest.zip"
SHA256 = "7ec965280a073311c339e571cd5de778b9975026cfcbe79f2b1cdcb1e15317ee"
PACKAGES = ("platforms;android-35", "build-tools;35.0.0", "ndk;28.2.13676358", "platform-tools")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--accept-licenses", action="store_true", help="accept SDK licenses non-interactively (e.g. CI)")
    args = parser.parse_args()
    sdk = args.sdk.resolve()
    manager = sdk / "cmdline-tools/19.0/bin/sdkmanager"
    if not manager.is_file():
        sdk.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="matterweave-sdk-", dir=sdk) as directory:
            archive = Path(directory) / "tools.zip"
            print(f"Downloading {ARCHIVE}", flush=True)
            urllib.request.urlretrieve(ARCHIVE, archive)
            with archive.open("rb") as downloaded:
                if hashlib.file_digest(downloaded, "sha256").hexdigest() != SHA256:
                    raise RuntimeError("Android command-line tools checksum mismatch")
            with zipfile.ZipFile(archive) as bundle:
                bundle.extractall(directory)
            destination = manager.parents[1]
            destination.parent.mkdir(parents=True, exist_ok=True)
            if destination.exists():
                raise RuntimeError(f"incomplete existing tools at {destination}; inspect before replacing")
            (Path(directory) / "cmdline-tools").rename(destination)
            for executable in (destination / "bin").iterdir():
                executable.chmod(executable.stat().st_mode | 0o111)
    command = [str(manager), f"--sdk_root={sdk}"]
    subprocess.run(command + ["--licenses"], input="y\n" * 200 if args.accept_licenses else None,
                   text=True, check=True)
    subprocess.run(command + list(PACKAGES), check=True)
    subprocess.run(["rustup", "target", "add", "--toolchain", "1.96.0", "aarch64-linux-android"], check=True)
    print(f"SDK ready. Set ANDROID_HOME={sdk}")


if __name__ == "__main__":
    main()
