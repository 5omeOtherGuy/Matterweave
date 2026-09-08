#!/usr/bin/env python3
"""Tests for the P00 read-only handoff validator (check_handoff.py).

Unittest suite, stdlib only. Covers the frozen v1 interface:
  validate_handoff(board: dict, submission: dict, artifact: pathlib.Path) -> None
plus the CLI: `check_handoff.py BOARD_JSON SUBMISSION_JSON ARTIFACT`.
"""
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

CHECK_HANDOFF = Path(__file__).with_name("check_handoff.py")

ARTIFACT_BYTES = b"binary git diff placeholder\n\x00\x01\x02partial work\n"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def make_board(run="performance-01", task="P00-handoff", attempt=1,
               owner="muse-p00-a1", state="running", **extra):
    board = {
        "schema_version": 1,
        "run": run,
        "tasks": {
            task: {"attempt": attempt, "owner": owner, "state": state},
        },
    }
    board.update(extra)
    return board


_MISSING = object()


def make_submission(run="performance-01", task="P00-handoff", attempt=1,
                    owner="muse-p00-a1", digest=_MISSING,
                    verified_inactive=True, evidence="lead supervision evidence: session log + process check",
                    **extra):
    submission = {
        "schema_version": 1,
        "run": run,
        "task": task,
        "attempt": attempt,
        "owner": owner,
        "artifact_sha256": sha256(ARTIFACT_BYTES) if digest is _MISSING else digest,
        "verified_inactive": verified_inactive,
        "evidence": evidence,
    }
    submission.update(extra)
    return submission


class HandoffTestBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        # Import the validator lazily so a missing implementation is a clean RED.
        sys.path.insert(0, str(CHECK_HANDOFF.parent))
        self.addCleanup(sys.path.remove, str(CHECK_HANDOFF.parent))

    def write_files(self, board, submission, artifact_bytes=ARTIFACT_BYTES):
        board_path = self.dir / "board.json"
        submission_path = self.dir / "submission.json"
        artifact_path = self.dir / "handoff.patch"
        board_path.write_bytes(json.dumps(board).encode())
        submission_path.write_bytes(json.dumps(submission).encode())
        artifact_path.write_bytes(artifact_bytes)
        return board_path, submission_path, artifact_path

    def run_cli(self, *args):
        return subprocess.run(
            [sys.executable, str(CHECK_HANDOFF), *map(str, args)],
            capture_output=True, text=True,
        )


class TestValidHandoff(HandoffTestBase):
    def test_valid_handoff_api_returns_none(self):
        from check_handoff import validate_handoff
        board = make_board()
        submission = make_submission()
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        self.assertIsNone(validate_handoff(board, submission, artifact))

    def test_valid_handoff_cli_exit_zero(self):
        board = make_board()
        submission = make_submission()
        paths = self.write_files(board, submission)
        proc = self.run_cli(*paths)
        self.assertEqual(proc.returncode, 0, msg=proc.stderr)

    def test_valid_open_states(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        for state in ("running", "submitted", "review-needed"):
            with self.subTest(state=state):
                board = make_board(state=state)
                self.assertIsNone(validate_handoff(board, make_submission(), artifact))

    def test_extra_fields_ignored(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        board = make_board(authority="sole lead", base="abc123",
                           extra_top="anything")
        board["tasks"]["P00-handoff"]["profile"] = "muse-free"
        board["tasks"]["OTHER-task"] = {"attempt": "bogus-not-checked"}
        submission = make_submission(note="extra allowed")
        self.assertIsNone(validate_handoff(board, submission, artifact))

    def test_files_unmutated_by_validation(self):
        from check_handoff import validate_handoff
        board = make_board()
        submission = make_submission()
        board_path, sub_path, art_path = self.write_files(board, submission)
        before = [p.read_bytes() for p in (board_path, sub_path, art_path)]
        validate_handoff(json.loads(board_path.read_bytes()),
                         json.loads(sub_path.read_bytes()), art_path)
        proc = self.run_cli(board_path, sub_path, art_path)
        self.assertEqual(proc.returncode, 0, msg=proc.stderr)
        after = [p.read_bytes() for p in (board_path, sub_path, art_path)]
        self.assertEqual(before, after)


class TestStaleAndMismatch(HandoffTestBase):
    def test_stale_attempt_after_replacement_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        before = artifact.read_bytes()
        # Lead replaced attempt 1 with attempt 2; the old submission must fail.
        board = make_board(attempt=2, owner="muse-p00-a2")
        stale = make_submission(attempt=1, owner="muse-p00-a1")
        with self.assertRaises(ValueError):
            validate_handoff(board, stale, artifact)
        # Partial-work artifact bytes and hash are preserved untouched.
        self.assertEqual(artifact.read_bytes(), before)
        self.assertEqual(sha256(artifact.read_bytes()), stale["artifact_sha256"])

    def test_wrong_owner_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), make_submission(owner="someone-else"), artifact)

    def test_wrong_run_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), make_submission(run="other-run"), artifact)

    def test_unknown_task_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), make_submission(task="P99-nope"), artifact)


class TestInactiveAttestation(HandoffTestBase):
    def test_verified_inactive_false_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        with self.assertRaises(ValueError):
            validate_handoff(make_board(),
                             make_submission(verified_inactive=False), artifact)

    def test_verified_inactive_missing_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        submission = make_submission()
        del submission["verified_inactive"]
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), submission, artifact)

    def test_verified_inactive_truthy_non_bool_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        with self.assertRaises(ValueError):
            validate_handoff(make_board(),
                             make_submission(verified_inactive=1), artifact)


class TestClosedStates(HandoffTestBase):
    def test_closed_states_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        for state in ("closed", "blocked", "replaced", "accepted", "revoked"):
            with self.subTest(state=state):
                with self.assertRaises(ValueError):
                    validate_handoff(make_board(state=state),
                                     make_submission(), artifact)


class TestArtifact(HandoffTestBase):
    def test_invalid_artifact_type_rejected_without_opening_descriptors(self):
        from check_handoff import validate_handoff
        for artifact in (None, [], 0):
            with self.subTest(artifact=artifact):
                with patch("builtins.open", side_effect=AssertionError("invalid artifact opened")):
                    with self.assertRaises(ValueError):
                        validate_handoff(make_board(), make_submission(), artifact)

    def test_changed_artifact_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(b"tampered bytes, not the frozen patch")
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), make_submission(), artifact)

    def test_missing_artifact_rejected(self):
        from check_handoff import validate_handoff
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), make_submission(),
                             self.dir / "does-not-exist.patch")

    def test_bad_digest_format_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        for bad in ("", "xyz", sha256(ARTIFACT_BYTES).upper(),
                    sha256(ARTIFACT_BYTES)[:63], "0" * 64 + "extra", 12345, None):
            with self.subTest(digest=repr(bad)):
                submission = make_submission(digest=bad)
                with self.assertRaises(ValueError):
                    validate_handoff(make_board(), submission, artifact)

    def test_artifact_is_directory_rejected(self):
        from check_handoff import validate_handoff
        subdir = self.dir / "adir"
        subdir.mkdir()
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), make_submission(), subdir)


class TestMalformedAndTypes(HandoffTestBase):
    def test_non_dict_top_level_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        for bad in ([], "str", 42, None, True):
            with self.subTest(board=repr(bad)):
                with self.assertRaises(ValueError):
                    validate_handoff(bad, make_submission(), artifact)
            with self.subTest(submission=repr(bad)):
                with self.assertRaises(ValueError):
                    validate_handoff(make_board(), bad, artifact)

    def test_missing_required_fields_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        board = make_board()
        del board["run"]
        with self.assertRaises(ValueError):
            validate_handoff(board, make_submission(), artifact)
        submission = make_submission()
        del submission["evidence"]
        with self.assertRaises(ValueError):
            validate_handoff(make_board(), submission, artifact)

    def test_bool_attempt_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        board = make_board(attempt=True)
        with self.assertRaises(ValueError):
            validate_handoff(board, make_submission(attempt=True), artifact)

    def test_invalid_board_entries_with_valid_submission(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        boards = [make_board(tasks=tasks) for tasks in (None, [], {"P00-handoff": None})]
        for key, value in (("attempt", True), ("attempt", None), ("owner", 42),
                           ("owner", ""), ("state", [])):
            board = make_board()
            board["tasks"]["P00-handoff"][key] = value
            boards.append(board)
        for board in boards:
            with self.subTest(board=board):
                with self.assertRaises(ValueError):
                    validate_handoff(board, make_submission(), artifact)

    def test_bad_types_rejected(self):
        from check_handoff import validate_handoff
        artifact = self.dir / "handoff.patch"
        artifact.write_bytes(ARTIFACT_BYTES)
        cases = [
            (make_board(run=""), make_submission()),
            (make_board(run=7), make_submission()),
            ({"schema_version": "1", "run": "r", "tasks": {}}, make_submission()),
            ({"schema_version": 2, "run": "r", "tasks": {}}, make_submission()),
            (make_board(), make_submission(attempt=0)),
            (make_board(), make_submission(attempt=-1)),
            (make_board(), make_submission(owner="")),
            (make_board(), make_submission(evidence="")),
            (make_board(), make_submission(evidence=123)),
        ]
        for board, submission in cases:
            with self.subTest(board=board, submission=submission):
                with self.assertRaises(ValueError):
                    validate_handoff(board, submission, artifact)


class TestCliErrors(HandoffTestBase):
    def test_cli_rejects_excessive_nesting_without_traceback(self):
        paths = self.write_files(make_board(), make_submission())
        paths[0].write_bytes(b'{"extra":' + b'[' * 3900 + b'0' + b']' * 3900 + b'}')
        self.assertLess(paths[0].stat().st_size, 8 * 1024)
        proc = self.run_cli(*paths)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_cli_rejects_malformed_json_without_traceback(self):
        board_path = self.dir / "board.json"
        sub_path = self.dir / "submission.json"
        art_path = self.dir / "handoff.patch"
        board_path.write_bytes(b"{not valid json!!!")
        sub_path.write_bytes(json.dumps(make_submission()).encode())
        art_path.write_bytes(ARTIFACT_BYTES)
        proc = self.run_cli(board_path, sub_path, art_path)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_cli_rejects_invalid_handoff_without_traceback(self):
        board = make_board(state="blocked")
        submission = make_submission()
        paths = self.write_files(board, submission)
        proc = self.run_cli(*paths)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotEqual(proc.stderr.strip(), "")
        self.assertNotIn("Traceback", proc.stderr)

    def test_cli_rejects_missing_files_without_traceback(self):
        proc = self.run_cli(self.dir / "no-board.json",
                            self.dir / "no-sub.json",
                            self.dir / "no-artifact.patch")
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_cli_rejects_json_array_top_level_without_traceback(self):
        board_path = self.dir / "board.json"
        sub_path = self.dir / "submission.json"
        art_path = self.dir / "handoff.patch"
        board_path.write_bytes(b"[]")
        sub_path.write_bytes(b"[]")
        art_path.write_bytes(ARTIFACT_BYTES)
        proc = self.run_cli(board_path, sub_path, art_path)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)


class TestSizeBounds(HandoffTestBase):
    def test_oversize_board_rejected(self):
        from check_handoff import validate_handoff  # noqa: F401 (import must exist)
        board = make_board(padding="x" * 9000)
        submission = make_submission()
        board_path, sub_path, art_path = self.write_files(board, submission)
        self.assertGreater(board_path.stat().st_size, 8 * 1024)
        proc = self.run_cli(board_path, sub_path, art_path)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)

    def test_oversize_submission_rejected(self):
        board = make_board()
        submission = make_submission(note="y" * 2500)
        board_path, sub_path, art_path = self.write_files(board, submission)
        self.assertGreater(sub_path.stat().st_size, 2 * 1024)
        proc = self.run_cli(board_path, sub_path, art_path)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn("Traceback", proc.stderr)


class TestCancellationFixture(HandoffTestBase):
    def test_replacement_preserves_partial_artifact_and_rejects_stale(self):
        """Cancellation/replacement: partial work bytes survive, attempt 1 is stale.

        This fixture does NOT prove process termination; it only proves the
        validator rejects a stale attempt-1 submission after the lead records
        attempt 2, without touching the preserved partial-work artifact.
        """
        from check_handoff import validate_handoff
        partial = b"partial attempt-1 work: half-written diff\n\x00\xff"
        artifact = self.dir / "partial.patch"
        artifact.write_bytes(partial)
        frozen_hash = sha256(partial)

        replaced_board = make_board(attempt=2, owner="muse-p00-a2", state="running")
        stale_submission = make_submission(attempt=1, owner="muse-p00-a1",
                                           digest=frozen_hash)
        with self.assertRaises(ValueError):
            validate_handoff(replaced_board, stale_submission, artifact)
        self.assertEqual(artifact.read_bytes(), partial)
        self.assertEqual(sha256(artifact.read_bytes()), frozen_hash)

        # The replacement attempt itself validates against the same bytes
        # when the lead freezes them under the new submission.
        fresh = make_submission(attempt=2, owner="muse-p00-a2", digest=frozen_hash)
        self.assertIsNone(validate_handoff(replaced_board, fresh, artifact))


if __name__ == "__main__":
    unittest.main()
