#!/usr/bin/env python3
"""Fail-closed Docker workspace MCP for a single SWE-bench task container.

Container ID is taken only from SWEBENCH_CONTAINER_ID (set by the orchestrator).
Tool arguments cannot retarget the container. All paths resolve under /testbed.
"""

from __future__ import annotations

import base64
import os
import shlex
import sys
from pathlib import PurePosixPath
from typing import Any, Optional

TESTBED = PurePosixPath("/testbed")
DEFAULT_EXEC_TIMEOUT_SEC = 120
DEFAULT_MAX_OUTPUT_CHARS = 200_000
ENV_CONTAINER_ID = "SWEBENCH_CONTAINER_ID"
ENV_EXEC_TIMEOUT = "SWEBENCH_EXEC_TIMEOUT_SEC"
ENV_MAX_OUTPUT = "SWEBENCH_MAX_OUTPUT_CHARS"


class WorkspaceError(Exception):
    """User-visible MCP tool failure."""


def _env_int(name: str, default: int) -> int:
    raw = os.environ.get(name, "").strip()
    if not raw:
        return default
    return int(raw)


def get_container_id() -> str:
    cid = os.environ.get(ENV_CONTAINER_ID, "").strip()
    if not cid:
        raise WorkspaceError(
            f"{ENV_CONTAINER_ID} is not set; orchestrator must bind one container"
        )
    return cid


def resolve_testbed_path(rel: str) -> PurePosixPath:
    """Resolve a relative path under /testbed; reject absolute/traversal."""
    if rel is None:
        raise WorkspaceError("path is required")
    text = str(rel).strip()
    if not text:
        raise WorkspaceError("path must not be empty")
    if text.startswith("/") or text.startswith("\\"):
        raise WorkspaceError("absolute paths are forbidden")
    # Windows drive / UNC
    if len(text) >= 2 and text[1] == ":":
        raise WorkspaceError("absolute paths are forbidden")
    if text.startswith("//") or text.startswith("\\\\"):
        raise WorkspaceError("absolute paths are forbidden")

    parts = [
        p
        for p in PurePosixPath(text.replace("\\", "/")).parts
        if p not in ("", ".")
    ]
    for part in parts:
        if part == "..":
            raise WorkspaceError("path traversal is forbidden")
        if part.startswith("~"):
            raise WorkspaceError("home-relative paths are forbidden")

    normalized = TESTBED.joinpath(*parts) if parts else TESTBED
    if normalized != TESTBED and not str(normalized).startswith("/testbed/"):
        raise WorkspaceError("path escapes /testbed")
    return normalized

def _truncate(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    return text[:limit] + f"\n...[truncated {len(text) - limit} chars]"


def docker_client():
    import docker

    return docker.from_env()


def container_exec(
    *,
    cmd: list[str],
    workdir: str = "/testbed",
    timeout_sec: Optional[int] = None,
    max_output: Optional[int] = None,
    user: str = "",
) -> dict[str, Any]:
    """Run argv in the bound container. Returns exit_code/stdout/stderr."""
    cid = get_container_id()
    timeout = timeout_sec if timeout_sec is not None else _env_int(ENV_EXEC_TIMEOUT, DEFAULT_EXEC_TIMEOUT_SEC)
    limit = max_output if max_output is not None else _env_int(ENV_MAX_OUTPUT, DEFAULT_MAX_OUTPUT_CHARS)

    client = docker_client()
    try:
        container = client.containers.get(cid)
    except Exception as exc:  # pragma: no cover - docker dependent
        raise WorkspaceError(f"container not found: {cid}") from exc

    # docker-py exec_run has no native timeout on all versions; wrap with demux.
    api = client.api
    exec_id = api.exec_create(
        container.id,
        cmd,
        workdir=workdir,
        user=user or None,
        environment=None,
    )["Id"]
    output = api.exec_start(exec_id, demux=True)
    inspect = api.exec_inspect(exec_id)
    exit_code = int(inspect.get("ExitCode") if inspect.get("ExitCode") is not None else -1)
    stdout_b, stderr_b = output if isinstance(output, tuple) else (output, b"")
    stdout = (stdout_b or b"").decode("utf-8", errors="replace")
    stderr = (stderr_b or b"").decode("utf-8", errors="replace")
    # Soft note: docker-py lacks portable kill-on-timeout for exec; orchestrator
    # sets container-level stop. We still enforce output caps.
    _ = timeout  # reserved for future timeout wrapper
    return {
        "exit_code": exit_code,
        "stdout": _truncate(stdout, limit),
        "stderr": _truncate(stderr, limit),
    }


def _ensure_under_testbed_real(path: PurePosixPath) -> None:
    """Ask the container to confirm the resolved realpath stays under /testbed."""
    script = (
        "import os,sys\n"
        f"p={str(path)!r}\n"
        "real=os.path.realpath(p)\n"
        "if not (real=='/testbed' or real.startswith('/testbed/')):\n"
        "  sys.stderr.write('symlink escape: '+real)\\n; sys.exit(2)\n"
        "print(real)\n"
    )
    result = container_exec(
        cmd=["python", "-c", script],
        workdir="/testbed",
        timeout_sec=30,
    )
    if result["exit_code"] != 0:
        raise WorkspaceError(
            f"path rejected (symlink escape or missing): {path}: {result['stderr'] or result['stdout']}"
        )


def tool_read_file(path: str) -> str:
    target = resolve_testbed_path(path)
    _ensure_under_testbed_real(target)
    result = container_exec(
        cmd=["python", "-c", f"import pathlib; print(pathlib.Path({str(target)!r}).read_text(encoding='utf-8'), end='')"],
        workdir="/testbed",
    )
    if result["exit_code"] != 0:
        raise WorkspaceError(result["stderr"] or f"read failed: {target}")
    return result["stdout"]


def tool_write_file(path: str, content: str) -> str:
    target = resolve_testbed_path(path)
    # Parent must stay under testbed; create parents inside container.
    parent = target.parent
    if parent != TESTBED:
        try:
            parent.relative_to(TESTBED)
        except ValueError as exc:
            raise WorkspaceError("path escapes /testbed") from exc
    payload = base64.b64encode(content.encode("utf-8")).decode("ascii")
    script = (
        "import base64,os,pathlib\n"
        f"p=pathlib.Path({str(target)!r})\n"
        "p.parent.mkdir(parents=True, exist_ok=True)\n"
        "real=os.path.realpath(str(p.parent))\n"
        "if not (real=='/testbed' or real.startswith('/testbed/')):\n"
        "  raise SystemExit('symlink escape')\n"
        f"p.write_bytes(base64.b64decode({payload!r}))\n"
        "print('ok')\n"
    )
    result = container_exec(cmd=["python", "-c", script], workdir="/testbed")
    if result["exit_code"] != 0:
        raise WorkspaceError(result["stderr"] or f"write failed: {target}")
    return f"wrote {target}"


def tool_search(query: str, path: str = ".", glob: str = "") -> str:
    if not str(query).strip():
        raise WorkspaceError("query must not be empty")
    root = resolve_testbed_path(path)
    _ensure_under_testbed_real(root)
    # Prefer ripgrep; fall back to grep -R.
    rg = ["rg", "--line-number", "--no-heading", "--color", "never"]
    if glob:
        rg.extend(["--glob", glob])
    rg.extend(["--", query, str(root)])
    result = container_exec(cmd=rg, workdir="/testbed", timeout_sec=60)
    if result["exit_code"] in (0, 1):
        return result["stdout"]
    # Fallback
    grepcmd = ["grep", "-R", "-n", "--", query, str(root)]
    result2 = container_exec(cmd=grepcmd, workdir="/testbed", timeout_sec=60)
    if result2["exit_code"] in (0, 1):
        return result2["stdout"]
    raise WorkspaceError(result["stderr"] or result2["stderr"] or "search failed")


def tool_exec(command: str, timeout_ms: int = 120_000) -> str:
    if not str(command).strip():
        raise WorkspaceError("command must not be empty")
    # Run via bash -lc inside /testbed; network is disabled at container level.
    timeout_sec = max(1, int(timeout_ms) // 1000)
    result = container_exec(
        cmd=["bash", "-lc", command],
        workdir="/testbed",
        timeout_sec=timeout_sec,
    )
    return (
        f"exit_code={result['exit_code']}\n"
        f"stdout:\n{result['stdout']}\n"
        f"stderr:\n{result['stderr']}"
    )


def tool_git_diff(paths: str = "") -> str:
    cmd = ["git", "diff", "--binary", "--no-ext-diff"]
    if paths.strip():
        for part in shlex.split(paths):
            resolve_testbed_path(part)
        cmd.append("--")
        cmd.extend(shlex.split(paths))
    result = container_exec(cmd=cmd, workdir="/testbed")
    if result["exit_code"] not in (0, 1):
        raise WorkspaceError(result["stderr"] or "git diff failed")
    return result["stdout"]


def build_server():
    from mcp.server.fastmcp import FastMCP

    server = FastMCP("workspace")

    @server.tool(name="read_file")
    def read_file(path: str) -> str:
        """Read a UTF-8 file under /testbed (relative path only)."""
        return tool_read_file(path)

    @server.tool(name="write_file")
    def write_file(path: str, content: str) -> str:
        """Write a UTF-8 file under /testbed (relative path only)."""
        return tool_write_file(path, content)

    @server.tool(name="search")
    def search(query: str, path: str = ".", glob: str = "") -> str:
        """Search file contents under /testbed."""
        return tool_search(query, path=path, glob=glob)

    @server.tool(name="exec")
    def exec_cmd(command: str, timeout_ms: int = 120000) -> str:
        """Run a shell command with cwd=/testbed in the bound container."""
        return tool_exec(command, timeout_ms=timeout_ms)

    @server.tool(name="git_diff")
    def git_diff(paths: str = "") -> str:
        """Show git diff under /testbed (optional relative paths)."""
        return tool_git_diff(paths)

    return server


def main() -> None:
    # Fail closed early if container id missing.
    try:
        get_container_id()
    except WorkspaceError as exc:
        print(f"workspace MCP misconfigured: {exc}", file=sys.stderr)
        raise SystemExit(2) from exc
    server = build_server()
    server.run(transport="stdio")


if __name__ == "__main__":
    main()
