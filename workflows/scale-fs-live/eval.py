#!/usr/bin/env python3
"""Live LLM Scale-FS eval harness (many-files search + large/oversize reads)."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from corpus import plant_from_task, verify_planted, write_needle_sidecar  # noqa: E402


def _repo_root_default() -> Path:
    return Path(__file__).resolve().parents[2]


def _escape_yaml_dq(text: str) -> str:
    return text.replace("\\", "\\\\").replace('"', '\\"')


def _yaml_scalar(text: str, key: str, default: str = "") -> str:
    for pattern in (
        rf'(?m)^\s*{re.escape(key)}:\s*"([^"]*)"',
        rf"(?m)^\s*{re.escape(key)}:\s*'([^']*)'",
        rf"(?m)^\s*{re.escape(key)}:\s*([^#\r\n]+)",
    ):
        match = re.search(pattern, text)
        if match:
            return match.group(1).strip()
    return default


def _load_dotenv(path: Path) -> None:
    if not path.is_file():
        return
    for raw in path.read_text(encoding="utf-8-sig").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        name, value = line.split("=", 1)
        name = name.strip()
        value = value.strip()
        if (value.startswith('"') and value.endswith('"')) or (
            value.startswith("'") and value.endswith("'")
        ):
            value = value[1:-1]
        if name and not os.environ.get(name):
            os.environ[name] = value


def _extract_json_object(text: str) -> dict[str, Any]:
    start = text.find("{")
    if start < 0:
        raise ValueError("no JSON object in agent stdout")
    depth = 0
    end = -1
    for i, ch in enumerate(text[start:], start=start):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                end = i
                break
    if end < start:
        raise ValueError("unterminated JSON object in agent stdout")
    return json.loads(text[start : end + 1])


def _which(name: str) -> Optional[str]:
    return shutil.which(name)


def _is_windows_apps_stub(path: str) -> bool:
    normalized = path.replace("/", "\\").lower()
    return "\\windowsapps\\" in normalized


def _py_launcher_executable() -> Optional[str]:
    py = _which("py")
    if not py:
        return None
    try:
        proc = subprocess.run(
            [py, "-3", "-c", "import sys; print(sys.executable)"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
    except OSError:
        return None
    if proc.returncode != 0:
        return None
    text = (proc.stdout or "").strip()
    return text or None


def _resolve_python(repo_root: Path) -> tuple[str, str]:
    """Pick a real interpreter; on Windows prefer AppContainer-friendly staged runtime."""
    # Prefer already-staged AoC/scale-fs runtimes (AppContainer can ACL-grant these).
    for rel in (
        Path("local") / "scale-fs-sandbox-runtime" / "python" / "python.exe",
        Path("local") / "aoc-sandbox-runtime" / "python" / "python.exe",
    ):
        staged = (repo_root / rel).resolve()
        if staged.is_file() and not _is_windows_apps_stub(str(staged)):
            return str(staged), str(staged.parent)

    candidates: list[str] = []
    for name in ("python3", "python"):
        path = _which(name)
        if path:
            candidates.append(path)
    launcher = _py_launcher_executable()
    if launcher:
        candidates.append(launcher)

    host_exe: Optional[Path] = None
    seen: set[str] = set()
    for candidate in candidates:
        if not candidate or candidate in seen:
            continue
        seen.add(candidate)
        if _is_windows_apps_stub(candidate):
            continue
        path = Path(candidate)
        if not path.is_file():
            continue
        try:
            resolved = Path(candidate).resolve()
        except OSError:
            continue
        if _is_windows_apps_stub(str(resolved)):
            continue
        host_exe = resolved
        break

    if host_exe is None:
        raise SystemExit(
            "Could not resolve a real python/python3 for sandboxed home.run "
            "(avoid WindowsApps stub; try the py launcher or a full install)."
        )

    if os.name == "nt":
        return _stage_windows_python(repo_root, host_exe)
    return str(host_exe), str(host_exe.parent)


def _stage_windows_python(repo_root: Path, source_exe: Path) -> tuple[str, str]:
    """Copy host Python under local/ so AppContainer can grant runtime_read_roots."""
    dest_root = (repo_root / "local" / "scale-fs-sandbox-runtime" / "python").resolve()
    marker = dest_root / ".ak-source"
    src_root = source_exe.parent.resolve()
    dest_exe = dest_root / "python.exe"
    need_sync = True
    if dest_exe.is_file() and marker.is_file():
        prev = marker.read_text(encoding="utf-8").strip()
        if prev == str(src_root):
            need_sync = False
    if need_sync:
        dest_root.mkdir(parents=True, exist_ok=True)
        print(f"Staging sandboxed Python runtime from {src_root} -> {dest_root}")
        # Mirror aoc-eval.ps1 robocopy filters.
        cmd = [
            "robocopy",
            str(src_root),
            str(dest_root),
            "/E",
            "/XD",
            "Doc",
            "Docs",
            "tcl",
            "tk",
            "include",
            "Test",
            "tests",
            "Tools",
            "/XF",
            "*.pdb",
            "*.htm",
            "*.chm",
            "/NFL",
            "/NDL",
            "/NJH",
            "/NJS",
            "/nc",
            "/ns",
            "/np",
        ]
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        # robocopy: exit codes < 8 are success
        if proc.returncode >= 8:
            raise SystemExit(
                f"robocopy python runtime failed with code {proc.returncode}: "
                f"{(proc.stderr or proc.stdout or '')[:400]}"
            )
        marker.write_text(str(src_root), encoding="utf-8")
    if not dest_exe.is_file():
        raise SystemExit(f"Staged python runtime missing executable: {dest_exe}")
    return str(dest_exe.resolve()), str(dest_root)


def _agent_bin(repo_root: Path, override: Optional[str]) -> Path:
    if override:
        path = Path(override).resolve()
    else:
        name = "agent_Kuibysheff.exe" if os.name == "nt" else "agent_Kuibysheff"
        path = (repo_root / "target" / "release" / name).resolve()
    if not path.is_file():
        raise SystemExit(f"missing agent binary: {path}")
    return path


def _ensure_profile(
    agent_bin: Path,
    project_root: Path,
    agent_id: str,
    settings_dir: Path,
) -> None:
    project_root.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [str(agent_bin), "init", agent_id, "--project-root", str(project_root), "--force"],
        check=True,
        capture_output=True,
        text=True,
    )
    subprocess.run(
        [
            str(agent_bin),
            "config",
            "--project-root",
            str(project_root),
            "--agent",
            agent_id,
            "import",
            "--from",
            str(settings_dir),
            "--force",
        ],
        check=True,
        capture_output=True,
        text=True,
    )


def _inherit_env_yaml() -> str:
    if os.name == "nt":
        return '["SYSTEMROOT", "SystemRoot"]'
    return "[]"


def _write_run_config(
    path: Path,
    *,
    base_text: str,
    log_dir: Path,
    project_root: Path,
    python_exe: str,
    python_root: str,
) -> None:
    provider_base_url = _yaml_scalar(base_text, "base_url", "https://api.openai.com/v1")
    provider_model = _yaml_scalar(base_text, "model", "gpt-4o-mini")
    provider_api_key_env = _yaml_scalar(base_text, "api_key_env", "OPENAI_API_KEY")
    provider_timeout_ms = _yaml_scalar(base_text, "timeout_ms", "120000")
    max_iterations = _yaml_scalar(base_text, "max_iterations", "24")
    max_tokens = _yaml_scalar(base_text, "max_tokens", "120000")
    max_duration_sec = _yaml_scalar(base_text, "max_duration_sec", "600")
    history_tail = _yaml_scalar(base_text, "max_tail_messages", "40")
    history_chars = _yaml_scalar(base_text, "max_chars", "200000")

    project_root_s = _escape_yaml_dq(str(project_root.resolve()))
    log_dir_s = _escape_yaml_dq(str(log_dir))
    python_exe_s = _escape_yaml_dq(python_exe.replace("\\", "/"))
    python_root_s = _escape_yaml_dq(python_root.replace("\\", "/"))

    content = f"""provider:
  base_url: "{_escape_yaml_dq(provider_base_url)}"
  model: "{_escape_yaml_dq(provider_model)}"
  api_key_env: "{_escape_yaml_dq(provider_api_key_env)}"
  timeout_ms: {provider_timeout_ms}
  max_retries: 3
  retry_base_delay_ms: 500
  history:
    max_tail_messages: {history_tail}
    max_chars: {history_chars}

mcp: []

limits:
  max_iterations: {max_iterations}
  max_tokens: {max_tokens}
  max_duration_sec: {max_duration_sec}

logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: true
  output_dir: "{log_dir_s}"

access:
  mode: strict
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
      - home.run
      - local_tools.search_docs
      - local_tools.read_file
  filesystem:
    home:
      read: ["in", "out"]
      write: ["out"]
    workspace:
      root: "{project_root_s}"
      read: ["corpus"]
  run:
    programs:
      - name: python
        executable: "{python_exe_s}"
        runtime_read_roots: ["{python_root_s}"]
        inherit_env: {_inherit_env_yaml()}
        allow_children: false
    max_args: 32
    max_arg_chars: 4096
    max_output_chars: 200000
    max_timeout_ms: 120000
"""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _score_task(
    *,
    output: dict[str, Any],
    expect: dict[str, Any],
    needle: str,
) -> tuple[bool, list[str]]:
    failures: list[str] = []
    stop = str(output.get("stop_reason") or "")
    want_stop = expect.get("stop_reason")
    if isinstance(want_stop, str) and want_stop:
        if stop != want_stop:
            failures.append(f"stop_reason={stop!r} expected {want_stop!r}")

    result = str(output.get("result") or "")
    if expect.get("result_contains_needle") is True:
        if needle not in result:
            failures.append(f"result missing needle {needle!r}: {result!r}")
    return not failures, failures


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-root", type=Path, default=None)
    parser.add_argument("--bank-dir", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--settings-dir", type=Path, required=True)
    parser.add_argument("--runs-root", type=Path, required=True)
    parser.add_argument("--agent", default="scale-fs-probe")
    parser.add_argument("--agent-bin", default="")
    parser.add_argument("--task-id", action="append", default=[])
    args = parser.parse_args(argv)

    repo_root = (args.repo_root or _repo_root_default()).resolve()
    _load_dotenv(repo_root / ".env")

    bank_dir = args.bank_dir.resolve()
    config_path = args.config.resolve()
    settings_dir = args.settings_dir.resolve()
    runs_root = args.runs_root.resolve()
    agent_id = args.agent

    if not bank_dir.is_dir():
        print(f"bank not found: {bank_dir}", file=sys.stderr)
        return 1
    if not config_path.is_file():
        print(f"config not found: {config_path}", file=sys.stderr)
        return 1
    if not settings_dir.is_dir():
        print(f"settings dir not found: {settings_dir}", file=sys.stderr)
        return 1

    base_text = config_path.read_text(encoding="utf-8-sig")
    api_key_env = _yaml_scalar(base_text, "api_key_env", "OPENAI_API_KEY")
    if not os.environ.get(api_key_env):
        print(f"missing provider API key env: {api_key_env}", file=sys.stderr)
        return 1

    agent_bin = _agent_bin(repo_root, args.agent_bin or None)
    python_exe, python_root = _resolve_python(repo_root)
    print(f"sandbox python={python_exe} root={python_root}")

    tasks = sorted(bank_dir.glob("*.json"))
    if args.task_id:
        wanted = set(args.task_id)
        tasks = [t for t in tasks if t.stem in wanted]
    if not tasks:
        print("no tasks to run", file=sys.stderr)
        return 1

    run_id = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    run_dir = runs_root / run_id
    run_dir.mkdir(parents=True, exist_ok=True)
    (runs_root / "LATEST").write_text(str(run_dir), encoding="utf-8")

    project_root = run_dir / "project"
    _ensure_profile(agent_bin, project_root, agent_id, settings_dir)

    results: list[dict[str, Any]] = []
    passed = 0
    failed = 0

    for task_path in tasks:
        task = json.loads(task_path.read_text(encoding="utf-8-sig"))
        task_id = str(task.get("id") or task_path.stem)
        prompt = str(task.get("prompt") or "")
        expect = task.get("expect") if isinstance(task.get("expect"), dict) else {}

        home_rel = f"homes/{task_id}"
        home_dir = project_root / ".kuibysheff" / home_rel
        if home_dir.exists():
            shutil.rmtree(home_dir)
        home_dir.mkdir(parents=True, exist_ok=True)
        (home_dir / "out").mkdir(parents=True, exist_ok=True)
        (home_dir / "in").mkdir(parents=True, exist_ok=True)

        # Fresh corpus tree per task (workspace grant is project_root/corpus).
        corpus_dir = project_root / "corpus"
        if corpus_dir.exists():
            shutil.rmtree(corpus_dir)

        planted = plant_from_task(task, workspace_root=project_root, home_dir=home_dir)
        verify_planted(planted, workspace_root=project_root, home_dir=home_dir)
        needle_path = run_dir / "needles" / f"{task_id}.json"
        write_needle_sidecar(needle_path, planted)

        log_dir = run_dir / "logs" / task_id
        log_dir.mkdir(parents=True, exist_ok=True)
        run_config = (
            project_root
            / ".kuibysheff"
            / "protected"
            / "agents"
            / agent_id
            / "agent-config.yaml"
        )
        _write_run_config(
            run_config,
            base_text=base_text,
            log_dir=log_dir,
            project_root=project_root,
            python_exe=python_exe,
            python_root=python_root,
        )

        print(f"=== {task_id} kind={planted.kind} needle={planted.needle} ===")
        entry: dict[str, Any] = {
            "id": task_id,
            "pass": False,
            "stop_reason": None,
            "result": None,
            "usage": None,
            "error": None,
            "home": str(home_dir),
            "needle": planted.needle,
            "corpus_kind": planted.kind,
            "failures": [],
            "elapsed_ms": None,
            "logs": None,
        }

        started = time.perf_counter()
        try:
            proc = subprocess.run(
                [
                    str(agent_bin),
                    "run",
                    "--project-root",
                    str(project_root),
                    "--agent",
                    agent_id,
                    "--home",
                    home_rel,
                    "--prompt",
                    prompt,
                ],
                cwd=str(project_root),
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            entry["elapsed_ms"] = int((time.perf_counter() - started) * 1000)
            (home_dir / "agent.stdout.json").write_text(proc.stdout or "", encoding="utf-8")
            (home_dir / "agent.stderr.txt").write_text(proc.stderr or "", encoding="utf-8")

            if proc.returncode != 0 and not (proc.stdout or "").strip():
                entry["error"] = f"agent exit {proc.returncode}: {(proc.stderr or '')[:500]}"
                failed += 1
                print(f"FAIL {task_id}: {entry['error']}")
                results.append(entry)
                continue

            output = _extract_json_object(proc.stdout or "")
            entry["stop_reason"] = output.get("stop_reason")
            entry["result"] = output.get("result")
            entry["usage"] = output.get("usage")
            entry["logs"] = output.get("logs")
            ok, failures = _score_task(output=output, expect=expect, needle=planted.needle)
            entry["failures"] = failures
            entry["pass"] = ok
            if ok:
                passed += 1
                print(f"PASS {task_id} result={entry['result']!r}")
            else:
                failed += 1
                print(f"FAIL {task_id}: {failures}")
        except Exception as exc:  # noqa: BLE001 - harness boundary
            entry["elapsed_ms"] = int((time.perf_counter() - started) * 1000)
            entry["error"] = str(exc)
            failed += 1
            print(f"FAIL {task_id}: {exc}")
        results.append(entry)

    report = {
        "run_id": run_id,
        "bank_dir": str(bank_dir),
        "config": str(config_path),
        "settings": str(settings_dir),
        "agent": agent_id,
        "python": python_exe,
        "passed": passed,
        "failed": failed,
        "total": len(results),
        "tasks": results,
    }
    report_path = run_dir / "report.json"
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Report: {report_path}")
    print(f"Summary: passed={passed} failed={failed} total={len(results)}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
