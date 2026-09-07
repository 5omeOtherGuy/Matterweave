#!/usr/bin/env python3
"""Verify ARM64 ELF and uncompressed APK library alignment; no device claims."""
from __future__ import annotations

import argparse
from pathlib import Path
import struct
import zipfile


def verify(path: Path) -> None:
    with zipfile.ZipFile(path) as apk, path.open("rb") as raw:
        libraries = [item for item in apk.infolist() if item.filename.endswith(".so")]
        if not libraries:
            raise ValueError("APK contains no native libraries")
        expected = "lib/arm64-v8a/libmatterweave_explorer.so"
        if expected not in {item.filename for item in libraries}:
            raise ValueError(f"missing {expected}")
        for info in libraries:
            if not info.filename.startswith("lib/arm64-v8a/"):
                raise ValueError(f"unexpected ABI: {info.filename}")
            raw.seek(info.header_offset)
            header = raw.read(30)
            if header[:4] != b"PK\x03\x04":
                raise ValueError("bad local zip header")
            name_len, extra_len = struct.unpack_from("<HH", header, 26)
            offset = info.header_offset + 30 + name_len + extra_len
            if info.compress_type != zipfile.ZIP_STORED or offset % 16384:
                raise ValueError(f"{info.filename}: must be stored and aligned to 16384 bytes")
            elf = apk.read(info)
            if elf[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", elf, 18)[0] != 183:
                raise ValueError(f"{info.filename}: not little-endian ARM64 ELF64")
            phoff = struct.unpack_from("<Q", elf, 32)[0]
            phsize, phcount = struct.unpack_from("<HH", elf, 54)
            if phsize != 56 or phoff + phcount * phsize > len(elf):
                raise ValueError("invalid ELF program header table")
            loads = []
            for index in range(phcount):
                record = struct.unpack_from("<IIQQQQQQ", elf, phoff + index * phsize)
                if record[0] == 1:
                    _, _, file_offset, vaddr, _, _, _, alignment = record
                    if alignment < 16384 or alignment & (alignment - 1) or (vaddr - file_offset) % 16384:
                        raise ValueError(f"{info.filename}: incompatible PT_LOAD alignment")
                    loads.append(alignment)
            if not loads:
                raise ValueError("no ELF LOAD segments")
            print(f"PASS {info.filename}: {len(elf)} bytes; zip offset {offset}; LOAD alignments {loads}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("apk", type=Path)
    arguments = parser.parse_args()
    verify(arguments.apk)
