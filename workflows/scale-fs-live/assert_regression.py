#!/usr/bin/env python3
"""Assert Scale-FS live regression report: all tasks pass needle checks."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional


def _fmt_usage(usage: Any) -> str:
    if not isinstance(usage, dict):
        return "n/a"
    parts: list[str] = []
    for key in ("iterations", "prompt_tokens", "completion_tokens", "total_tokens"):
        if key in usage and usage[key] is not None:
            parts.append(f"{key}={usage[key]}")
    return ", ".join(parts) if parts else "n/a"


def print_summary(report_path: Path, report: dict[str, Any]) -> None:
    print(f"report: {report_path}")
    print(f"run_id: {report.get('run_id')}")
    print(
        f"totals: passed={report.get('passed')} failed={report.get('failed')} "
        f"total={report.get('total')}"
    )
    tasks = report.get("tasks")
    if not isinstance(tasks, list):
        print("tasks: <missing>")
        return
    for row in tasks:
        if not isinstance(row, dict):
            continue
        print(
            f"  - {row.get('id')}: pass={row.get('pass')} kind={row.get('corpus_kind')!r} "
            f"stop={row.get('stop_reason')!r} result={row.get('result')!r} "
            f"needle={row.get('needle')!r} usage={_fmt_usage(row.get('usage'))}"
        )
        if row.get("failures"):
            print(f"      failures: {row.get('failures')}")
        if row.get("error"):
            print(f"      error: {row.get('error')}")


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="Path to report.json")
    args = parser.parse_args(argv)

    report_path = args.report.resolve()
    if not report_path.is_file():
        print(f"missing report.json: {report_path}", file=sys.stderr)
        return 1

    try:
        report = json.loads(report_path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"failed to read report.json: {exc}", file=sys.stderr)
        return 1

    if not isinstance(report, dict):
        print("report.json root must be an object", file=sys.stderr)
        return 1

    print_summary(report_path, report)

    tasks = report.get("tasks")
    if not isinstance(tasks, list) or not tasks:
        print("Scale-FS regression FAILED: no tasks in report", file=sys.stderr)
        return 1

    failed_rows = [
        row for row in tasks if isinstance(row, dict) and row.get("pass") is not True
    ]
    if failed_rows:
        ids = [str(r.get("id")) for r in failed_rows]
        print(
            f"Scale-FS regression FAILED: {len(failed_rows)} task(s): {', '.join(ids)}",
            file=sys.stderr,
        )
        return 1

    if int(report.get("failed") or 0) != 0:
        print(
            f"Scale-FS regression FAILED: report.failed={report.get('failed')}",
            file=sys.stderr,
        )
        return 1

    print("Scale-FS regression PASSED.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
