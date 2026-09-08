#!/usr/bin/env python3
"""Read-only handoff validator for the sole lead-written compact campaign board.

The lead freezes a worker's patch file (binary git diff plus separately
captured untracked files as needed) as an opaque artifact, records its SHA256
in a submission JSON, verifies runner/children are stopped, then runs this
validator before manual review. The validator checks the submission against
the current board entry and the artifact bytes only.

Trusted local guard, not authentication or a sandbox. Performs no Git
operations, kills no processes, and never writes to the checked files.

Frozen interface v1:
    validate_handoff(board: dict, submission: dict, artifact: pathlib.Path)
        -> None; raises ValueError on invalid input.

CLI: check_handoff.py BOARD_JSON SUBMISSION_JSON ARTIFACT
    exit 0 when valid, nonzero with a concise diagnostic when invalid.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

BOARD_LIMIT = 8 * 1024
SUBMISSION_LIMIT = 2 * 1024
OPEN_STATES = frozenset({"running", "submitted", "review-needed"})
_DIGEST_RE = re.compile(r"[0-9a-f]{64}")


def _nonempty_str(value, name):
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be a nonempty string")
    return value


def _positive_int(value, name):
    # type(x) is int: bool is a subclass of int and must not pass.
    if type(value) is not int or value <= 0:
        raise ValueError(f"{name} must be a positive integer")
    return value


def _check_digest(value):
    if not isinstance(value, str) or not _DIGEST_RE.fullmatch(value):
        raise ValueError("submission artifact_sha256 must be 64 lowercase hex chars")


def validate_handoff(board: dict, submission: dict, artifact: Path) -> None:
    """Validate a handoff submission against the board and artifact bytes.

    Raises ValueError with a concise reason on any invalid input. Never
    writes to any file. Extra fields in board/submission are ignored so
    existing board metadata keeps validating.
    """
    if not isinstance(board, dict):
        raise ValueError("board must be a JSON object")
    if not isinstance(submission, dict):
        raise ValueError("submission must be a JSON object")
    if board.get("schema_version") != 1 or type(board.get("schema_version")) is not int:
        raise ValueError("board schema_version must be 1")
    if submission.get("schema_version") != 1 or type(submission.get("schema_version")) is not int:
        raise ValueError("submission schema_version must be 1")

    board_run = _nonempty_str(board.get("run"), "board run")
    sub_run = _nonempty_str(submission.get("run"), "submission run")
    task = _nonempty_str(submission.get("task"), "submission task")
    attempt = _positive_int(submission.get("attempt"), "submission attempt")
    owner = _nonempty_str(submission.get("owner"), "submission owner")
    _check_digest(submission.get("artifact_sha256"))
    if submission.get("verified_inactive") is not True:
        raise ValueError(
            "submission verified_inactive must be true "
            "(manual lead attestation, never a claim of OS proof)")
    _nonempty_str(submission.get("evidence"),
                  "submission evidence (lead supervision evidence)")

    if sub_run != board_run:
        raise ValueError("submission run does not match board run")

    tasks = board.get("tasks")
    if not isinstance(tasks, dict):
        raise ValueError("board tasks must be an object")
    entry = tasks.get(task)
    if not isinstance(entry, dict):
        raise ValueError("submission task is not the current board task")
    if _positive_int(entry.get("attempt"), "board attempt") != attempt:
        raise ValueError("stale or unknown attempt: board attempt does not match")
    if _nonempty_str(entry.get("owner"), "board owner") != owner:
        raise ValueError("submission owner does not match board owner")
    state = entry.get("state")
    if not isinstance(state, str) or state not in OPEN_STATES:
        raise ValueError(
            f"task state {state!r} is not open for validation "
            "(want running, submitted, or review-needed)")

    if not isinstance(artifact, Path):
        raise ValueError("artifact must be a pathlib.Path, not a file descriptor")
    try:
        with open(artifact, "rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
    except OSError as exc:
        raise ValueError(f"cannot read artifact: {exc.strerror or exc}") from None
    if actual != submission["artifact_sha256"]:
        raise ValueError("artifact sha256 mismatch: artifact changed since freeze")


def _load_json(path: Path, limit: int, name: str) -> dict:
    try:
        with open(path, "rb") as stream:
            raw = stream.read(limit + 1)
    except OSError as exc:
        raise ValueError(f"cannot read {name}: {exc.strerror or exc}") from None
    if len(raw) > limit:
        raise ValueError(f"{name} exceeds {limit} bytes")
    try:
        value = json.loads(raw)
    except (ValueError, UnicodeDecodeError) as exc:
        raise ValueError(f"{name} is not valid JSON: {exc}") from None
    if not isinstance(value, dict):
        raise ValueError(f"{name} top level must be a JSON object")
    return value


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate a frozen handoff (board + submission + artifact).")
    parser.add_argument("board_json")
    parser.add_argument("submission_json")
    parser.add_argument("artifact")
    args = parser.parse_args(argv)
    try:
        board = _load_json(Path(args.board_json), BOARD_LIMIT, "board JSON")
        submission = _load_json(Path(args.submission_json), SUBMISSION_LIMIT,
                                "submission JSON")
        validate_handoff(board, submission, Path(args.artifact))
    except ValueError as exc:
        print(f"error: invalid handoff: {exc}", file=sys.stderr)
        return 1
    print("handoff valid")
    return 0


if __name__ == "__main__":
    sys.exit(main())
