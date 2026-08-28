#!/usr/bin/env python3
"""Coverage ratchet + changed-lines gate for cargo-llvm-cov LCOV reports.

Usage:
  # Absolute floor from committed baseline JSON:
  python scripts/coverage_ratchet.py --lcov lcov.info --baseline .github/coverage-baseline.json --os linux

  # Compare current LCOV totals against a previous (main) LCOV artifact:
  python scripts/coverage_ratchet.py --lcov lcov.info --vs-lcov baseline/lcov.info --tolerance 0.5

  # Changed-lines gate vs a git merge-base:
  python scripts/coverage_ratchet.py --lcov lcov.info --changed-from origin/main --changed-min 70
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from typing import DefaultDict, Dict, Iterable, Optional, Set, Tuple


def parse_lcov(path: Path) -> Tuple[Dict[str, Dict[int, int]], float, int, int]:
    """Return (file -> line -> hits), line_percent, hit_lines, found_lines."""
    files: Dict[str, Dict[int, int]] = {}
    current: Optional[str] = None
    hits = 0
    found = 0
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if raw.startswith("SF:"):
            current = raw[3:].replace("\\", "/")
            files.setdefault(current, {})
        elif raw.startswith("DA:") and current is not None:
            # DA:<line>,<hits>
            parts = raw[3:].split(",")
            if len(parts) < 2:
                continue
            line = int(parts[0])
            count = int(float(parts[1]))
            files[current][line] = count
            found += 1
            if count > 0:
                hits += 1
        elif raw == "end_of_record":
            current = None
    percent = (100.0 * hits / found) if found else 0.0
    return files, percent, hits, found


def load_baseline(path: Path, os_key: str) -> float:
    data = json.loads(path.read_text(encoding="utf-8"))
    if os_key not in data:
        raise SystemExit(f"baseline missing key `{os_key}` in {path}")
    return float(data[os_key]["min_lines_percent"])


def git_changed_lines(merge_base_ref: str) -> DefaultDict[str, Set[int]]:
    """Map repo-relative path -> set of added/changed line numbers in the diff."""
    result = subprocess.run(
        ["git", "diff", "--unified=0", f"{merge_base_ref}...HEAD"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise SystemExit(
            f"error: git diff {merge_base_ref}...HEAD failed "
            f"(exit {result.returncode}): {detail}"
        )
    changed: DefaultDict[str, Set[int]] = defaultdict(set)
    path: Optional[str] = None
    hunk = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
    for line in result.stdout.splitlines():
        if line.startswith("+++ b/"):
            path = line[6:].replace("\\", "/")
            continue
        if path is None or not path.endswith(".rs"):
            continue
        m = hunk.match(line)
        if m:
            start = int(m.group(1))
            count = int(m.group(2) or "1")
            if count == 0:
                continue
            for n in range(start, start + count):
                changed[path].add(n)
    return changed


def resolve_lcov_file(
    lcov_files: Dict[str, Dict[int, int]], rel_path: str
) -> Optional[Dict[int, int]]:
    """Match git-relative paths to SF: entries (absolute or relative)."""
    if rel_path in lcov_files:
        return lcov_files[rel_path]
    suffix = "/" + rel_path
    for key, lines in lcov_files.items():
        if key.endswith(suffix) or key.endswith(rel_path):
            return lines
    return None


def changed_line_coverage(
    lcov_files: Dict[str, Dict[int, int]], changed: DefaultDict[str, Set[int]]
) -> Tuple[float, int, int, list[str]]:
    covered = 0
    total = 0
    details: list[str] = []
    for rel, lines in sorted(changed.items()):
        mapped = resolve_lcov_file(lcov_files, rel)
        if mapped is None:
            # File not in LCOV (e.g. test-only cfg); skip rather than fail closed.
            details.append(f"skip {rel}: not in LCOV")
            continue
        for line in sorted(lines):
            if line not in mapped:
                # Non-executable / comment / blank in the diff hunk.
                continue
            total += 1
            if mapped[line] > 0:
                covered += 1
            else:
                details.append(f"uncovered {rel}:{line}")
    percent = (100.0 * covered / total) if total else 100.0
    return percent, covered, total, details


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lcov", type=Path, required=True, help="Current LCOV report")
    parser.add_argument(
        "--baseline",
        type=Path,
        help="JSON baseline with per-OS min_lines_percent",
    )
    parser.add_argument(
        "--os",
        choices=("linux", "windows"),
        help="OS key inside baseline JSON",
    )
    parser.add_argument(
        "--vs-lcov",
        type=Path,
        help="Previous LCOV (e.g. main artifact) for total-coverage ratchet",
    )
    parser.add_argument(
        "--tolerance",
        type=float,
        default=0.5,
        help="Allowed drop vs --vs-lcov in percentage points (default 0.5)",
    )
    parser.add_argument(
        "--changed-from",
        help="Git ref to diff against for changed-lines gate (e.g. origin/main)",
    )
    parser.add_argument(
        "--changed-min",
        type=float,
        default=70.0,
        help="Minimum percent of changed executable lines that must be covered",
    )
    parser.add_argument(
        "--max-uncovered-details",
        type=int,
        default=40,
        help="Max uncovered changed-line details to print",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    if not args.lcov.is_file():
        print(f"error: LCOV not found: {args.lcov}", file=sys.stderr)
        return 2

    files, percent, hits, found = parse_lcov(args.lcov)
    print(f"current line coverage: {percent:.2f}% ({hits}/{found})")

    failed = False

    if args.baseline:
        if not args.os:
            print("error: --os required with --baseline", file=sys.stderr)
            return 2
        floor = load_baseline(args.baseline, args.os)
        print(f"baseline floor ({args.os}): {floor:.2f}%")
        if percent + 1e-9 < floor:
            print(
                f"FAIL: coverage {percent:.2f}% is below committed floor {floor:.2f}%",
                file=sys.stderr,
            )
            failed = True
        else:
            print(f"ok: above committed floor ({percent:.2f}% >= {floor:.2f}%)")

    if args.vs_lcov:
        if not args.vs_lcov.is_file():
            print(
                f"warn: no main LCOV at {args.vs_lcov}; skipping vs-main ratchet",
                file=sys.stderr,
            )
        else:
            _, base_pct, base_hits, base_found = parse_lcov(args.vs_lcov)
            print(
                f"main line coverage: {base_pct:.2f}% ({base_hits}/{base_found}); "
                f"tolerance={args.tolerance:.2f}pp"
            )
            if percent + args.tolerance + 1e-9 < base_pct:
                print(
                    f"FAIL: coverage dropped vs main "
                    f"({percent:.2f}% < {base_pct:.2f}% - {args.tolerance:.2f})",
                    file=sys.stderr,
                )
                failed = True
            else:
                print(
                    f"ok: within tolerance of main "
                    f"({percent:.2f}% vs {base_pct:.2f}%)"
                )

    if args.changed_from:
        changed = git_changed_lines(args.changed_from)
        ch_pct, ch_hit, ch_total, details = changed_line_coverage(files, changed)
        print(
            f"changed-line coverage vs {args.changed_from}: "
            f"{ch_pct:.2f}% ({ch_hit}/{ch_total})"
        )
        if ch_total == 0:
            print("ok: no executable changed Rust lines to gate")
        elif ch_pct + 1e-9 < args.changed_min:
            print(
                f"FAIL: changed-line coverage {ch_pct:.2f}% "
                f"< {args.changed_min:.2f}%",
                file=sys.stderr,
            )
            for row in details[: args.max_uncovered_details]:
                print(f"  {row}", file=sys.stderr)
            if len(details) > args.max_uncovered_details:
                print(
                    f"  ... and {len(details) - args.max_uncovered_details} more",
                    file=sys.stderr,
                )
            failed = True
        else:
            print(
                f"ok: changed-line coverage {ch_pct:.2f}% >= {args.changed_min:.2f}%"
            )

    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
