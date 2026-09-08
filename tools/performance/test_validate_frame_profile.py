"""Unittest suite for tools/performance/validate_frame_profile.py (CSV v2).

ALL fixtures in this file are explicitly SYNTHETIC hand-built row lists and
byte strings, except the column contract which is pinned against
apps/explorer/src/metrics.rs. No real device measurements appear here; real
private captures are checked manually via the CLI, not as CI inputs.
Expected summary values are computed independently by hand.

Run: python3 -m unittest discover -s tools/performance -p 'test_*.py'
(or: python3 tools/performance/test_validate_frame_profile.py)
"""

import csv
import hashlib
import json
import os
import re
import subprocess
import sys
import unittest
from unittest.mock import patch

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import validate_frame_profile as vfp  # noqa: E402

HEADER = "#matterweave-frame-capture schema_version=2"

# Pinned contract, also asserted name-for-name against metrics.rs.
COLUMNS = [
    "draw_attempt_id", "renderer_epoch", "presented_count", "result",
    "submitted_gpu_frame_id", "completed_gpu_frame_id",
    "completed_gpu_renderer_epoch", "draw_interval_wall_ms", "main_wall_ms",
    "main_cpu_busy_ms", "stream_request_elapsed_ms", "physics_wall_ms",
    "mesh_sync_wall_ms", "mesh_sync_fence_wait_wall_ms",
    "dynamic_mesh_build_wall_ms", "dynamic_upload_wall_ms", "render_wall_ms",
    "save_wall_ms", "render_fence_wait_wall_ms", "acquire_wall_ms",
    "present_wall_ms", "gpu_prev_render_ms", "gpu_prev_shadow_ms",
    "gpu_prev_shadows", "gpu_prev_shadow_map_size", "mesh_sync_fence_waits",
    "physics_fixed_steps", "voxel_bodies_total", "voxel_bodies_active",
    "voxel_bodies_sleeping", "voxel_bodies_not_simulated",
    "chunk_mesh_uploads", "dynamic_mesh_builds", "dynamic_mesh_uploads",
    "save_attempts", "save_failures", "shadow_caster_meshes",
]
assert len(COLUMNS) == 37

REQUIRED = {"draw_attempt_id", "renderer_epoch", "presented_count", "result"}
OPTIONAL = [c for c in COLUMNS if c not in REQUIRED]
assert len(OPTIONAL) == 33

SUMMARY_KEYS = {
    "schema_version", "row_count", "renderer_epochs", "missing_attempts",
    "unmatched_completions", "uncompleted_submissions", "missing_cells",
    "missing_cells_total", "raw_sha256", "evidence",
}


def synthetic_row(**over):
    cells = {c: "" for c in COLUMNS}
    cells.update({
        "draw_attempt_id": "1", "renderer_epoch": "1", "presented_count": "1",
        "result": "presented", "submitted_gpu_frame_id": "1",
    })
    cells.update(over)
    return [cells[c] for c in COLUMNS]


def synthetic_contract_rows():
    return [
        synthetic_row(draw_attempt_id="1", presented_count="1",
                      submitted_gpu_frame_id="1", main_wall_ms="27.3845",
                      shadow_caster_meshes="1"),
        synthetic_row(draw_attempt_id="2", presented_count="1",
                      result="retry", submitted_gpu_frame_id=""),
        synthetic_row(draw_attempt_id="3", presented_count="1",
                      result="retry", submitted_gpu_frame_id="2",
                      completed_gpu_frame_id="1",
                      completed_gpu_renderer_epoch="1",
                      gpu_prev_render_ms="3.3911", gpu_prev_shadows="1",
                      gpu_prev_shadow_map_size="2048",
                      shadow_caster_meshes="2"),
        synthetic_row(draw_attempt_id="4", presented_count="1",
                      result="out_of_memory", submitted_gpu_frame_id=""),
        synthetic_row(draw_attempt_id="9007199254740993",
                      presented_count="9007199254740002",
                      result="presented",
                      submitted_gpu_frame_id="9007199254740999",
                      completed_gpu_frame_id="2",
                      completed_gpu_renderer_epoch="1",
                      shadow_caster_meshes="3"),
        synthetic_row(draw_attempt_id="9007199254740994", renderer_epoch="2",
                      presented_count="9007199254740003",
                      result="presented", submitted_gpu_frame_id="1",
                      voxel_bodies_total="29", voxel_bodies_active="29",
                      voxel_bodies_sleeping="0",
                      voxel_bodies_not_simulated="0", save_attempts="1",
                      save_failures="0", shadow_caster_meshes="5"),
    ]


def synthetic_gap_rows():
    # draw1 submits 1; draw2 is missing; draw3 submits 3 and observes
    # completion 2, whose submission row went missing with draw2.
    return [
        synthetic_row(draw_attempt_id="1", presented_count="1",
                      submitted_gpu_frame_id="1"),
        synthetic_row(draw_attempt_id="3", presented_count="3",
                      result="presented", submitted_gpu_frame_id="3",
                      completed_gpu_frame_id="2",
                      completed_gpu_renderer_epoch="1",
                      gpu_prev_render_ms="1.0000"),
    ]


class TmpCase(unittest.TestCase):
    def setUp(self):
        self.dir = os.path.join(
            os.environ.get("TMPDIR", "/tmp"), "vfp-test-%d" % os.getpid())
        os.makedirs(self.dir, exist_ok=True)
        self.n = 0

    def write_text(self, rows):
        self.n += 1
        path = os.path.join(self.dir, "cap-%d.csv" % self.n)
        with open(path, "w", newline="") as f:
            f.write(HEADER + "\n")
            f.write(",".join(COLUMNS) + "\n")
            csv.writer(f).writerows(rows)
        return path

    def write_bytes(self, data):
        self.n += 1
        path = os.path.join(self.dir, "cap-%d.csv" % self.n)
        with open(path, "wb") as f:
            f.write(data)
        return path


class TestContract(TmpCase):
    def test_columns_match_rust_source(self):
        metrics = os.path.join(HERE, "..", "..", "apps", "explorer",
                               "src", "metrics.rs")
        with open(metrics) as f:
            text = f.read()
        m = re.search(r"pub const COLUMNS: &\[&str\] = &\[(.*?)\];", text,
                      re.S)
        self.assertIsNotNone(m, "COLUMNS block not found in metrics.rs")
        self.assertEqual(re.findall(r'"([^"]+)"', m.group(1)), COLUMNS)
        self.assertEqual(vfp.COLUMNS, COLUMNS)
        self.assertEqual(vfp.SCHEMA_VERSION, 2)

    def test_positive_contract_summary(self):
        summary = vfp.validate_profile(
            self.write_text(synthetic_contract_rows()))
        self.assertEqual(summary["schema_version"], 2)
        self.assertEqual(summary["row_count"], 6)
        self.assertEqual(summary["renderer_epochs"], [1, 2])
        self.assertEqual(summary["evidence"],
                         "capture_format_and_identity_only")
        self.assertEqual(summary["missing_attempts"],
                         9007199254740993 - 4 - 1)
        # Every observed completion joins a known submission.
        self.assertEqual(summary["unmatched_completions"], 0)
        # Submissions never completed: (1,9007199254740999) and (2,1).
        self.assertEqual(summary["uncompleted_submissions"], 2)
        self.assertGreater(summary["missing_cells_total"], 0)
        self.assertTrue(re.fullmatch(r"[0-9a-f]{64}", summary["raw_sha256"]))

    def test_summary_keys_exact(self):
        summary = vfp.validate_profile(
            self.write_text([synthetic_row()]))
        self.assertEqual(set(summary), SUMMARY_KEYS)
        self.assertEqual(set(summary["missing_cells"]), set(OPTIONAL))

    def test_missingness_per_column(self):
        summary = vfp.validate_profile(self.write_text(
            [synthetic_row(main_wall_ms="1.0000",
                           shadow_caster_meshes="1")]))
        missing = summary["missing_cells"]
        self.assertEqual(missing["main_wall_ms"], 0)
        self.assertEqual(missing["shadow_caster_meshes"], 0)
        self.assertEqual(missing["main_cpu_busy_ms"], 1)
        self.assertEqual(missing["gpu_prev_render_ms"], 1)
        self.assertEqual(missing["save_attempts"], 1)
        # 33 optional columns; submitted id, main_wall_ms and
        # shadow_caster_meshes filled.
        self.assertEqual(summary["missing_cells_total"], 30)
        self.assertEqual(sum(missing.values()), 30)

    def test_exact_large_ids(self):
        summary = vfp.validate_profile(self.write_text([synthetic_row(
            draw_attempt_id="9007199254740993",
            renderer_epoch="9007199254740995",
            presented_count="9007199254740993",
            submitted_gpu_frame_id="18446744073709551615",
            shadow_caster_meshes="32")]))
        self.assertEqual(summary["row_count"], 1)
        self.assertEqual(summary["unmatched_completions"], 0)
        self.assertEqual(summary["uncompleted_submissions"], 1)

    def test_id_overflow_rejected(self):
        bad_cells = [
            ("draw_attempt_id", "18446744073709551616"),
            ("submitted_gpu_frame_id", "18446744073709551616"),
            ("completed_gpu_frame_id", "18446744073709551616"),
            ("presented_count", "18446744073709551616"),
            ("shadow_caster_meshes", "4294967296"),
            ("physics_fixed_steps", "4294967296"),
            ("draw_attempt_id", "-1"), ("draw_attempt_id", "1.0"),
            ("draw_attempt_id", "0"), ("renderer_epoch", "0"),
            ("submitted_gpu_frame_id", "-3"),
            ("submitted_gpu_frame_id", "12a"),
        ]
        for col, bad in bad_cells:
            with self.subTest(col=col, bad=bad):
                with self.assertRaises(ValueError):
                    vfp.validate_profile(
                        self.write_text([synthetic_row(**{col: bad})]))

    def test_raw_sha256_matches_bytes(self):
        path = self.write_text(synthetic_contract_rows())
        with open(path, "rb") as f:
            digest = hashlib.sha256(f.read()).hexdigest()
        self.assertEqual(vfp.validate_profile(path)["raw_sha256"], digest)


class TestIdentity(TmpCase):
    def test_presented_requires_submission(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text(
                [synthetic_row(result="presented",
                               submitted_gpu_frame_id="")]))
        vfp.validate_profile(self.write_text(
            [synthetic_row(result="retry", submitted_gpu_frame_id="",
                           presented_count="0")]))
        vfp.validate_profile(self.write_text(
            [synthetic_row(result="out_of_memory",
                           submitted_gpu_frame_id="1", presented_count="0",
                           shadow_caster_meshes="0")]))

    def test_no_submission_implies_no_shadow_casters(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text(
                [synthetic_row(result="retry", submitted_gpu_frame_id="",
                               presented_count="0",
                               shadow_caster_meshes="3")]))

    def test_submission_increase_and_epoch_restart(self):
        rows = [
            synthetic_row(draw_attempt_id="1", presented_count="1",
                          submitted_gpu_frame_id="1"),
            synthetic_row(draw_attempt_id="2", presented_count="2",
                          submitted_gpu_frame_id="2",
                          completed_gpu_frame_id="1",
                          completed_gpu_renderer_epoch="1",
                          gpu_prev_render_ms="1.0000"),
            synthetic_row(draw_attempt_id="3", renderer_epoch="2",
                          presented_count="3", submitted_gpu_frame_id="1",
                          shadow_caster_meshes=""),
        ]
        self.assertEqual(
            vfp.validate_profile(self.write_text(rows))["renderer_epochs"],
            [1, 2])
        dup = [
            synthetic_row(draw_attempt_id="1", presented_count="1",
                          submitted_gpu_frame_id="2"),
            synthetic_row(draw_attempt_id="2", presented_count="2",
                          submitted_gpu_frame_id="2"),
        ]
        with self.assertRaises(ValueError) as ctx:
            vfp.validate_profile(self.write_text(dup))
        self.assertIn("not increasing", str(ctx.exception))

    def test_completion_both_or_neither(self):
        for kw in [{"completed_gpu_frame_id": ""},
                   {"completed_gpu_renderer_epoch": ""}]:
            base = dict(draw_attempt_id="2", presented_count="2",
                        submitted_gpu_frame_id="2",
                        completed_gpu_frame_id="1",
                        completed_gpu_renderer_epoch="1",
                        gpu_prev_render_ms="1.0000")
            base.update(kw)
            rows = [synthetic_row(draw_attempt_id="1", presented_count="1",
                                  submitted_gpu_frame_id="1"),
                    synthetic_row(**base)]
            with self.subTest(kw=kw):
                with self.assertRaises(ValueError):
                    vfp.validate_profile(self.write_text(rows))

    def test_future_completion_rejected(self):
        rows = [synthetic_row(draw_attempt_id="1", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", presented_count="2",
                              submitted_gpu_frame_id="2",
                              completed_gpu_frame_id="1",
                              completed_gpu_renderer_epoch="2",
                              gpu_prev_render_ms="1.0000")]
        with self.assertRaises(ValueError) as ctx:
            vfp.validate_profile(self.write_text(rows))
        self.assertIn("future", str(ctx.exception))

    def test_duplicate_completion_rejected(self):
        rows = [synthetic_row(draw_attempt_id="1", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", presented_count="2",
                              submitted_gpu_frame_id="2",
                              completed_gpu_frame_id="1",
                              completed_gpu_renderer_epoch="1",
                              gpu_prev_render_ms="1.0000"),
                synthetic_row(draw_attempt_id="3", presented_count="3",
                              submitted_gpu_frame_id="3",
                              completed_gpu_frame_id="1",
                              completed_gpu_renderer_epoch="1",
                              gpu_prev_render_ms="1.0000")]
        with self.assertRaises(ValueError) as ctx:
            vfp.validate_profile(self.write_text(rows))
        self.assertIn("duplicate", str(ctx.exception))

    def test_orphan_completion_without_gap_rejected(self):
        # Completion (1,9) was never submitted and no draw attempts are
        # missing, so no submission row can be hiding: invalid.
        rows = [synthetic_row(draw_attempt_id="1", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", renderer_epoch="2",
                              presented_count="2", submitted_gpu_frame_id="1",
                              completed_gpu_frame_id="9",
                              completed_gpu_renderer_epoch="1",
                              gpu_prev_render_ms="1.0000",
                              shadow_caster_meshes="")]
        with self.assertRaises(ValueError) as ctx:
            vfp.validate_profile(self.write_text(rows))
        self.assertIn("orphan", str(ctx.exception))

    def test_late_completion_reports_not_before_submission(self):
        # Second row submits 2 and observes completion 2: the completion is
        # not before this row's own submission, so the exact causal reason
        # must be reported (not the duplicate-submission path).
        rows = [synthetic_row(draw_attempt_id="1", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", presented_count="2",
                              submitted_gpu_frame_id="2",
                              completed_gpu_frame_id="2",
                              completed_gpu_renderer_epoch="1",
                              gpu_prev_render_ms="1.0000")]
        with self.assertRaises(ValueError) as ctx:
            vfp.validate_profile(self.write_text(rows))
        self.assertIn("not before submission", str(ctx.exception))

    def test_gap_completion_is_unmatched_not_orphan(self):
        summary = vfp.validate_profile(
            self.write_text(synthetic_gap_rows()))
        self.assertEqual(summary["missing_attempts"], 1)
        # Observed completion 2 has no submission row: genuinely missing.
        self.assertEqual(summary["unmatched_completions"], 1)
        # Submitted but never completed: 1 and 3.
        self.assertEqual(summary["uncompleted_submissions"], 2)

    def test_single_presented_row_counts(self):
        summary = vfp.validate_profile(
            self.write_text([synthetic_row()]))
        self.assertEqual(summary["unmatched_completions"], 0)
        self.assertEqual(summary["uncompleted_submissions"], 1)

    def test_gpu_prev_blank_without_completion(self):
        for col, val in [("gpu_prev_render_ms", "1.0000"),
                         ("gpu_prev_shadow_ms", "0.5000"),
                         ("gpu_prev_shadows", "1"),
                         ("gpu_prev_shadow_map_size", "2048")]:
            with self.subTest(col=col):
                with self.assertRaises(ValueError):
                    vfp.validate_profile(
                        self.write_text([synthetic_row(**{col: val})]))


class TestOrderingCounts(TmpCase):
    def test_draw_ids_increase_and_epochs_not_decrease(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text([
                synthetic_row(draw_attempt_id="2", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", presented_count="2",
                              submitted_gpu_frame_id="2")]))
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text([
                synthetic_row(draw_attempt_id="1", renderer_epoch="2",
                              presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", renderer_epoch="1",
                              presented_count="2",
                              submitted_gpu_frame_id="1",
                              shadow_caster_meshes="")]))

    def test_presented_delta_rules(self):
        def rows(count1, count2, result2="presented", sub2="2", draw2="2"):
            return [
                synthetic_row(draw_attempt_id="1", presented_count=count1,
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id=draw2, presented_count=count2,
                              result=result2, submitted_gpu_frame_id=sub2),
            ]
        with self.assertRaises(ValueError):  # presented must advance by 1
            vfp.validate_profile(self.write_text(rows("1", "1")))
        with self.assertRaises(ValueError):  # retry must not advance
            vfp.validate_profile(self.write_text(
                rows("1", "2", result2="retry", sub2="")))
        with self.assertRaises(ValueError):  # never backwards
            vfp.validate_profile(self.write_text(
                rows("2", "1", result2="retry", sub2="")))
        # Gap allows unknown intervening increments, never overfull.
        vfp.validate_profile(self.write_text([
            synthetic_row(draw_attempt_id="1", presented_count="1",
                          submitted_gpu_frame_id="1"),
            synthetic_row(draw_attempt_id="4", presented_count="4",
                          result="presented", submitted_gpu_frame_id="2")]))
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text([
                synthetic_row(draw_attempt_id="1", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="4", presented_count="5",
                              result="presented",
                              submitted_gpu_frame_id="2")]))
        gap = vfp.validate_profile(self.write_text([
            synthetic_row(draw_attempt_id="1", presented_count="1",
                          submitted_gpu_frame_id="1"),
            synthetic_row(draw_attempt_id="4", presented_count="2",
                          result="retry", submitted_gpu_frame_id="")]))
        self.assertEqual(gap["missing_attempts"], 2)

    def test_numeric_invalid(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(
                self.write_text([synthetic_row(result="dropped")]))
        for col, bad in [("main_wall_ms", "-1"), ("main_wall_ms", "nan"),
                         ("main_wall_ms", "inf"), ("main_wall_ms", "abc"),
                         ("gpu_prev_shadows", "2"),
                         ("gpu_prev_shadows", "-1"),
                         ("gpu_prev_shadows", "true")]:
            with self.subTest(col=col, bad=bad):
                with self.assertRaises(ValueError):
                    vfp.validate_profile(
                        self.write_text([synthetic_row(**{col: bad})]))

    def test_body_partition_and_saves(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text(
                [synthetic_row(voxel_bodies_total="29",
                               voxel_bodies_active="29",
                               voxel_bodies_sleeping="0",
                               voxel_bodies_not_simulated="1")]))
        vfp.validate_profile(self.write_text(
            [synthetic_row(voxel_bodies_total="29",
                           voxel_bodies_active="29",
                           voxel_bodies_sleeping="0",
                           voxel_bodies_not_simulated="0")]))
        vfp.validate_profile(
            self.write_text([synthetic_row(voxel_bodies_total="29")]))
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text(
                [synthetic_row(save_attempts="1", save_failures="2")]))


class TestMalformed(TmpCase):
    def test_bad_schema_header_column_width_empty(self):
        path = os.path.join(self.dir, "bad-schema.csv")
        with open(path, "w") as f:
            f.write("#matterweave-frame-capture schema_version=1\n"
                    + ",".join(COLUMNS) + "\n")
        with self.assertRaises(ValueError):
            vfp.validate_profile(path)
        cols = list(COLUMNS)
        cols[0], cols[1] = cols[1], cols[0]
        path = os.path.join(self.dir, "bad-header.csv")
        with open(path, "w") as f:
            f.write(HEADER + "\n" + ",".join(cols) + "\n")
        with self.assertRaises(ValueError):
            vfp.validate_profile(path)
        with self.assertRaises(ValueError):
            vfp.validate_profile(
                self.write_bytes((HEADER + "\n" + ",".join(COLUMNS)
                                  + "\n1,1,1\n").encode()))
        with self.assertRaises(ValueError):  # headers but no data rows
            vfp.validate_profile(
                self.write_bytes((HEADER + "\n" + ",".join(COLUMNS)
                                  + "\n").encode()))
        with self.assertRaises(ValueError):  # completely empty
            vfp.validate_profile(self.write_bytes(b""))

    def test_long_physical_line_rejected(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_bytes(
                (HEADER + "\n" + ",".join(COLUMNS) + "\n").encode()
                + b"9" * 20000 + b"\n"))

    def test_unclosed_final_quote_rejected(self):
        # Lead repro: lenient csv ACCEPTED a final '"' cell; strict parsing
        # must reject it as a ValueError (no csv.Error leak, no traceback).
        cells = synthetic_row()
        cells[-1] = '"'
        data = (HEADER + "\n" + ",".join(COLUMNS) + "\n"
                + ",".join(cells) + "\n").encode()
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_bytes(data))

    def test_embedded_cr_rejected_as_value_error(self):
        # Lead repro: a raw CR inside an unquoted duration cell raised a raw
        # csv.Error; the API must translate it to ValueError.
        cells = synthetic_row()
        cells[COLUMNS.index("draw_interval_wall_ms")] = "1\r2"
        data = (HEADER + "\n" + ",".join(COLUMNS) + "\n"
                + ",".join(cells) + "\n").encode()
        try:
            vfp.validate_profile(self.write_bytes(data))
        except ValueError:
            pass
        else:
            self.fail("embedded CR did not raise ValueError")

    def test_physical_line_byte_bounds(self):
        base = ",".join(synthetic_row(main_wall_ms="1.0000")).encode()
        prefix = (HEADER + "\n" + ",".join(COLUMNS) + "\n").encode()
        for ending in (b"", b"\n", b"\r\n"):
            with self.subTest(ending=ending):
                padding = vfp.MAX_LINE_BYTES - len(base) - len(ending)
                at = base.replace(b"1.0000", b"1." + b"0" * (4 + padding), 1) + ending
                over = base.replace(b"1.0000", b"1." + b"0" * (5 + padding), 1) + ending
                self.assertEqual(len(at), vfp.MAX_LINE_BYTES)
                self.assertEqual(len(over), vfp.MAX_LINE_BYTES + 1)
                path = self.write_bytes(prefix + at)
                summary = vfp.validate_profile(path)
                self.assertEqual(summary["row_count"], 1)
                self.assertEqual(summary["raw_sha256"], hashlib.sha256(prefix + at).hexdigest())
                with self.assertRaisesRegex(ValueError, "physical line"):
                    vfp.validate_profile(self.write_bytes(prefix + over))

    def test_read_failure_is_value_error(self):
        from unittest.mock import patch
        with patch("builtins.open") as opened:
            opened.return_value.readline.side_effect = OSError("synthetic EIO")
            with self.assertRaisesRegex(ValueError, "read"):
                vfp.validate_profile(Path("synthetic-capture.csv"))

    def test_row_and_byte_caps(self):
        old_rows, old_bytes = vfp.MAX_ROWS, vfp.MAX_TOTAL_BYTES
        self.addCleanup(setattr, vfp, "MAX_ROWS", old_rows)
        self.addCleanup(setattr, vfp, "MAX_TOTAL_BYTES", old_bytes)
        vfp.MAX_ROWS = 2
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text([
                synthetic_row(draw_attempt_id="1", presented_count="1",
                              submitted_gpu_frame_id="1"),
                synthetic_row(draw_attempt_id="2", presented_count="2",
                              submitted_gpu_frame_id="2"),
                synthetic_row(draw_attempt_id="3", presented_count="3",
                              submitted_gpu_frame_id="3")]))
        vfp.MAX_ROWS = old_rows
        vfp.MAX_TOTAL_BYTES = 64
        with self.assertRaises(ValueError):
            vfp.validate_profile(self.write_text([synthetic_row()]))

    def test_descriptors_rejected_before_open(self):
        for bad in (0, 1, False, True, None, ["x"]):
            with self.subTest(bad=bad):
                with patch("builtins.open") as mocked:
                    with self.assertRaises(ValueError):
                        vfp.validate_profile(bad)
                mocked.assert_not_called()

    def test_missing_file_is_value_error(self):
        with self.assertRaises(ValueError):
            vfp.validate_profile(os.path.join(self.dir, "absent.csv"))


class TestCLI(TmpCase):
    def run_cli(self, *args):
        return subprocess.run(
            [sys.executable, os.path.join(HERE, "validate_frame_profile.py"),
             *args], capture_output=True, text=True)

    def test_cli_valid(self):
        proc = self.run_cli(self.write_text(synthetic_contract_rows()))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        summary = json.loads(proc.stdout)
        self.assertEqual(summary["row_count"], 6)
        self.assertEqual(summary["evidence"],
                         "capture_format_and_identity_only")
        self.assertEqual(summary["unmatched_completions"], 0)
        self.assertEqual(summary["uncompleted_submissions"], 2)

    def test_cli_invalid_concise_and_read_only(self):
        bad = self.write_text([synthetic_row(result="bogus")])
        with open(bad, "rb") as f:
            before = f.read()
        proc = self.run_cli(bad)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr + proc.stdout)
        with open(bad, "rb") as f:
            self.assertEqual(f.read(), before)
        proc = self.run_cli(os.path.join(self.dir, "absent.csv"))
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr + proc.stdout)

    def test_cli_csv_error_concise(self):
        cells = synthetic_row()
        cells[COLUMNS.index("draw_interval_wall_ms")] = "1\r2"
        data = (HEADER + "\n" + ",".join(COLUMNS) + "\n"
                + ",".join(cells) + "\n").encode()
        proc = self.run_cli(self.write_bytes(data))
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr + proc.stdout)
        self.assertIn("error:", proc.stderr)


if __name__ == "__main__":
    unittest.main()
