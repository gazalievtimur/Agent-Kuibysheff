#!/usr/bin/env python3
"""Live A2A regression harness (Agent Card, Bearer gate, SendMessage + LLM)."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SCRIPT_DIR = Path(__file__).resolve().parent


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


def provider_api_key_available(text: str) -> bool:
    api_key_env = yaml_scalar(text, "api_key_env", "OPENAI_API_KEY")
    return bool(os.environ.get(api_key_env, "").strip())


AGENT_ID = "a2a-probe"
DEFAULT_TOKEN_ENV = "A2A_LIVE_TOKEN"


@dataclass
class TaskResult:
    id: str
    kind: str
    pass_: bool
    failures: list[str] = field(default_factory=list)
    elapsed_ms: int = 0
    home: str | None = None
    task_state: str | None = None
    response_text: str | None = None
    usage: dict[str, Any] | None = None
    error: str | None = None


def pick_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def load_tasks(bank_dir: Path, task_ids: list[str]) -> list[dict[str, Any]]:
    tasks: list[dict[str, Any]] = []
    for path in sorted(bank_dir.glob("*.json")):
        obj = json.loads(path.read_text(encoding="utf-8"))
        tid = str(obj.get("id") or "")
        if not tid:
            raise SystemExit(f"Task file missing id: {path}")
        if task_ids and tid not in task_ids:
            continue
        obj.setdefault("kind", "send")
        tasks.append(obj)
    if not tasks:
        raise SystemExit("No tasks matched the bank / filter.")
    return tasks


def needs_llm(tasks: list[dict[str, Any]]) -> bool:
    return any(str(t.get("kind", "send")) == "send" for t in tasks)


def run_kbshff(args: list[str], *, cwd: Path | None = None) -> None:
    proc = subprocess.run(args, cwd=cwd, capture_output=True, text=True)
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise RuntimeError(f"kbshff {' '.join(args)} failed: {detail}")


def ensure_profile(agent_bin: Path, project_root: Path, template_dir: Path) -> None:
    profile = project_root / ".kuibysheff" / "protected" / "agents" / AGENT_ID
    if (profile / "agent-config.yaml").is_file() and (profile / "skills.dsl").is_file():
        return
    project_root.mkdir(parents=True, exist_ok=True)
    run_kbshff(
        [str(agent_bin), "init", AGENT_ID, "--project-root", str(project_root), "--force"],
        cwd=project_root.parent,
    )
    run_kbshff(
        [
            str(agent_bin),
            "config",
            "--project-root",
            str(project_root),
            "--agent",
            AGENT_ID,
            "import",
            "--from",
            str(template_dir),
            "--force",
        ],
        cwd=project_root.parent,
    )


def render_run_config(
    *,
    template_text: str,
    output_dir: Path,
    provider_overrides: dict[str, str],
) -> str:
    lines = template_text.splitlines()
    out: list[str] = []
    in_logging = False
    in_provider = False
    for line in lines:
        if re.match(r"^\s*logging:\s*$", line):
            in_logging = True
            in_provider = False
            out.append(line)
            continue
        if re.match(r"^\s*provider:\s*$", line):
            in_provider = True
            in_logging = False
            out.append(line)
            continue
        if in_logging and re.match(r"^\S", line):
            in_logging = False
        if in_provider and re.match(r"^\S", line):
            in_provider = False
        if in_logging and re.match(r"^\s*output_dir:", line):
            out.append(f'  output_dir: "{output_dir.as_posix()}"')
            continue
        replaced = False
        if in_provider:
            for key, value in provider_overrides.items():
                pat = rf"^(\s*{re.escape(key)}:\s*).*$"
                m = re.match(pat, line)
                if m:
                    if key in ("base_url", "model", "api_key_env"):
                        out.append(f'{m.group(1)}"{value}"')
                    else:
                        out.append(f"{m.group(1)}{value}")
                    replaced = True
                    break
        if not replaced:
            out.append(line)
    return "\n".join(out) + "\n"


def http_json(
    method: str,
    url: str,
    *,
    body: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
) -> tuple[int, dict[str, Any] | None]:
    data = None
    req_headers = {"Accept": "application/json"}
    if headers:
        req_headers.update(headers)
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        req_headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=req_headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            raw = resp.read().decode("utf-8")
            status = resp.status
    except urllib.error.HTTPError as exc:
        status = exc.code
        raw = exc.read().decode("utf-8", errors="replace")
    parsed: dict[str, Any] | None = None
    if raw.strip():
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            parsed = None
    return status, parsed


def wait_for_server(base_url: str, timeout_sec: float = 30.0) -> None:
    deadline = time.monotonic() + timeout_sec
    url = f"{base_url}/.well-known/agent-card.json"
    while time.monotonic() < deadline:
        try:
            status, _ = http_json("GET", url)
            if status == 200:
                return
        except urllib.error.URLError:
            pass
        time.sleep(0.15)
    raise TimeoutError(f"A2A server did not become ready at {base_url}")


def fetch_card(base_url: str) -> dict[str, Any]:
    status, card = http_json("GET", f"{base_url}/.well-known/agent-card.json")
    if status != 200 or not isinstance(card, dict):
        raise RuntimeError(f"agent card GET failed: status={status}")
    return card


def rpc_send_message(base_url: str, text: str, *, token: str | None = None) -> dict[str, Any]:
    headers: dict[str, str] = {}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": f"m-{uuid.uuid4()}",
                "role": "ROLE_USER",
                "parts": [{"text": text}],
            }
        },
    }
    status, body = http_json("POST", f"{base_url}/jsonrpc", body=payload, headers=headers)
    if status != 200 or not isinstance(body, dict):
        raise RuntimeError(f"SendMessage RPC failed: status={status} body={body!r}")
    if "error" in body:
        raise RuntimeError(f"SendMessage RPC error: {body['error']}")
    result = body.get("result")
    if not isinstance(result, dict):
        raise RuntimeError(f"SendMessage unexpected result: {result!r}")
    return result


def task_state_name(task: dict[str, Any]) -> str:
    status = task.get("status") or {}
    state = status.get("state") or status.get("taskState") or ""
    return str(state).lower().replace("task_state_", "")


def agent_message_text(task: dict[str, Any]) -> str:
    status = task.get("status") or {}
    message = status.get("message") or {}
    parts = message.get("parts") or []
    texts: list[str] = []
    for part in parts:
        if isinstance(part, dict):
            if "text" in part:
                texts.append(str(part["text"]))
            elif isinstance(part.get("content"), dict) and "text" in part["content"]:
                texts.append(str(part["content"]["text"]))
    return "\n".join(texts)


def normalize_task(task_payload: dict[str, Any]) -> dict[str, Any]:
    if "task" in task_payload and isinstance(task_payload["task"], dict):
        return task_payload["task"]
    return task_payload


def assert_card(card: dict[str, Any], expect: dict[str, Any]) -> list[str]:
    failures: list[str] = []
    want_name = expect.get("card_name")
    if want_name and card.get("name") != want_name:
        failures.append(f"card.name expected {want_name!r}, got {card.get('name')!r}")
    urls = [
        str(item.get("url") or "")
        for item in (card.get("supportedInterfaces") or card.get("supported_interfaces") or [])
        if isinstance(item, dict)
    ]
    for needle in expect.get("interfaces_contain") or []:
        if not any(needle in url for url in urls):
            failures.append(f"supportedInterfaces missing substring {needle!r} in {urls!r}")
    if expect.get("streaming") is True:
        caps = card.get("capabilities") or {}
        if caps.get("streaming") is not True:
            failures.append(f"capabilities.streaming expected true, got {caps.get('streaming')!r}")
    return failures


def assert_bearer(base_url: str, expect: dict[str, Any], token: str) -> list[str]:
    failures: list[str] = []
    if expect.get("card_public") is True:
        status, _ = http_json("GET", f"{base_url}/.well-known/agent-card.json")
        if status != 200:
            failures.append(f"agent card should stay public, got status {status}")
    want = int(expect.get("rpc_without_token_status") or 401)
    status, _ = http_json(
        "POST",
        f"{base_url}/jsonrpc",
        body={
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": "probe-unauth",
                    "role": "ROLE_USER",
                    "parts": [{"text": "ping"}],
                }
            },
        },
    )
    if status != want:
        failures.append(f"unauthenticated RPC expected status {want}, got {status}")
    ok_status, _ = http_json(
        "POST",
        f"{base_url}/jsonrpc",
        body={
            "jsonrpc": "2.0",
            "id": 2,
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": "probe-auth",
                    "role": "ROLE_USER",
                    "parts": [{"text": "ping"}],
                }
            },
        },
        headers={"Authorization": f"Bearer {token}"},
    )
    if ok_status == want:
        failures.append("authenticated RPC should not return the unauthenticated status")
    return failures


def read_home_file(home_dir: Path, rel: str) -> str:
    path = home_dir / rel.replace("/", os.sep)
    if not path.is_file():
        raise FileNotFoundError(rel)
    return path.read_text(encoding="utf-8")


def assert_send(
    *,
    home_dir: Path,
    task: dict[str, Any],
    expect: dict[str, Any],
) -> list[str]:
    failures: list[str] = []
    want_state = str(expect.get("task_state") or "completed").lower()
    got_state = task_state_name(task)
    if want_state not in got_state:
        failures.append(f"task state expected {want_state!r}, got {got_state!r}")
    response = agent_message_text(task)
    needle = expect.get("response_contains")
    if needle and needle not in response:
        failures.append(f"response missing {needle!r}: {response!r}")
    for rel in expect.get("written_paths") or []:
        if not (home_dir / rel.replace("/", os.sep)).is_file():
            failures.append(f"missing home file {rel}")
    exact = expect.get("file_exact") or {}
    for rel, want in exact.items():
        try:
            got = read_home_file(home_dir, rel)
        except FileNotFoundError:
            failures.append(f"missing home file {rel}")
            continue
        if got.strip() != str(want).strip():
            failures.append(f"{rel}: expected {want!r}, got {got.strip()!r}")
    return failures


def start_server(
    *,
    agent_bin: Path,
    project_root: Path,
    home_rel: str,
    port: int,
    token_env: str | None,
) -> subprocess.Popen[str]:
    args = [
        str(agent_bin),
        "a2a",
        "--project-root",
        str(project_root),
        "--agent",
        AGENT_ID,
        "--home",
        home_rel,
        "--bind",
        f"127.0.0.1:{port}",
        "--public-url",
        f"http://127.0.0.1:{port}",
    ]
    if token_env:
        args.extend(["--token-env", token_env])
    env = os.environ.copy()
    if token_env and not env.get(token_env, "").strip():
        env[token_env] = "a2a-live-smoke-token"
    return subprocess.Popen(
        args,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )


def stop_server(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def run_task(
    *,
    agent_bin: Path,
    template_dir: Path,
    config_text: str,
    provider_overrides: dict[str, str],
    runs_root: Path,
    task: dict[str, Any],
) -> TaskResult:
    tid = str(task["id"])
    kind = str(task.get("kind") or "send")
    started = time.monotonic()
    failures: list[str] = []
    home_rel = f"homes/{tid}"
    project_root = runs_root / tid / "project"
    home_dir = project_root / ".kuibysheff" / home_rel.replace("/", os.sep)
    log_dir = runs_root / tid / "logs"
    for sub in ("in", "out"):
        (home_dir / sub).mkdir(parents=True, exist_ok=True)
    log_dir.mkdir(parents=True, exist_ok=True)

    ensure_profile(agent_bin, project_root, template_dir)
    cfg_path = project_root / ".kuibysheff" / "protected" / "agents" / AGENT_ID / "agent-config.yaml"
    cfg_path.write_text(
        render_run_config(
            template_text=config_text,
            output_dir=log_dir,
            provider_overrides=provider_overrides,
        ),
        encoding="utf-8",
    )

    server_cfg = task.get("server") or {}
    token_env = server_cfg.get("token_env")
    port = pick_free_port()
    base_url = f"http://127.0.0.1:{port}"
    proc = start_server(
        agent_bin=agent_bin,
        project_root=project_root,
        home_rel=home_rel,
        port=port,
        token_env=str(token_env) if token_env else None,
    )
    task_payload: dict[str, Any] | None = None
    response_text: str | None = None
    try:
        wait_for_server(base_url)
        if kind == "card":
            card = fetch_card(base_url)
            failures.extend(assert_card(card, task.get("expect") or {}))
        elif kind == "bearer":
            token = os.environ.get(str(token_env or DEFAULT_TOKEN_ENV), "a2a-live-smoke-token")
            failures.extend(assert_bearer(base_url, task.get("expect") or {}, token))
        elif kind == "send":
            message = str(task.get("message") or "")
            if not message.strip():
                failures.append("send task missing message")
            else:
                token = None
                if token_env:
                    token = os.environ.get(str(token_env), "a2a-live-smoke-token")
                raw = rpc_send_message(base_url, message, token=token)
                task_payload = normalize_task(raw)
                response_text = agent_message_text(task_payload)
                failures.extend(
                    assert_send(
                        home_dir=home_dir,
                        task=task_payload,
                        expect=task.get("expect") or {},
                    )
                )
        else:
            failures.append(f"unknown task kind {kind!r}")
    except Exception as exc:  # noqa: BLE001 — collect per-task failure for report
        failures.append(str(exc))
    finally:
        stop_server(proc)

    elapsed_ms = int((time.monotonic() - started) * 1000)
    usage = None
    if task_payload:
        meta = task_payload.get("metadata") or {}
        usage = meta.get("kuibysheff.usage")
    return TaskResult(
        id=tid,
        kind=kind,
        pass_=not failures,
        failures=failures,
        elapsed_ms=elapsed_ms,
        home=str(home_dir),
        task_state=task_state_name(task_payload) if task_payload else None,
        response_text=response_text,
        usage=usage if isinstance(usage, dict) else None,
        error=failures[0] if failures else None,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Live A2A regression eval")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--bank-dir", type=Path, default=None)
    parser.add_argument("--config", type=Path, default=None)
    parser.add_argument("--settings-dir", type=Path, default=None)
    parser.add_argument("--runs-root", type=Path, default=None)
    parser.add_argument("--agent-bin", type=Path, default=None)
    parser.add_argument("--task-id", action="append", default=[])
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    bank_dir = (args.bank_dir or repo_root / "local" / "a2a-bank").resolve()
    if not bank_dir.is_dir():
        raise SystemExit(f"A2A bank not found: {bank_dir}\nCopy local/a2a-bank.example -> local/a2a-bank")

    local_config = repo_root / "agent-config.local.yaml"
    example_config = repo_root / "test-agents" / "a2a-probe" / "agent-config.example.yaml"
    provider_config_path = (args.config or (local_config if local_config.is_file() else example_config)).resolve()
    if not provider_config_path.is_file():
        raise SystemExit(f"Config not found: {provider_config_path}")

    settings_dir = (args.settings_dir or repo_root / "test-agents" / "a2a-probe").resolve()
    if not settings_dir.is_dir():
        raise SystemExit(f"Settings dir not found: {settings_dir}")

    template_config_path = settings_dir / "agent-config.example.yaml"
    if not template_config_path.is_file():
        raise SystemExit(f"A2A template config not found: {template_config_path}")

    agent_bin = (args.agent_bin or repo_root / "target" / "release" / ("kbshff.exe" if os.name == "nt" else "kbshff")).resolve()
    if not agent_bin.is_file():
        raise SystemExit(f"Release binary missing: {agent_bin}\nRun: cargo build --release --bin kbshff")

    tasks = load_tasks(bank_dir, args.task_id)
    template_text = template_config_path.read_text(encoding="utf-8")
    provider_text = provider_config_path.read_text(encoding="utf-8")
    if needs_llm(tasks) and not provider_api_key_available(provider_text):
        api_key_env = yaml_scalar(provider_text, "api_key_env", "OPENAI_API_KEY")
        raise SystemExit(
            f"A2A send tasks require provider API key env {api_key_env} (or agent-config.local.yaml)"
        )

    provider_overrides = {
        "base_url": yaml_scalar(provider_text, "base_url", "https://api.openai.com/v1"),
        "model": yaml_scalar(provider_text, "model", "gpt-4o-mini"),
        "api_key_env": yaml_scalar(provider_text, "api_key_env", "OPENAI_API_KEY"),
        "timeout_ms": yaml_scalar(provider_text, "timeout_ms", "120000"),
        "max_retries": yaml_scalar(provider_text, "max_retries", "3"),
        "retry_base_delay_ms": yaml_scalar(provider_text, "retry_base_delay_ms", "500"),
    }

    run_id = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    runs_root = (args.runs_root or repo_root / "local" / "a2a-runs" / run_id).resolve()
    runs_root.mkdir(parents=True, exist_ok=True)

    results: list[TaskResult] = []
    for task in tasks:
        print(f"A2A task {task['id']} ({task.get('kind', 'send')})...")
        results.append(
            run_task(
                agent_bin=agent_bin,
                template_dir=settings_dir,
                config_text=template_text,
                provider_overrides=provider_overrides,
                runs_root=runs_root,
                task=task,
            )
        )

    passed = sum(1 for r in results if r.pass_)
    failed = len(results) - passed
    report = {
        "run_id": run_id,
        "bank_dir": str(bank_dir),
        "config": str(provider_config_path),
        "template_config": str(template_config_path),
        "settings": str(settings_dir),
        "agent": AGENT_ID,
        "passed": passed,
        "failed": failed,
        "total": len(results),
        "tasks": [
            {
                "id": r.id,
                "kind": r.kind,
                "pass": r.pass_,
                "failures": r.failures,
                "elapsed_ms": r.elapsed_ms,
                "home": r.home,
                "task_state": r.task_state,
                "response_text": r.response_text,
                "usage": r.usage,
                "error": r.error,
            }
            for r in results
        ],
    }
    report_path = runs_root / "report.json"
    report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    latest = repo_root / "local" / "a2a-runs" / "LATEST"
    latest.write_text(str(runs_root) + "\n", encoding="utf-8")

    print(f"A2A eval run={run_id} passed={passed} failed={failed} report={report_path}")
    for r in results:
        if not r.pass_:
            print(f"  FAIL {r.id}: {'; '.join(r.failures)}", file=sys.stderr)
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
