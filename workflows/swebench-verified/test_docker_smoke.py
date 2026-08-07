#!/usr/bin/env python3
"""Optional Docker smoke for workspace MCP + patch extraction (manual/nightly).

Requires a local Linux Docker engine. Uses a small alpine/git image fixture —
not an official SWE-bench instance image.

Usage:
  python workflows/swebench-verified/test_docker_smoke.py
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

WORKFLOW_DIR = Path(__file__).resolve().parent
if str(WORKFLOW_DIR) not in sys.path:
    sys.path.insert(0, str(WORKFLOW_DIR))

from docker_workspace_mcp import (  # noqa: E402
    ENV_CONTAINER_ID,
    resolve_testbed_path,
    tool_exec,
    tool_read_file,
    tool_write_file,
)
from runtime import (  # noqa: E402
    STATUS_OK,
    check_docker_linux,
    docker_from_env,
    extract_model_patch,
    remove_container,
)


FIXTURE_IMAGE = "alpine/git:2.45.2"
SETUP_SCRIPT = r"""
set -euo pipefail
mkdir -p /testbed
cd /testbed
git init
git config user.email smoke@example.com
git config user.name smoke
echo 'hello' > README.md
git add README.md
git commit -m 'init'
"""


def main() -> int:
    print("== docker smoke: engine ==")
    info = check_docker_linux()
    print(info)

    client = docker_from_env()
    print(f"== pull {FIXTURE_IMAGE} ==")
    try:
        client.images.get(FIXTURE_IMAGE)
    except Exception:
        client.images.pull(FIXTURE_IMAGE)

    container = None
    try:
        container = client.containers.run(
            FIXTURE_IMAGE,
            command=["sleep", "infinity"],
            detach=True,
            network_disabled=True,
            working_dir="/",
            labels={"swebench.workflow": "docker-smoke"},
        )
        # alpine/git may lack bash/python — install minimal tools
        # Prefer busybox ash + install python3 if apk available.
        code, out = container.exec_run(
            ["sh", "-lc", "command -v python3 || (apk add --no-cache python3 bash >/dev/null && command -v python3)"],
        )
        if code != 0:
            print("SKIP: could not install python3 in fixture image")
            return 0

        code, out = container.exec_run(["sh", "-lc", SETUP_SCRIPT])
        if code != 0:
            raise RuntimeError(f"fixture setup failed: {out}")

        import os

        os.environ[ENV_CONTAINER_ID] = container.id

        # Path guards (local, no docker)
        assert str(resolve_testbed_path("README.md")) == "/testbed/README.md"

        before = tool_read_file("README.md")
        assert "hello" in before
        tool_write_file("README.md", "hello\nworld\n")
        tool_write_file("new_file.txt", "created\n")
        exec_out = tool_exec("test -f new_file.txt && echo OK")
        assert "OK" in exec_out

        patch, status = extract_model_patch(container)
        assert status == STATUS_OK, status
        assert "new_file.txt" in patch or "diff --git" in patch
        print("patch excerpt:\n", patch[:400])
        print("OK: docker smoke passed")
        return 0
    finally:
        remove_container(container)
        # Ensure removal
        if container is not None:
            time.sleep(0.2)
            try:
                docker_from_env().containers.get(container.id)
                print("WARN: container still present after remove")
            except Exception:
                print("container removed")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"FAIL: {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
