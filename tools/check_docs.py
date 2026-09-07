#!/usr/bin/env python3
"""Check repository-local Markdown links and ADR/requirement consistency.

Uses only the standard library. This intentionally checks the Markdown conventions
used by this repository, not every CommonMark construct or external URL availability.
"""

from __future__ import annotations

from datetime import date
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[1]
ADR_DIR = ROOT / "docs" / "adr"
STATUSES = {"Proposed", "Accepted", "Rejected", "Superseded"}
SECTIONS = {"Context", "Decision", "Alternatives", "Consequences", "Validation", "References"}
SKIP_PARTS = {"vendor", ".git", "build", "out", ".gradle", ".cache", ".venv", "node_modules"}


def without_fences(text: str) -> str:
    return re.sub(r"^```[^\n]*\n.*?^```\s*$", "", text, flags=re.M | re.S)


def heading_ids(text: str) -> set[str]:
    found: set[str] = set()
    counts: dict[str, int] = {}
    for heading in re.findall(r"^#{1,6}\s+(.+?)\s*#*\s*$", without_fences(text), re.M):
        slug = re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")
        count = counts.get(slug, 0)
        counts[slug] = count + 1
        found.add(f"{slug}-{count}" if count else slug)
    return found


def main() -> int:
    errors: list[str] = []
    markdown = sorted(
        path for path in ROOT.rglob("*.md")
        if not any(part in SKIP_PARTS for part in path.relative_to(ROOT).parts)
    )
    contents = {path: path.read_text(encoding="utf-8") for path in markdown}
    link_count = 0
    for path, text in contents.items():
        relative = path.relative_to(ROOT)
        if not text.endswith("\n"):
            errors.append(f"{relative}: missing final newline")
        for target in re.findall(r"!?\[[^\]\n]*\]\(([^)\s]+)\)", without_fences(text)):
            parts = urlsplit(target)
            if parts.scheme or parts.netloc:
                continue
            link_count += 1
            resolved = (path.parent / unquote(parts.path)).resolve() if parts.path else path
            if not resolved.is_relative_to(ROOT):
                errors.append(f"{relative}: link escapes repository: {target}")
            elif not resolved.exists():
                errors.append(f"{relative}: missing link target: {target}")
            elif parts.fragment and resolved.suffix == ".md":
                target_text = contents.get(resolved, resolved.read_text(encoding="utf-8"))
                if unquote(parts.fragment) not in heading_ids(target_text):
                    errors.append(f"{relative}: missing heading: {target}")

    requirements_path = ROOT / "docs" / "REQUIREMENTS.md"
    if requirements_path not in contents:
        errors.append("docs/REQUIREMENTS.md: required file missing")
        requirements: set[str] = set()
    else:
        requirement_rows = re.findall(r"^\| (R\d{2,}) \|", contents[requirements_path], re.M)
        requirements = set(requirement_rows)
        if not requirements or len(requirements) != len(requirement_rows):
            errors.append("docs/REQUIREMENTS.md: missing or duplicate requirement IDs")

    index_path = ADR_DIR / "README.md"
    index_text = contents.get(index_path, "")
    index_rows = re.findall(
        r"^\| \[(\d{4})\]\(([^)]+)\) \| [^|]+ \| (\w+) \|", index_text, re.M
    )
    indexed = {number: (filename, status) for number, filename, status in index_rows}
    if len(indexed) != len(index_rows):
        errors.append("docs/adr/README.md: duplicate ADR index IDs")

    records: dict[str, Path] = {}
    covered: set[str] = set()
    for path in sorted(ADR_DIR.glob("[0-9][0-9][0-9][0-9]-*.md")):
        text = contents[path]
        relative = path.relative_to(ROOT)
        number = path.name[:4]
        if number in records:
            errors.append(f"{relative}: duplicate ADR ID {number}")
        records[number] = path
        if not text.startswith(f"# ADR-{number}:"):
            errors.append(f"{relative}: heading does not match filename ID")
        status_match = re.search(r"^- Status: (\w+)\s*$", text, re.M)
        status = status_match.group(1) if status_match else ""
        if status not in STATUSES:
            errors.append(f"{relative}: invalid/missing Status")
        if indexed.get(number) != (path.name, status):
            errors.append(f"{relative}: missing/mismatched index entry")
        date_match = re.search(r"^- Date: (\S+)\s*$", text, re.M)
        try:
            date.fromisoformat(date_match.group(1) if date_match else "")
        except ValueError:
            errors.append(f"{relative}: invalid/missing Date")
        if not re.search(r"^- Basis: \S", text, re.M):
            errors.append(f"{relative}: missing decision basis")
        sections = set(re.findall(r"^## (.+)$", text, re.M))
        if missing := SECTIONS - sections:
            errors.append(f"{relative}: missing sections {sorted(missing)}")
        requirement_match = re.search(r"^- Requirements: (.+)$", text, re.M)
        refs = set(re.findall(r"\bR\d{2,}\b", requirement_match.group(1) if requirement_match else ""))
        if not refs or refs - requirements:
            errors.append(f"{relative}: missing/unknown requirement references {sorted(refs - requirements)}")
        covered.update(refs)

    if set(indexed) != set(records):
        errors.append("docs/adr/README.md: index and ADR files differ")
    if not records:
        errors.append("No ADR records found")
    if requirements - covered:
        errors.append(f"Requirements without ADR coverage: {sorted(requirements - covered)}")
    for row in contents.get(requirements_path, "").splitlines():
        if re.match(r"^\| R\d", row):
            cells = row.split("|")
            referenced = set(re.findall(r"\b\d{4}\b", cells[-2]))
            if not referenced or referenced - set(records):
                errors.append(f"Requirement row has missing/unknown ADR reference: {cells[1].strip()}")

    if errors:
        print("Documentation validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"PASS: {len(markdown)} Markdown files, {link_count} local links, "
          f"{len(records)} ADRs and {len(requirements)} requirements.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
