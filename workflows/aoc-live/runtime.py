"""Prepare per-run home, one-task AoC bank, and agent-config.yaml."""

from __future__ import annotations

import json
import os
import platform
import re
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

from aoc_http import PuzzlePage


@dataclass(frozen=True)
class RuntimePaths:
    run_dir: Path
    home: Path
    bank_dir: Path
    config_path: Path
    log_dir: Path
    task_id: str


def load_dotenv(path: Path) -> None:
    if not path.is_file():
        return
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip().strip("'").strip('"')
        if key and key not in os.environ:
            os.environ[key] = value


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


def resolve_agent_binary(repo_root: Path, override: Optional[Path] = None) -> Path:
    if override is not None:
        path = override.resolve()
        if not path.is_file():
            raise FileNotFoundError(f"agent binary not found: {path}")
        return path
    release = repo_root / "target" / "release"
    candidates = [
        release / "agent_Kuibyshev.exe",
        release / "agent_Kuibyshev",
    ]
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise FileNotFoundError(
        "Release binary missing under target/release "
        "(run `cargo build --release` first, or pass --agent-bin)"
    )


def resolve_python_for_sandbox(repo_root: Path) -> tuple[Path, Path, list[str]]:
    """Return (executable, runtime_root, inherit_env)."""
    staged = repo_root / "local" / "aoc-sandbox-runtime" / "python"
    staged_exe = staged / ("python.exe" if os.name == "nt" else "python")
    if staged_exe.is_file():
        inherit = ["SYSTEMROOT", "SystemRoot"] if os.name == "nt" else []
        return staged_exe.resolve(), staged.resolve(), inherit

    for name in ("python", "python3"):
        found = shutil.which(name)
        if not found:
            continue
        if "WindowsApps" in found:
            continue
        exe = Path(found).resolve()
        root = exe.parent
        inherit = ["SYSTEMROOT", "SystemRoot"] if os.name == "nt" else []
        return exe, root, inherit

    raise FileNotFoundError(
        "Could not resolve python for sandboxed home.run "
        "(stage via aoc-eval/aoc-regression or install Python on PATH)"
    )


def prepare_runtime(
    *,
    repo_root: Path,
    runs_root: Path,
    base_config: Path,
    puzzle: PuzzlePage,
    puzzle_input: str,
    part: int,
    run_id: str,
) -> RuntimePaths:
    task_id = f"{puzzle.year}-{puzzle.day:02d}-{part}"
    run_dir = runs_root / run_id
    home = run_dir / "home"
    bank_dir = run_dir / "bank"
    log_dir = home / "logs"
    for path in (home / "in", home / "out", bank_dir, log_dir):
        path.mkdir(parents=True, exist_ok=True)

    bank_task = {
        "id": task_id,
        "url": puzzle.url,
        "title": puzzle.title,
        "text": puzzle.text,
        "input": puzzle_input.rstrip("\n") + "\n",
    }
    (bank_dir / f"{task_id}.json").write_text(
        json.dumps(bank_task, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    # Seed input.txt so the solver can run even if aoc_get_input is skipped.
    (home / "input.txt").write_text(puzzle_input, encoding="utf-8")

    base_text = base_config.read_text(encoding="utf-8")
    provider_base_url = yaml_scalar(base_text, "base_url", "https://api.openai.com/v1")
    provider_model = yaml_scalar(base_text, "model", "gpt-4o")
    provider_api_key_env = yaml_scalar(base_text, "api_key_env", "OPENAI_API_KEY")
    provider_api_key = yaml_provider_api_key(base_text)
    provider_timeout_ms = yaml_scalar(base_text, "timeout_ms", "120000")
    max_iterations = yaml_scalar(base_text, "max_iterations", "40")
    max_tokens = yaml_scalar(base_text, "max_tokens", "500000")
    max_duration_sec = yaml_scalar(base_text, "max_duration_sec", "900")

    python_exe, python_root, inherit_env = resolve_python_for_sandbox(repo_root)
    mcp_js = (repo_root / "mcp-aoc-tasks.js").resolve()
    if not mcp_js.is_file():
        raise FileNotFoundError(f"mcp-aoc-tasks.js not found: {mcp_js}")

    config_path = home / "agent-config.yaml"
    config_path.write_text(
        render_run_config(
            provider_base_url=provider_base_url,
            provider_model=provider_model,
            provider_api_key=provider_api_key,
            provider_api_key_env=provider_api_key_env,
            provider_timeout_ms=provider_timeout_ms,
            max_iterations=max_iterations,
            max_tokens=max_tokens,
            max_duration_sec=max_duration_sec,
            mcp_js=mcp_js,
            bank_dir=bank_dir,
            home=home,
            log_dir=log_dir,
            python_exe=python_exe,
            python_root=python_root,
            inherit_env=inherit_env,
        ),
        encoding="utf-8",
    )

    return RuntimePaths(
        run_dir=run_dir,
        home=home,
        bank_dir=bank_dir,
        config_path=config_path,
        log_dir=log_dir,
        task_id=task_id,
    )


def render_run_config(
    *,
    provider_base_url: str,
    provider_model: str,
    provider_api_key: str,
    provider_api_key_env: str,
    provider_timeout_ms: str,
    max_iterations: str,
    max_tokens: str,
    max_duration_sec: str,
    mcp_js: Path,
    bank_dir: Path,
    home: Path,
    log_dir: Path,
    python_exe: Path,
    python_root: Path,
    inherit_env: list[str],
) -> str:
    def posix(path: Path) -> str:
        return str(path.resolve()).replace("\\", "/")

    api_key_line = ""
    if provider_api_key:
        escaped = provider_api_key.replace('"', '\\"')
        api_key_line = f'  api_key: "{escaped}"\n'

    inherit_yaml = json.dumps(inherit_env)

    return f"""provider:
  base_url: "{provider_base_url}"
  model: "{provider_model}"
{api_key_line}  api_key_env: "{provider_api_key_env}"
  timeout_ms: {provider_timeout_ms}
  max_retries: 3
  retry_base_delay_ms: 500

mcp:
  - name: "aoc"
    command: "node"
    args:
      - "{posix(mcp_js)}"
      - "--bank-dir={posix(bank_dir)}"
      - "--home-dir={posix(home)}"
    env:
      AOC_BANK_DIR: "{posix(bank_dir)}"
      AOC_HOME_DIR: "{posix(home)}"
    timeout_ms: 30000

limits:
  max_iterations: {max_iterations}
  max_tokens: {max_tokens}
  max_duration_sec: {max_duration_sec}

logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: true
  output_dir: "{posix(log_dir)}"

access:
  mode: strict
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
      - home.run
  filesystem:
    home:
      read: [".", "in", "out"]
      write: [".", "out"]
  run:
    programs:
      - name: python
        executable: "{posix(python_exe)}"
        runtime_read_roots: ["{posix(python_root)}"]
        inherit_env: {inherit_yaml}
        allow_children: false
    max_args: 32
    max_arg_chars: 4096
    max_output_chars: 200000
    max_timeout_ms: 120000
"""


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def host_label() -> str:
    return f"{platform.system()}-{platform.machine()}"
