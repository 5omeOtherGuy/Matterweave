"""Strict format/identity validator for matterweave frame capture CSV schema v2.

Source of truth: apps/explorer/src/metrics.rs (COLUMNS, write rules) and
docs/performance/measurement-v2.md. Capture inputs are read-only; this tool
never mutates them.

API: validate_profile(path) -> summary dict, or raise ValueError on any
contract violation. CLI prints the summary as JSON; on failure it prints a
concise error without a traceback and exits nonzero.

Identity-only scope: checks format, exact integer identities, counter deltas
and submission/completion joins. Makes no mobile performance, scanout or
overhead claims, and deliberately does NOT compare CPU busy time against wall
time (different clocks).

Completion accounting: an observed completion whose submission row is missing
is reported under unmatched_completions only when earlier draw attempts are
genuinely missing (it may be hiding in the gap); with no missing attempts it
is an invalid orphan. uncompleted_submissions separately counts submitted
pairs never observed as completed (unfinished or unobserved work, not proven
GPU failure). No join is ever fabricated.
"""
from __future__ import annotations

import csv
import hashlib
import json
import math
import os
import re
import sys

SCHEMA_VERSION = 2
HEADER_MAGIC = "#matterweave-frame-capture schema_version=2"

# Pinned exact column order; regression-tested against metrics::COLUMNS.
COLUMNS = [
    "draw_attempt_id",
    "renderer_epoch",
    "presented_count",
    "result",
    "submitted_gpu_frame_id",
    "completed_gpu_frame_id",
    "completed_gpu_renderer_epoch",
    "draw_interval_wall_ms",
    "main_wall_ms",
    "main_cpu_busy_ms",
    "stream_request_elapsed_ms",
    "physics_wall_ms",
    "mesh_sync_wall_ms",
    "mesh_sync_fence_wait_wall_ms",
    "dynamic_mesh_build_wall_ms",
    "dynamic_upload_wall_ms",
    "render_wall_ms",
    "save_wall_ms",
    "render_fence_wait_wall_ms",
    "acquire_wall_ms",
    "present_wall_ms",
    "gpu_prev_render_ms",
    "gpu_prev_shadow_ms",
    "gpu_prev_shadows",
    "gpu_prev_shadow_map_size",
    "mesh_sync_fence_waits",
    "physics_fixed_steps",
    "voxel_bodies_total",
    "voxel_bodies_active",
    "voxel_bodies_sleeping",
    "voxel_bodies_not_simulated",
    "chunk_mesh_uploads",
    "dynamic_mesh_builds",
    "dynamic_mesh_uploads",
    "save_attempts",
    "save_failures",
    "shadow_caster_meshes",
]

# Bounded streaming: never stat-then-read; readline with a physical line cap,
# a total byte cap and a row cap.
MAX_LINE_BYTES = 16 * 1024
MAX_TOTAL_BYTES = 128 * 1024 * 1024
MAX_ROWS = 240_000

U64_MAX = 2**64 - 1
U32_MAX = 2**32 - 1

RESULTS = ("presented", "retry", "out_of_memory")

_UINT_RE = re.compile(r"[0-9]+")
_FLOAT_RE = re.compile(r"[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?")

IDX = {name: i for i, name in enumerate(COLUMNS)}

REQUIRED_COLUMNS = [
    "draw_attempt_id",
    "renderer_epoch",
    "presented_count",
    "result",
]
OPTIONAL_COLUMNS = [name for name in COLUMNS if name not in REQUIRED_COLUMNS]

DURATION_COLUMNS = COLUMNS[IDX["draw_interval_wall_ms"]:IDX["gpu_prev_shadow_ms"] + 1]
COUNT_COLUMNS = [
    "gpu_prev_shadow_map_size",
    "mesh_sync_fence_waits",
    "physics_fixed_steps",
    "voxel_bodies_total",
    "voxel_bodies_active",
    "voxel_bodies_sleeping",
    "voxel_bodies_not_simulated",
    "chunk_mesh_uploads",
    "dynamic_mesh_builds",
    "dynamic_mesh_uploads",
    "save_attempts",
    "save_failures",
    "shadow_caster_meshes",
]
GPU_PREV_COLUMNS = [
    "gpu_prev_render_ms",
    "gpu_prev_shadow_ms",
    "gpu_prev_shadows",
    "gpu_prev_shadow_map_size",
]


def _fail(message):
    raise ValueError(message)


def _parse_uint(cell, name, lineno, limit):
    if not _UINT_RE.fullmatch(cell):
        _fail(f"line {lineno}: {name} is not an exact decimal integer: {cell!r}")
    value = int(cell)
    if value > limit:
        _fail(f"line {lineno}: {name} overflows: {cell!r}")
    return value


def _parse_opt_uint(cell, name, lineno, limit):
    if cell == "":
        return None
    return _parse_uint(cell, name, lineno, limit)


def _parse_duration(cell, name, lineno):
    if cell == "":
        return None
    if not _FLOAT_RE.fullmatch(cell):
        _fail(f"line {lineno}: {name} is not a numeric duration: {cell!r}")
    value = float(cell)
    if not math.isfinite(value) or value < 0:
        _fail(f"line {lineno}: {name} is not finite and >= 0: {cell!r}")
    return value


def _parse_shadows(cell, lineno):
    if cell == "":
        return None
    if cell == "0":
        return False
    if cell == "1":
        return True
    _fail(f"line {lineno}: gpu_prev_shadows must be blank, 0 or 1: {cell!r}")


def _parse_csv_line(text, lineno):
    try:
        return next(csv.reader([text], strict=True))
    except csv.Error as error:
        _fail(f"line {lineno}: malformed CSV: {error}")


class _BoundedReader:
    """Bounded binary line reader that hashes raw bytes as they are read.

    Enforces the physical line cap and total byte cap without a separate
    stat or second pass, so raw_sha256 covers exactly the bytes consumed.
    The content length is checked on the raw bytes before stripping line
    endings, so LF- and CRLF-terminated lines share one bound.
    """

    def __init__(self, path):
        try:
            self.handle = open(path, "rb")
        except OSError as error:
            _fail(f"cannot open {path}: {error.strerror or error}")
        self.digest = hashlib.sha256()
        self.total = 0

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.handle.close()
        return False

    def __iter__(self):
        while True:
            # +3 admits the longest valid raw line: content plus CRLF.
            chunk = self.handle.readline(MAX_LINE_BYTES + 3)
            if not chunk:
                return
            self.total += len(chunk)
            if self.total > MAX_TOTAL_BYTES:
                _fail(f"capture exceeds {MAX_TOTAL_BYTES} bytes")
            self.digest.update(chunk)
            if chunk.endswith(b"\r\n"):
                content = chunk[:-2]
            elif chunk.endswith(b"\n"):
                content = chunk[:-1]
            else:
                # Final line without a terminator, or a truncated read of a
                # longer physical line; the length check decides.
                content = chunk
            if len(content) > MAX_LINE_BYTES:
                _fail("physical line exceeds 16 KiB")
            try:
                yield content.decode("utf-8")
            except UnicodeDecodeError:
                _fail("capture is not valid UTF-8")


def validate_profile(path):
    """Validate a schema v2 frame capture; return the summary dict."""
    if isinstance(path, (bool, int)):
        _fail(f"path must be a path, not {type(path).__name__}")
    try:
        path = os.fspath(path)
    except TypeError:
        _fail(f"path must be a path, not {type(path).__name__}")
    with _BoundedReader(path) as reader:
        lines = iter(reader)
        try:
            magic = next(lines)
        except StopIteration:
            _fail("capture is empty: missing schema header")
        if magic != HEADER_MAGIC:
            _fail(f"bad schema header: {magic!r}")
        try:
            header = next(lines)
        except StopIteration:
            _fail("capture is empty: missing column header")
        if _parse_csv_line(header, 2) != COLUMNS:
            _fail("column header does not match schema v2 column order")
        summary = _validate_rows(lines)
        summary["raw_sha256"] = reader.digest.hexdigest()
        return summary


def _validate_rows(lines):
    """Validate data rows from an iterator of physical lines."""
    row_count = 0
    missing_attempts = 0
    unmatched_completions = 0
    missing = {name: 0 for name in OPTIONAL_COLUMNS}
    epochs = set()
    submitted = set()  # (renderer_epoch, submitted_gpu_frame_id) pairs
    submitted_max = {}  # renderer_epoch -> max submitted id so far
    completed = set()  # observed (epoch, id) completion pairs
    prev_draw = 0
    prev_presented = 0
    prev_epoch = 0
    lineno = 2

    for text in lines:
        lineno += 1
        row_count += 1
        if row_count > MAX_ROWS:
            _fail(f"capture exceeds {MAX_ROWS} rows")
        cells = _parse_csv_line(text, lineno)
        if len(cells) != len(COLUMNS):
            _fail(f"line {lineno}: width {len(cells)} != {len(COLUMNS)}")
        cell = dict(zip(COLUMNS, cells))
        for name in OPTIONAL_COLUMNS:
            if cell[name] == "":
                missing[name] += 1

        draw_id = _parse_uint(cell["draw_attempt_id"], "draw_attempt_id",
                              lineno, U64_MAX)
        if draw_id < 1:
            _fail(f"line {lineno}: draw_attempt_id must be positive")
        epoch = _parse_uint(cell["renderer_epoch"], "renderer_epoch",
                             lineno, U64_MAX)
        if epoch < 1:
            _fail(f"line {lineno}: renderer_epoch must be positive")
        presented = _parse_uint(cell["presented_count"], "presented_count",
                                lineno, U64_MAX)
        result = cell["result"]
        if result not in RESULTS:
            _fail(f"line {lineno}: bad result: {result!r}")
        submitted_id = _parse_opt_uint(cell["submitted_gpu_frame_id"],
                                       "submitted_gpu_frame_id", lineno,
                                       U64_MAX)
        completed_id = _parse_opt_uint(cell["completed_gpu_frame_id"],
                                       "completed_gpu_frame_id", lineno,
                                       U64_MAX)
        completed_epoch = _parse_opt_uint(
            cell["completed_gpu_renderer_epoch"],
            "completed_gpu_renderer_epoch", lineno, U64_MAX)

        for name in DURATION_COLUMNS:
            _parse_duration(cell[name], name, lineno)
        _parse_shadows(cell["gpu_prev_shadows"], lineno)
        counts = {n: _parse_opt_uint(cell[n], n, lineno, U32_MAX)
                  for n in COUNT_COLUMNS}

        # Ordering: draw ids strictly increase, epochs never decrease.
        if draw_id <= prev_draw:
            _fail(f"line {lineno}: draw_attempt_id {draw_id} not after "
                  f"{prev_draw}")
        if epoch < prev_epoch:
            _fail(f"line {lineno}: renderer_epoch {epoch} goes backwards")
        gap = draw_id - prev_draw - 1
        missing_attempts += gap
        epochs.add(epoch)

        # Presented-count delta: consecutive rows advance exactly the current
        # row's increment; gaps allow only unknown intervening increments plus
        # the current result, never backwards and never overfull.
        increment = 1 if result == "presented" else 0
        delta = presented - prev_presented
        if delta < increment or delta - increment > gap:
            _fail(f"line {lineno}: presented_count {presented} inconsistent "
                  f"with {result} after {prev_presented}")

        # Presented attempts submitted GPU work; other outcomes may or may not.
        if result == "presented" and submitted_id is None:
            _fail(f"line {lineno}: presented attempt submitted nothing")
        # Without a submission there is no shadow work of this attempt.
        if submitted_id is None and counts["shadow_caster_meshes"] is not None:
            _fail(f"line {lineno}: shadow_caster_meshes without submission")
        # Submission ids strictly increase within each epoch; the counter
        # restarts per epoch so cross-epoch ids are distinct pairs.
        if submitted_id is not None:
            if submitted_id <= submitted_max.get(epoch, -1):
                _fail(f"line {lineno}: submitted_gpu_frame_id "
                      f"{submitted_id} not increasing in epoch {epoch}")
            submitted_max[epoch] = submitted_id
            pair = (epoch, submitted_id)
            if pair in completed:
                _fail(f"line {lineno}: submission reuses completed pair "
                      f"{pair}")
            submitted.add(pair)

        # Completion identity: both columns present or both absent, never from
        # a future epoch, and never at or after this row's own same-epoch
        # submission (causally impossible, even across gaps). Duplicates are
        # rejected. A completion of a never-submitted pair is an orphan
        # unless earlier draw attempts are missing, in which case its
        # submission row may be hiding in the gap: it is counted as
        # unmatched, never joined to a fabricated submission.
        if (completed_id is None) != (completed_epoch is None):
            _fail(f"line {lineno}: completion id/epoch must both be present "
                  f"or both absent")
        if completed_id is not None:
            if completed_epoch > epoch:
                _fail(f"line {lineno}: completion epoch {completed_epoch} "
                      f"is in the future")
            pair = (completed_epoch, completed_id)
            if (completed_epoch == epoch and submitted_id is not None
                    and completed_id >= submitted_id):
                _fail(f"line {lineno}: same-epoch completion {completed_id} "
                      f"is not before submission {submitted_id}")
            if pair in completed:
                _fail(f"line {lineno}: duplicate completion pair {pair}")
            if pair not in submitted:
                if missing_attempts == 0:
                    _fail(f"line {lineno}: orphan completion {pair}: never "
                          f"submitted")
                unmatched_completions += 1
            completed.add(pair)
        else:
            gpu_cells = [cell[n] for n in GPU_PREV_COLUMNS]
            if any(c != "" for c in gpu_cells):
                _fail(f"line {lineno}: gpu_prev_* set without a completion")
        # Missing timings are allowed; no check invents values for them.

        # Body partition balances when all four buckets are present.
        bodies = [counts["voxel_bodies_total"], counts["voxel_bodies_active"],
                  counts["voxel_bodies_sleeping"],
                  counts["voxel_bodies_not_simulated"]]
        if all(b is not None for b in bodies) and bodies[0] != sum(bodies[1:]):
            _fail(f"line {lineno}: voxel body partition does not balance")
        # Save failures are a subset of save attempts.
        attempts, failures = counts["save_attempts"], counts["save_failures"]
        if (attempts is not None and failures is not None
                and failures > attempts):
            _fail(f"line {lineno}: save_failures exceeds save_attempts")

        prev_draw, prev_presented, prev_epoch = draw_id, presented, epoch

    if row_count == 0:
        _fail("capture is empty: no data rows")
    return {
        "schema_version": SCHEMA_VERSION,
        "row_count": row_count,
        "renderer_epochs": sorted(epochs),
        "missing_attempts": missing_attempts,
        "unmatched_completions": unmatched_completions,
        "uncompleted_submissions": len(submitted - completed),
        "missing_cells": missing,
        "missing_cells_total": sum(missing.values()),
        "raw_sha256": "",
        "evidence": "capture_format_and_identity_only",
    }


def main(argv):
    if len(argv) != 2:
        print(f"usage: {argv[0]} <frame-profile-v2.csv>",
              file=sys.stderr)
        return 2
    try:
        summary = validate_profile(argv[1])
    except (ValueError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
