#!/usr/bin/env python3
"""Assert one SWE-bench regression instance is harness-resolved.

Reads report.json, prints a short UX summary (status, stop_reason, resolved,
elapsed, usage/cost), and exits 0 only when the target instance has
harness_resolved == true.
"""

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
    for key in ("input_tokens", "output_tokens", "total_tokens", "iterations"):
        if key in usage and usage[key] is not None:
            parts.append(f"{key}={usage[key]}")
    cost = usage.get("cost")
    if isinstance(cost, dict):
        status = cost.get("status")
        amount = cost.get("amount")
        currency = cost.get("currency") or ""
        if status:
            parts.append(f"cost.status={status}")
        if amount is not None:
            parts.append(f"cost={amount}{currency}")
    elif cost is not None:
        parts.append(f"cost={cost}")
    return ", ".join(parts) if parts else "n/a"


def _find_instance(report: dict[str, Any], instance_id: str) -> Optional[dict[str, Any]]:
    per = report.get("per_instance")
    if not isinstance(per, list):
        return None
    for row in per:
        if isinstance(row, dict) and str(row.get("instance_id") or "") == instance_id:
            return row
    return None


def print_summary(
    *,
    report_path: Path,
    instance_id: str,
    report: dict[str, Any],
    row: Optional[dict[str, Any]],
) -> None:
    print(f"report: {report_path}")
    print(f"run_id: {report.get('run_id')}")
    print(f"instance_id: {instance_id}")
    if row is None:
        print("status: missing from report")
        print("harness_resolved: <missing>")
        return

    resolved = row.get("harness_resolved")
    print(f"status: {row.get('status')}")
    print(f"stop_reason: {row.get('stop_reason')}")
    print(f"harness_resolved: {resolved}")
    print(f"elapsed_sec: {row.get('elapsed_sec')}")
    print(f"usage: {_fmt_usage(row.get('usage'))}")
    aggregates = report.get("usage_aggregates")
    if aggregates is not None:
        print(f"usage_aggregates: {aggregates}")
    if row.get("error"):
        print(f"error: {row.get('error')}")
    print(
        "totals: "
        f"resolved={report.get('resolved')} "
        f"graded={report.get('graded')} "
        f"generated_patches={report.get('generated_patches')} "
        f"agent_errors={report.get('agent_errors')} "
        f"infrastructure_errors={report.get('infrastructure_errors')}"
    )


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="Path to report.json")
    parser.add_argument(
        "--instance-id",
        required=True,
        help="Expected instance id (must be harness_resolved)",
    )
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

    instance_id = args.instance_id.strip()
    row = _find_instance(report, instance_id)
    print_summary(
        report_path=report_path,
        instance_id=instance_id,
        report=report,
        row=row,
    )

    if row is None:
        print("SWE-bench regression FAILED: instance missing from report", file=sys.stderr)
        return 1

    if row.get("harness_resolved") is True:
        print("SWE-bench regression PASSED (harness_resolved=true)")
        return 0

    print(
        "SWE-bench regression FAILED: expected harness_resolved=true, "
        f"got {row.get('harness_resolved')!r}",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
