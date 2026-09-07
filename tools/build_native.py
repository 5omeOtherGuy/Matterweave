#!/usr/bin/env python3
"""Build the pinned Rust native library; called by Gradle or directly on Linux."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
NDK = "28.2.13676358"
TARGET = "aarch64-linux-android"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", default=os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT"))
    parser.add_argument("--profile", choices=("dev", "release"), default="dev")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not args.sdk:
        parser.error("set ANDROID_HOME to an Android SDK containing NDK " + NDK)
    if sys.platform != "linux":
        parser.error("the documented MVP build host is Linux x86_64")
    toolchain = Path(args.sdk).resolve() / "ndk" / NDK / "toolchains/llvm/prebuilt/linux-x86_64/bin"
    compiler = toolchain / "aarch64-linux-android28-clang"
    if not compiler.is_file():
        parser.error(f"missing {compiler}; install 'ndk;{NDK}' with sdkmanager")
    env = os.environ.copy()
    env["CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"] = str(compiler)
    env["CC_aarch64_linux_android"] = str(compiler)
    env["AR_aarch64_linux_android"] = str(toolchain / "llvm-ar")
    env.setdefault("CARGO_BUILD_JOBS", "2")
    # Both ELF LOAD segments and the APK zip entries must support 16 KiB pages.
    flags = env.get("CARGO_ENCODED_RUSTFLAGS", "")
    if not flags and env.get("RUSTFLAGS"):
        import shlex
        flags = "\x1f".join(shlex.split(env["RUSTFLAGS"]))
    env["CARGO_ENCODED_RUSTFLAGS"] = (flags + "\x1f" if flags else "") + "-Clink-arg=-Wl,-z,max-page-size=16384"
    subprocess.run(["cargo", "build", "--locked", "-p", "matterweave-explorer", "--lib",
                    "--target", TARGET, "--profile", args.profile], cwd=ROOT, env=env, check=True)
    target_dir = Path(env.get("CARGO_TARGET_DIR", ROOT / "target"))
    if not target_dir.is_absolute():
        target_dir = ROOT / target_dir
    library = target_dir / TARGET / ("debug" if args.profile == "dev" else "release") / "libmatterweave_explorer.so"
    output = args.out.resolve() / "arm64-v8a"
    output.mkdir(parents=True, exist_ok=True)
    shutil.copy2(library, output / library.name)
    print(f"Native library: {output / library.name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
