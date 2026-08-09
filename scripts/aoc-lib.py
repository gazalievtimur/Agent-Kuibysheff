#!/usr/bin/env python3
"""Helpers for Linux AoC eval / regression scripts."""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from typing import Any


def yaml_scalar(text: str, key: str, default: str = "") -> str:
    patterns = [
        rf'(?m)^\s*{re.escape(key)}:\s*"([^"]*)"',
        rf"(?m)^\s*{re.escape(key)}:\s*'([^']*)'",
        rf"(?m)^\s*{re.escape(key)}:\s*([^#\r\n]+)",
    ]
    for pattern in patterns:
        match = re.search(pattern, text)
        if match:
            return match.group(1).strip()
    return default


def yaml_provider_api_key(text: str) -> str:
    patterns = [
        r'(?m)^\s*api_key:\s*"([^"]*)"',
        r"(?m)^\s*api_key:\s*'([^']*)'",
        r"(?m)^\s*api_key:\s*([^#\r\n]+)",
    ]
    for pattern in patterns:
        match = re.search(pattern, text)
        if match:
            value = match.group(1).strip()
            if value:
                return value
    return ""


def provider_api_key_available(text: str) -> bool:
    """True when the configured api_key_env is present in the environment."""
    api_key_env = yaml_scalar(text, "api_key_env", "OPENAI_API_KEY")
    return bool(os.environ.get(api_key_env, "").strip())


def extract_json_object(raw: str) -> dict[str, Any]:
    text = raw.strip()
    start = text.find("{")
    if start < 0:
        raise ValueError("no JSON object in agent stdout")
    depth = 0
    end = -1
    for i, ch in enumerate(text[start:], start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                end = i
                break
    if end < start:
        raise ValueError("unbalanced JSON in agent stdout")
    return json.loads(text[start : end + 1])


def cmd_yaml_scalar() -> None:
    key = sys.argv[2]
    default = sys.argv[3] if len(sys.argv) > 3 else ""
    text = sys.stdin.read()
    sys.stdout.write(yaml_scalar(text, key, default))


def cmd_yaml_api_key() -> None:
    sys.stdout.write(yaml_provider_api_key(sys.stdin.read()))


def cmd_api_key_available() -> None:
    raise SystemExit(0 if provider_api_key_available(sys.stdin.read()) else 1)


def cmd_task_meta() -> None:
    path = Path(sys.argv[2])
    obj = json.loads(path.read_text(encoding="utf-8"))
    tid = obj.get("id")
    if not tid:
        raise SystemExit(f"Task file missing id: {path}")
    if "expected" not in obj or obj["expected"] is None:
        raise SystemExit(f"Task {tid} missing expected")
    print(f"{tid}\t{str(obj['expected']).strip()}\t{path}")


def cmd_seed_input() -> None:
    src = Path(sys.argv[2])
    dest = Path(sys.argv[3])
    obj = json.loads(src.read_text(encoding="utf-8"))
    text = str(obj["input"])
    if not text.endswith("\n"):
        text += "\n"
    dest.write_text(text, encoding="utf-8")


def cmd_parse_stdout() -> None:
    raw = Path(sys.argv[2]).read_text(encoding="utf-8")
    obj = extract_json_object(raw)
    actual = "" if obj.get("result") is None else str(obj["result"]).strip()
    stop = str(obj.get("stop_reason") or "")
    print(
        json.dumps(
            {
                "result": actual,
                "stop_reason": stop,
                "usage": obj.get("usage"),
                "logs": obj.get("logs"),
            },
            ensure_ascii=False,
        )
    )


def cmd_print_logs() -> None:
    logs = json.loads(sys.argv[2])
    if not isinstance(logs, dict):
        return
    for key, label in (
        ("system_log", "system"),
        ("ai_log", "ai"),
        ("mcp_log", "mcp"),
        ("chat_log", "chat"),
    ):
        if logs.get(key):
            print(f"  {label}: {logs[key]}")


def cmd_append_task() -> None:
    tasks = json.loads(sys.argv[2])
    entry = {
        "id": sys.argv[3],
        "expected": sys.argv[4],
        "pass": sys.argv[5] == "true",
        "stop_reason": sys.argv[6] or None,
        "result": sys.argv[7] or None,
        "usage": json.loads(sys.argv[8]),
        "error": sys.argv[9] or None,
        "home": sys.argv[10],
        "log_dir": sys.argv[11],
        "logs": json.loads(sys.argv[12]),
        "elapsed_ms": int(sys.argv[13]),
    }
    tasks.append(entry)
    print(json.dumps(tasks, ensure_ascii=False))


def cmd_write_report() -> None:
    report = {
        "run_id": sys.argv[2],
        "bank_dir": sys.argv[3],
        "config": sys.argv[4],
        "settings": sys.argv[5],
        "passed": int(sys.argv[6]),
        "failed": int(sys.argv[7]),
        "total": int(sys.argv[8]),
        "tasks": json.loads(sys.argv[9]),
    }
    path = Path(sys.argv[10])
    path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


COMMANDS = {
    "yaml-scalar": cmd_yaml_scalar,
    "yaml-api-key": cmd_yaml_api_key,
    "api-key-available": cmd_api_key_available,
    "task-meta": cmd_task_meta,
    "seed-input": cmd_seed_input,
    "parse-stdout": cmd_parse_stdout,
    "print-logs": cmd_print_logs,
    "append-task": cmd_append_task,
    "write-report": cmd_write_report,
}


def main() -> None:
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        names = ", ".join(sorted(COMMANDS))
        raise SystemExit(f"usage: aoc-lib.py <{'|'.join(sorted(COMMANDS))}> ...\nknown: {names}")
    COMMANDS[sys.argv[1]]()


if __name__ == "__main__":
    main()
