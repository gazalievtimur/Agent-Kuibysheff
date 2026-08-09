"""Prepare per-run home, one-task AoC bank, and protected agent-config.yaml."""

from __future__ import annotations

import json
import os
import platform
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

from aoc_http import PuzzlePage


DEFAULT_AGENT_ID = "aoc-live"
DEFAULT_HOME_REL = "homes/work"


@dataclass(frozen=True)
class RuntimePaths:
    run_dir: Path
    project_root: Path
    agent_id: str
    home_rel: str
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


def provider_api_key_available(text: str) -> bool:
    """True when the configured api_key_env is present in the environment."""
    api_key_env = yaml_scalar(text, "api_key_env", "OPENAI_API_KEY")
    return bool(os.environ.get(api_key_env, "").strip())


WORKFLOW_DIR = Path(__file__).resolve().parent


def default_project_root(repo_root: Optional[Path] = None) -> Path:
    """Repo monorepo → `local/aoc-live-project`; standalone workflow → `project/`."""
    if repo_root is not None and (repo_root / "Cargo.toml").is_file():
        return (repo_root / "local" / "aoc-live-project").resolve()
    return (WORKFLOW_DIR / "project").resolve()


def protected_profile_dir(project_root: Path, agent_id: str) -> Path:
    return project_root / ".kuibysheff" / "protected" / "agents" / agent_id


def resolve_home_abs(project_root: Path, home_rel: str) -> Path:
    """Resolve `--home` relative under `{project}/.kuibysheff/`."""
    rel = home_rel.replace("\\", "/").strip("/")
    if not rel or rel.startswith("/") or ".." in Path(rel).parts:
        raise ValueError(f"invalid relative --home: {home_rel!r}")
    if rel.split("/")[0] == "protected":
        raise ValueError(f"--home must not be under protected/: {home_rel!r}")
    return (project_root / ".kuibysheff" / Path(rel)).resolve()


def ensure_agent_profile(
    *,
    agent_bin: Path,
    project_root: Path,
    agent_id: str,
    template_dir: Path,
) -> Path:
    """Create/refresh protected profile via `init` + `config import --from`."""
    if not template_dir.is_dir():
        raise FileNotFoundError(f"agent template dir not found: {template_dir}")
    project_root.mkdir(parents=True, exist_ok=True)
    profile = protected_profile_dir(project_root, agent_id)
    config_path = profile / "agent-config.yaml"
    skills_path = profile / "skills.dsl"
    if not (config_path.is_file() and skills_path.is_file()):
        init = subprocess.run(
            [
                str(agent_bin),
                "init",
                agent_id,
                "--project-root",
                str(project_root),
                "--force",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        if init.returncode != 0:
            raise RuntimeError(
                f"agent init failed ({init.returncode}): {init.stderr or init.stdout}"
            )
        imp = subprocess.run(
            [
                str(agent_bin),
                "config",
                "--project-root",
                str(project_root),
                "--agent",
                agent_id,
                "import",
                "--from",
                str(template_dir),
                "--force",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        if imp.returncode != 0:
            raise RuntimeError(
                f"config import failed ({imp.returncode}): {imp.stderr or imp.stdout}"
            )
    return profile


def resolve_agent_binary(
    repo_root: Optional[Path] = None,
    override: Optional[Path] = None,
) -> Path:
    """Resolve agent_Kuibysheff: explicit path, PATH, then optional Cargo fallback."""
    if override is not None:
        path = override.resolve()
        if not path.is_file():
            raise FileNotFoundError(f"agent binary not found: {path}")
        return path

    for name in ("agent_Kuibysheff.exe", "agent_Kuibysheff"):
        found = shutil.which(name)
        if found:
            return Path(found).resolve()

    search_roots: list[Path] = []
    if repo_root is not None:
        search_roots.append(repo_root)
    # Legacy monorepo layout when workflow lives under workflows/<name>/.
    parent = WORKFLOW_DIR.parent
    if parent.name == "workflows":
        search_roots.append(parent.parent)

    seen: set[Path] = set()
    for root in search_roots:
        root = root.resolve()
        if root in seen:
            continue
        seen.add(root)
        release = root / "target" / "release"
        for candidate in (
            release / "agent_Kuibysheff.exe",
            release / "agent_Kuibysheff",
        ):
            if candidate.is_file():
                return candidate.resolve()

    raise FileNotFoundError(
        "agent_Kuibysheff not found on PATH or under target/release "
        "(install the binary, run `cargo build --release`, or pass --agent-bin)"
    )


def resolve_mcp_js(
    workflow_dir: Path = WORKFLOW_DIR,
    override: Optional[Path] = None,
    repo_root: Optional[Path] = None,
) -> Path:
    """Resolve mcp-aoc-tasks.js: explicit path, workflow copy, then repo-root shim."""
    if override is not None:
        path = override.resolve()
        if not path.is_file():
            raise FileNotFoundError(f"mcp-aoc-tasks.js not found: {path}")
        return path

    candidates = [workflow_dir / "mcp-aoc-tasks.js"]
    if repo_root is not None:
        candidates.append(repo_root / "mcp-aoc-tasks.js")
    parent = workflow_dir.parent
    if parent.name == "workflows":
        candidates.append(parent.parent / "mcp-aoc-tasks.js")

    for candidate in candidates:
        if candidate.is_file():
            # Prefer the real workflow asset over a thin root shim that only
            # re-exports; if both exist, workflow_dir wins (first in list).
            return candidate.resolve()

    raise FileNotFoundError(
        f"mcp-aoc-tasks.js not found under {workflow_dir} "
        "(bundle it with the workflow copy unit or pass --mcp-js)"
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
    project_root: Path,
    agent_id: str,
    home_rel: str,
    runs_root: Path,
    base_config: Path,
    puzzle: PuzzlePage,
    puzzle_input: str,
    part: int,
    run_id: str,
    mcp_js: Optional[Path] = None,
    workflow_dir: Path = WORKFLOW_DIR,
) -> RuntimePaths:
    task_id = f"{puzzle.year}-{puzzle.day:02d}-{part}"
    run_dir = runs_root / run_id
    run_dir.mkdir(parents=True, exist_ok=True)

    home = resolve_home_abs(project_root, home_rel)
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
    provider_timeout_ms = yaml_scalar(base_text, "timeout_ms", "120000")
    max_iterations = yaml_scalar(base_text, "max_iterations", "40")
    max_tokens = yaml_scalar(base_text, "max_tokens", "500000")
    max_duration_sec = yaml_scalar(base_text, "max_duration_sec", "900")

    python_exe, python_root, inherit_env = resolve_python_for_sandbox(repo_root)
    resolved_mcp = resolve_mcp_js(
        workflow_dir=workflow_dir,
        override=mcp_js,
        repo_root=repo_root,
    )

    config_path = protected_profile_dir(project_root, agent_id) / "agent-config.yaml"
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(
        render_run_config(
            provider_base_url=provider_base_url,
            provider_model=provider_model,
            provider_api_key_env=provider_api_key_env,
            provider_timeout_ms=provider_timeout_ms,
            max_iterations=max_iterations,
            max_tokens=max_tokens,
            max_duration_sec=max_duration_sec,
            mcp_js=resolved_mcp,
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
        project_root=project_root,
        agent_id=agent_id,
        home_rel=home_rel,
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

    inherit_yaml = json.dumps(inherit_env)

    # Inline provider.api_key is rejected by ConfigSafetyValidator — api_key_env only.
    return f"""provider:
  base_url: "{provider_base_url}"
  model: "{provider_model}"
  api_key_env: "{provider_api_key_env}"
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
