"""Runtime helpers for SWE-bench Verified orchestration."""

from __future__ import annotations

import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional, Sequence

from swebench_adapter import (
    DATASET_NAME,
    DATASET_SPLIT,
    SafeInstance,
    assert_no_oracle_leak,
    instance_image_key,
    project_safe,
    swebench_version,
)

WORKFLOW_DIR = Path(__file__).resolve().parent
SCHEMA_VERSION = 1

DEFAULT_AGENT_ID = "swebench-solver"
DEFAULT_HOME_REL = "homes/work"

STATUS_OK = "ok"
STATUS_EMPTY_PATCH = "empty_patch"
STATUS_AGENT_ERROR = "agent_error"
STATUS_INFRA_ERROR = "infra_error"
STATUS_INVALID_PATCH = "invalid_patch"

TERMINAL_SUCCESS = frozenset({STATUS_OK})
RETRYABLE = frozenset(
    {STATUS_EMPTY_PATCH, STATUS_AGENT_ERROR, STATUS_INFRA_ERROR, STATUS_INVALID_PATCH}
)

MAX_PATCH_BYTES = 8 * 1024 * 1024


@dataclass(frozen=True)
class InstancePaths:
    instance_dir: Path
    status_path: Path
    patch_path: Path
    run_output_path: Path
    stderr_path: Path
    provenance_path: Path
    project_root: Path
    agent_id: str
    home_rel: str
    home: Path
    config_path: Path
    log_dir: Path


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


def protected_profile_dir(project_root: Path, agent_id: str) -> Path:
    return project_root / ".kuibysheff" / "protected" / "agents" / agent_id


def resolve_home_abs(project_root: Path, home_rel: str) -> Path:
    """Resolve `--home` relative under `{project}/.kuibysheff/`."""
    rel = home_rel.replace("\\", "/").strip("/")
    if not rel or ".." in Path(rel).parts:
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
    if config_path.is_file() and skills_path.is_file():
        return profile

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


def escape_yaml_double(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


def posix(path: Path) -> str:
    return str(path.resolve()).replace("\\", "/")


def file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def default_run_id() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def host_label() -> str:
    return f"{platform.system()}-{platform.machine()}"


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


def resolve_python() -> Path:
    for name in ("python", "python3"):
        found = shutil.which(name)
        if found and "WindowsApps" not in found:
            return Path(found).resolve()
    raise FileNotFoundError("python not found on PATH")


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


def write_json_atomic(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=path.name + ".", dir=str(path.parent))
    tmp = Path(tmp_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            json.dump(obj, fh, ensure_ascii=False, indent=2)
            fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        tmp.replace(path)
    finally:
        if tmp.exists():
            tmp.unlink(missing_ok=True)


def write_text_atomic(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=path.name + ".", dir=str(path.parent))
    tmp = Path(tmp_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as fh:
            fh.write(text)
            if text and not text.endswith("\n"):
                fh.write("\n")
            fh.flush()
            os.fsync(fh.fileno())
        tmp.replace(path)
    finally:
        if tmp.exists():
            tmp.unlink(missing_ok=True)


def instance_paths(
    run_dir: Path,
    instance_id: str,
    *,
    agent_id: str = DEFAULT_AGENT_ID,
    home_rel: str = DEFAULT_HOME_REL,
) -> InstancePaths:
    instance_dir = run_dir / "instances" / instance_id
    project_root = instance_dir
    home = resolve_home_abs(project_root, home_rel)
    log_dir = home / "logs"
    config_path = protected_profile_dir(project_root, agent_id) / "agent-config.yaml"
    return InstancePaths(
        instance_dir=instance_dir,
        status_path=instance_dir / "status.json",
        patch_path=instance_dir / "model.patch",
        run_output_path=instance_dir / "run-output.json",
        stderr_path=instance_dir / "agent.stderr.txt",
        provenance_path=instance_dir / "provenance.json",
        project_root=project_root,
        agent_id=agent_id,
        home_rel=home_rel,
        home=home,
        config_path=config_path,
        log_dir=log_dir,
    )


def prepare_instance_dirs(paths: InstancePaths) -> None:
    for p in (paths.home / "in", paths.home / "out", paths.log_dir):
        p.mkdir(parents=True, exist_ok=True)


def read_status(paths: InstancePaths) -> Optional[dict[str, Any]]:
    if not paths.status_path.is_file():
        return None
    return json.loads(paths.status_path.read_text(encoding="utf-8"))


def should_skip_instance(status: Optional[dict[str, Any]], *, resume: bool) -> bool:
    if not resume or not status:
        return False
    if status.get("status") != STATUS_OK:
        return False
    patch = status.get("patch_path")
    if not patch:
        return False
    return Path(patch).is_file() and Path(patch).stat().st_size > 0


def render_agent_prompt(safe: SafeInstance) -> str:
    payload = safe.to_dict()
    assert_no_oracle_leak(payload)
    return (
        "Solve the following SWE-bench Verified issue.\n"
        "Use only workspace.* MCP tools. Work only under /testbed.\n"
        "Do not ask for oracle patches or hidden tests.\n"
        "When finished, set done=true and put a short summary in result "
        "(the orchestrator extracts the git patch).\n\n"
        f"instance_id: {safe.instance_id}\n"
        f"repo: {safe.repo}\n"
        f"base_commit: {safe.base_commit}\n\n"
        "problem_statement:\n"
        f"{safe.problem_statement}\n"
    )


def render_run_config(
    *,
    base_config_text: str,
    mcp_script: Path,
    container_id: str,
    log_dir: Path,
    python_exe: Path,
) -> str:
    provider_base_url = yaml_scalar(base_config_text, "base_url", "https://api.openai.com/v1")
    provider_model = yaml_scalar(base_config_text, "model", "gpt-4o")
    provider_api_key_env = yaml_scalar(base_config_text, "api_key_env", "OPENAI_API_KEY")
    provider_timeout_ms = yaml_scalar(base_config_text, "timeout_ms", "120000")
    max_iterations = yaml_scalar(base_config_text, "max_iterations", "80")
    max_tokens = yaml_scalar(base_config_text, "max_tokens", "800000")
    max_duration_sec = yaml_scalar(base_config_text, "max_duration_sec", "1800")

    # Inline provider.api_key is rejected by ConfigSafetyValidator — api_key_env only.
    return f"""provider:
  base_url: "{escape_yaml_double(provider_base_url)}"
  model: "{escape_yaml_double(provider_model)}"
  api_key_env: "{escape_yaml_double(provider_api_key_env)}"
  timeout_ms: {provider_timeout_ms}
  max_retries: 3
  retry_base_delay_ms: 500

mcp:
  - name: "workspace"
    command: "{posix(python_exe)}"
    args:
      - "{posix(mcp_script)}"
    env:
      SWEBENCH_CONTAINER_ID: "{escape_yaml_double(container_id)}"
    timeout_ms: 180000

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
    builtins: []
  filesystem:
    home:
      read: [".", "in", "out"]
      write: [".", "out"]
  run:
    programs: []
    max_args: 32
    max_arg_chars: 4096
    max_output_chars: 200000
    max_timeout_ms: 120000
"""


def docker_from_env():
    import docker

    return docker.from_env()


def check_docker_linux() -> dict[str, Any]:
    client = docker_from_env()
    info = client.info()
    os_type = str(info.get("OSType") or "")
    arch = str(info.get("Architecture") or "")
    if os_type.lower() != "linux":
        raise RuntimeError(
            f"Docker Linux engine required for MVP; got OSType={os_type!r}"
        )
    return {
        "os_type": os_type,
        "architecture": arch,
        "server_version": info.get("ServerVersion"),
        "n_cpu": info.get("NCPU"),
        "mem_total": info.get("MemTotal"),
    }

def pull_instance_image(client: Any, image: str) -> str:
    """Pull image if needed; return image digest/id."""
    try:
        img = client.images.get(image)
    except Exception:
        client.images.pull(image)
        img = client.images.get(image)
    digests = img.attrs.get("RepoDigests") or []
    if digests:
        return str(digests[0])
    return str(img.id)


def start_task_container(
    client: Any,
    *,
    image: str,
    run_id: str,
    instance_id: str,
) -> Any:
    safe_name = re.sub(r"[^a-zA-Z0-9_.-]", "_", f"sweb-{run_id}-{instance_id}")[:63]
    # Remove any leftover with same name.
    try:
        old = client.containers.get(safe_name)
        old.remove(force=True)
    except Exception:
        pass

    kwargs: dict[str, Any] = {
        "image": image,
        "command": ["sleep", "infinity"],
        "detach": True,
        "name": safe_name,
        "network_disabled": True,
        "labels": {
            "swebench.workflow": "swebench-verified",
            "swebench.run_id": run_id,
            "swebench.instance_id": instance_id,
        },
        # Do not pass host env, volumes, or docker socket.
    }
    # Resource limits when compatible.
    kwargs["mem_limit"] = "4g"
    kwargs["nano_cpus"] = 2_000_000_000
    kwargs["pids_limit"] = 512
    kwargs["security_opt"] = ["no-new-privileges:true"]

    try:
        return client.containers.run(**kwargs)
    except Exception:
        # Some images reject nano_cpus/security_opt; retry minimal.
        kwargs.pop("nano_cpus", None)
        kwargs.pop("security_opt", None)
        kwargs.pop("pids_limit", None)
        return client.containers.run(**kwargs)


def remove_container(container: Any) -> None:
    if container is None:
        return
    try:
        container.remove(force=True)
    except Exception:
        pass


def container_exec(
    container: Any,
    cmd: list[str],
    *,
    workdir: str = "/testbed",
) -> tuple[int, str, str]:
    exit_code, output = container.exec_run(
        cmd,
        workdir=workdir,
        demux=True,
    )
    stdout_b, stderr_b = output if isinstance(output, tuple) else (output, b"")
    stdout = (stdout_b or b"").decode("utf-8", errors="replace")
    stderr = (stderr_b or b"").decode("utf-8", errors="replace")
    return int(exit_code), stdout, stderr


def extract_model_patch(container: Any) -> tuple[str, str]:
    """Extract a valid unified diff including untracked files.

    Returns (patch_text, status) where status is ok/empty_patch/invalid_patch.
    """
    code, status_out, status_err = container_exec(
        container, ["git", "status", "--porcelain", "-z"]
    )
    if code != 0:
        raise RuntimeError(f"git status failed: {status_err or status_out}")

    untracked: list[str] = []
    # porcelain -z: entries separated by NUL; untracked are ?? path
    entries = [e for e in status_out.split("\0") if e]
    for entry in entries:
        if entry.startswith("?? "):
            untracked.append(entry[3:])
        elif entry.startswith("??"):
            # format without space in some git versions
            untracked.append(entry[2:].lstrip())

    for path in untracked:
        # Intent-to-add so untracked files appear in git diff
        c, out, err = container_exec(container, ["git", "add", "-N", "--", path])
        if c != 0:
            raise RuntimeError(f"git add -N failed for {path}: {err or out}")

    code, diff_out, diff_err = container_exec(
        container, ["git", "diff", "--binary", "--no-ext-diff"]
    )
    if code not in (0, 1):
        raise RuntimeError(f"git diff failed: {diff_err or diff_out}")

    patch = diff_out
    if not patch.strip():
        return "", STATUS_EMPTY_PATCH

    if len(patch.encode("utf-8")) > MAX_PATCH_BYTES:
        return patch, STATUS_INVALID_PATCH

    code, _check_out, _check_err = container_exec(
        container, ["bash", "-lc", "git diff --check"]
    )
    if code != 0:
        return patch, STATUS_INVALID_PATCH

    # Validate apply on a clean temporary worktree copy inside container.
    b64 = __import__("base64").b64encode(patch.encode("utf-8")).decode("ascii")
    script = (
        "set -euo pipefail\n"
        f"echo {b64!r} | base64 -d > /tmp/model.patch\n"
        "TMP=$(mktemp -d)\n"
        "trap 'rm -rf \"$TMP\"' EXIT\n"
        "git archive HEAD | tar -x -C \"$TMP\"\n"
        "cd \"$TMP\"\n"
        "git apply --check /tmp/model.patch\n"
    )
    code, _out, _err = container_exec(container, ["bash", "-lc", script])
    if code != 0:
        return patch, STATUS_INVALID_PATCH
    return patch, STATUS_OK


def run_agent(
    *,
    agent_bin: Path,
    project_root: Path,
    agent_id: str,
    prompt: str,
    home_rel: str,
    run_id: str,
) -> tuple[int, str, str]:
    cmd = [
        str(agent_bin),
        "run",
        "--project-root",
        str(project_root),
        "--agent",
        agent_id,
        "--prompt",
        prompt,
        "--home",
        home_rel,
        "--run-id",
        run_id,
        "--save-chat-history",
    ]
    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return proc.returncode, proc.stdout, proc.stderr


def classify_agent_result(
    returncode: int, stdout: str
) -> tuple[str, Optional[dict[str, Any]]]:
    """Return (status, run_output_or_none). Non-zero exit may still have JSON."""
    try:
        obj = extract_json_object(stdout)
    except (ValueError, json.JSONDecodeError):
        return STATUS_AGENT_ERROR, None

    stop = str(obj.get("stop_reason") or "")
    if stop == "error" or (returncode != 0 and stop not in ("goal_reached", "limit_reached")):
        # Still keep parsed output for usage.
        if stop == "error":
            return STATUS_AGENT_ERROR, obj
        if returncode != 0 and stop not in ("goal_reached", "limit_reached"):
            return STATUS_AGENT_ERROR, obj
    return STATUS_OK, obj


def reduce_predictions(
    run_dir: Path,
    *,
    model_name_or_path: str,
) -> Path:
    """Deterministically build predictions.jsonl from instance dirs."""
    instances_root = run_dir / "instances"
    rows: list[dict[str, str]] = []
    if instances_root.is_dir():
        for instance_dir in sorted(instances_root.iterdir(), key=lambda p: p.name):
            if not instance_dir.is_dir():
                continue
            status_path = instance_dir / "status.json"
            patch_path = instance_dir / "model.patch"
            if not status_path.is_file():
                continue
            status = json.loads(status_path.read_text(encoding="utf-8"))
            if status.get("status") != STATUS_OK:
                continue
            if not patch_path.is_file():
                continue
            patch = patch_path.read_text(encoding="utf-8")
            if not patch.strip():
                continue
            rows.append(
                {
                    "instance_id": instance_dir.name,
                    "model_name_or_path": model_name_or_path,
                    "model_patch": patch if patch.endswith("\n") else patch + "\n",
                }
            )
    out = run_dir / "predictions.jsonl"
    lines = [json.dumps(r, ensure_ascii=False) for r in rows]
    write_text_atomic(out, "\n".join(lines) + ("\n" if lines else ""))
    return out


def generate_one_instance(
    *,
    raw: dict[str, Any],
    run_dir: Path,
    run_id: str,
    repo_root: Path,
    base_config: Path,
    settings_dir: Path,
    agent_bin: Path,
    model_name_or_path: str,
    resume: bool,
    agent_id: str = DEFAULT_AGENT_ID,
    home_rel: str = DEFAULT_HOME_REL,
) -> dict[str, Any]:
    del repo_root  # reserved for future monorepo-relative resolution
    safe = project_safe(raw)
    paths = instance_paths(
        run_dir, safe.instance_id, agent_id=agent_id, home_rel=home_rel
    )
    existing = read_status(paths)
    if should_skip_instance(existing, resume=resume):
        return existing or {"status": STATUS_OK, "instance_id": safe.instance_id, "skipped": True}

    prepare_instance_dirs(paths)
    container = None
    image_digest = ""
    started = time.time()
    try:
        client = docker_from_env()
        image = instance_image_key(raw)
        image_digest = pull_instance_image(client, image)
        container = start_task_container(
            client, image=image, run_id=run_id, instance_id=safe.instance_id
        )
        python_exe = resolve_python()
        mcp_script = WORKFLOW_DIR / "docker_workspace_mcp.py"
        base_text = base_config.read_text(encoding="utf-8")
        if not provider_api_key_available(base_text):
            raise RuntimeError(
                "provider API key missing: set the env named by provider.api_key_env "
                "(inline provider.api_key is rejected)"
            )

        ensure_agent_profile(
            agent_bin=agent_bin,
            project_root=paths.project_root,
            agent_id=paths.agent_id,
            template_dir=settings_dir,
        )

        config_text = render_run_config(
            base_config_text=base_text,
            mcp_script=mcp_script,
            container_id=container.id,
            log_dir=paths.log_dir,
            python_exe=python_exe,
        )
        write_text_atomic(paths.config_path, config_text)

        prompt = render_agent_prompt(safe)
        rc, stdout, stderr = run_agent(
            agent_bin=agent_bin,
            project_root=paths.project_root,
            agent_id=paths.agent_id,
            prompt=prompt,
            home_rel=paths.home_rel,
            run_id=f"{run_id}-{safe.instance_id}",
        )
        write_text_atomic(paths.stderr_path, stderr)
        agent_status, run_output = classify_agent_result(rc, stdout)
        if run_output is not None:
            write_json_atomic(paths.run_output_path, run_output)
        else:
            write_text_atomic(paths.run_output_path.with_suffix(".txt"), stdout)

        if agent_status != STATUS_OK:
            status = {
                "status": agent_status,
                "instance_id": safe.instance_id,
                "stop_reason": (run_output or {}).get("stop_reason"),
                "elapsed_sec": round(time.time() - started, 3),
                "image": image,
                "image_digest": image_digest,
            }
            write_json_atomic(paths.status_path, status)
            return status

        patch, patch_status = extract_model_patch(container)
        if patch:
            write_text_atomic(paths.patch_path, patch)
        status = {
            "status": patch_status if patch_status != STATUS_OK else STATUS_OK,
            "instance_id": safe.instance_id,
            "stop_reason": (run_output or {}).get("stop_reason"),
            "patch_path": str(paths.patch_path) if patch_status == STATUS_OK else None,
            "patch_bytes": len(patch.encode("utf-8")) if patch else 0,
            "elapsed_sec": round(time.time() - started, 3),
            "image": image,
            "image_digest": image_digest,
            "model_name_or_path": model_name_or_path,
        }
        if run_output and isinstance(run_output.get("usage"), dict):
            status["usage"] = run_output["usage"]
        write_json_atomic(paths.status_path, status)
        write_json_atomic(
            paths.provenance_path,
            {
                "instance_id": safe.instance_id,
                "repo": safe.repo,
                "base_commit": safe.base_commit,
                "image": image,
                "image_digest": image_digest,
                "generated_at": utc_now(),
                "agent_bin": str(agent_bin),
                "config_sha256": file_sha256(paths.config_path),
            },
        )
        return status
    except Exception as exc:
        status = {
            "status": STATUS_INFRA_ERROR,
            "instance_id": safe.instance_id,
            "error": f"{type(exc).__name__}: {exc}",
            "elapsed_sec": round(time.time() - started, 3),
        }
        write_json_atomic(paths.status_path, status)
        return status
    finally:
        remove_container(container)


def generate_batch(
    *,
    rows: Sequence[dict[str, Any]],
    run_dir: Path,
    run_id: str,
    repo_root: Path,
    base_config: Path,
    settings_dir: Path,
    agent_bin: Path,
    model_name_or_path: str,
    workers: int,
    resume: bool,
    agent_id: str = DEFAULT_AGENT_ID,
    home_rel: str = DEFAULT_HOME_REL,
) -> list[dict[str, Any]]:
    run_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []
    workers = max(1, int(workers))

    def _job(raw: dict[str, Any]) -> dict[str, Any]:
        return generate_one_instance(
            raw=raw,
            run_dir=run_dir,
            run_id=run_id,
            repo_root=repo_root,
            base_config=base_config,
            settings_dir=settings_dir,
            agent_bin=agent_bin,
            model_name_or_path=model_name_or_path,
            resume=resume,
            agent_id=agent_id,
            home_rel=home_rel,
        )

    if workers == 1:
        for raw in rows:
            results.append(_job(raw))
    else:
        with ThreadPoolExecutor(max_workers=workers) as pool:
            futs = {pool.submit(_job, raw): raw for raw in rows}
            for fut in as_completed(futs):
                results.append(fut.result())
    # Stable order by instance_id
    results.sort(key=lambda r: str(r.get("instance_id") or ""))
    reduce_predictions(run_dir, model_name_or_path=model_name_or_path)
    return results


def run_official_grade(
    *,
    predictions_path: Path | str,
    run_id: str,
    max_workers: int,
    dataset_name: str = DATASET_NAME,
    split: str = DATASET_SPLIT,
    instance_ids: Optional[Sequence[str]] = None,
    cwd: Optional[Path] = None,
) -> subprocess.CompletedProcess[str]:
    pred = (
        predictions_path
        if isinstance(predictions_path, str)
        else str(predictions_path)
    )
    cmd = [
        str(resolve_python()),
        "-m",
        "swebench.harness.run_evaluation",
        "--dataset_name",
        dataset_name,
        "--split",
        split,
        "--predictions_path",
        pred,
        "--max_workers",
        str(max_workers),
        "--run_id",
        run_id,
    ]
    if instance_ids:
        cmd.append("--instance_ids")
        cmd.extend(list(instance_ids))
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )


def link_harness_logs(run_dir: Path, run_id: str, model_name_or_path: str) -> Optional[Path]:
    """Copy/link upstream harness logs into the run directory when present."""
    model_dir = model_name_or_path.replace("/", "__")
    candidates = [
        Path("logs") / "run_evaluation" / run_id / model_dir,
        Path("evaluation_results") / run_id,
    ]
    dest = run_dir / "harness"
    dest.mkdir(parents=True, exist_ok=True)
    linked = None
    for src in candidates:
        if src.exists():
            target = dest / src.name
            if target.exists():
                continue
            try:
                if src.is_dir():
                    shutil.copytree(src, target, dirs_exist_ok=True)
                else:
                    shutil.copy2(src, target)
                linked = target
            except OSError:
                continue
    return linked


def collect_usage_aggregates(statuses: Sequence[dict[str, Any]]) -> dict[str, Any]:
    total_tokens = 0
    total_prompt = 0
    total_completion = 0
    total_iterations = 0
    total_elapsed_ms = 0
    cost_statuses: dict[str, int] = {}
    known_amounts: list[str] = []

    for st in statuses:
        usage = st.get("usage") or {}
        if not isinstance(usage, dict):
            continue
        total_tokens += int(usage.get("total_tokens") or 0)
        total_prompt += int(usage.get("prompt_tokens") or 0)
        total_completion += int(usage.get("completion_tokens") or 0)
        total_iterations += int(usage.get("iterations") or 0)
        total_elapsed_ms += int(usage.get("elapsed_ms") or 0)
        cost = usage.get("cost") or {}
        if isinstance(cost, dict):
            cs = str(cost.get("status") or "unavailable")
            cost_statuses[cs] = cost_statuses.get(cs, 0) + 1
            known = cost.get("known_total")
            if isinstance(known, dict) and known.get("amount") is not None:
                known_amounts.append(str(known["amount"]))

    return {
        "iterations": total_iterations,
        "prompt_tokens": total_prompt,
        "completion_tokens": total_completion,
        "total_tokens": total_tokens,
        "elapsed_ms": total_elapsed_ms,
        "cost_status_counts": cost_statuses,
        "known_cost_amounts": known_amounts,
        "note": "missing prices are never treated as zero; see cost_status_counts",
    }


def load_harness_resolved(run_dir: Path, run_id: str) -> dict[str, bool]:
    """Best-effort parse of upstream resolved flags from harness outputs."""
    resolved: dict[str, bool] = {}
    harness_root = run_dir / "harness"
    if not harness_root.exists():
        # Also scan CWD logs
        for path in Path(".").rglob("report.json"):
            if run_id not in str(path):
                continue
            try:
                data = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                continue
            if isinstance(data, dict):
                for iid, meta in data.items():
                    if isinstance(meta, dict) and "resolved" in meta:
                        resolved[str(iid)] = bool(meta["resolved"])
        return resolved

    for path in harness_root.rglob("report.json"):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(data, dict):
            for iid, meta in data.items():
                if isinstance(meta, dict) and "resolved" in meta:
                    resolved[str(iid)] = bool(meta["resolved"])
    return resolved


def build_report(
    *,
    run_dir: Path,
    run_id: str,
    selected_ids: Sequence[str],
    statuses: Sequence[dict[str, Any]],
) -> dict[str, Any]:
    by_status: dict[str, int] = {}
    for st in statuses:
        key = str(st.get("status") or "unknown")
        by_status[key] = by_status.get(key, 0) + 1

    resolved_map = load_harness_resolved(run_dir, run_id)
    resolved_count = sum(1 for v in resolved_map.values() if v)
    graded = len(resolved_map)
    generated = by_status.get(STATUS_OK, 0)

    per_instance = []
    for st in sorted(statuses, key=lambda s: str(s.get("instance_id") or "")):
        iid = str(st.get("instance_id") or "")
        per_instance.append(
            {
                "instance_id": iid,
                "status": st.get("status"),
                "stop_reason": st.get("stop_reason"),
                "patch_path": st.get("patch_path"),
                "harness_resolved": resolved_map.get(iid),
                "elapsed_sec": st.get("elapsed_sec"),
                "usage": st.get("usage"),
                "error": st.get("error"),
            }
        )

    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "total_selected": len(selected_ids),
        "generated_patches": generated,
        "empty_patches": by_status.get(STATUS_EMPTY_PATCH, 0),
        "agent_errors": by_status.get(STATUS_AGENT_ERROR, 0),
        "infrastructure_errors": by_status.get(STATUS_INFRA_ERROR, 0),
        "invalid_patches": by_status.get(STATUS_INVALID_PATCH, 0),
        "graded": graded,
        "resolved": resolved_count,
        "resolved_rate": (resolved_count / graded) if graded else None,
        "usage_aggregates": collect_usage_aggregates(statuses),
        "per_instance": per_instance,
        "status_counts": by_status,
        "generated_at": utc_now(),
    }


def build_manifest(
    *,
    run_id: str,
    repo_root: Path,
    settings_dir: Path,
    base_config: Path,
    agent_bin: Path,
    cli_args: dict[str, Any],
    dataset_info: dict[str, Any],
    image_digests: dict[str, str],
) -> dict[str, Any]:
    hashes = {}
    for name in ("master_prompt.md", "skills.dsl", "rules.md"):
        path = settings_dir / name
        if path.is_file():
            hashes[name] = file_sha256(path)
    if base_config.is_file():
        hashes["base_config"] = file_sha256(base_config)

    worker_meta = {
        "path": str(agent_bin),
        "sha256": file_sha256(agent_bin) if agent_bin.is_file() else None,
    }
    # Best-effort git commit for worker/repo
    commit = None
    try:
        proc = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=str(repo_root),
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode == 0:
            commit = proc.stdout.strip()
    except OSError:
        commit = None

    return {
        "schema_version": SCHEMA_VERSION,
        "workflow": "swebench-verified",
        "run_id": run_id,
        "timestamps": {"created_at": utc_now()},
        "host": host_label(),
        "dataset": dataset_info,
        "swebench_version": swebench_version(),
        "worker": worker_meta,
        "repo_commit": commit,
        "solver_profile_hashes": hashes,
        "cli_args": cli_args,
        "docker_image_digests": image_digests,
        "model": {
            "from_config": yaml_scalar(
                base_config.read_text(encoding="utf-8") if base_config.is_file() else "",
                "model",
                "",
            ),
            "config_sha256": hashes.get("base_config"),
        },
    }
