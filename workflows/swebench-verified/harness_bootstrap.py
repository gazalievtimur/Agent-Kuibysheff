#!/usr/bin/env python3
"""Bootstrap official swebench harness with Unix (LF) text writes.

On Windows, ``Path.write_text`` defaults to CRLF. The harness writes ``eval.sh``
(and prediction patches) that way, then copies them into Linux containers where
bash sees ``pipefail\\r`` and fails. This entrypoint forces LF before importing
``swebench.harness.run_evaluation``.

Usage (same CLI as the upstream module)::

    python harness_bootstrap.py --dataset_name ... --predictions_path ...
"""

from __future__ import annotations

import pathlib
import runpy
import sys
from pathlib import Path

WORKFLOW_DIR = Path(__file__).resolve().parent


def _prefer_installed_swebench() -> None:
    """Drop this workflow dir so ``import swebench`` hits the pip package."""
    here = WORKFLOW_DIR.resolve()
    stubs = here / "win_stubs"
    filtered = [p for p in sys.path if not p or Path(p).resolve() != here]
    if stubs.is_dir() and str(stubs) not in filtered:
        filtered.insert(0, str(stubs))
    sys.path[:] = filtered
    for key in list(sys.modules):
        if key != "swebench" and not key.startswith("swebench."):
            continue
        mod = sys.modules.get(key)
        origin = getattr(mod, "__file__", None) if mod is not None else None
        if origin and Path(origin).resolve().parent == here:
            del sys.modules[key]


def _install_lf_path_writes() -> None:
    original = pathlib.Path.write_text

    def write_text_lf(
        self: Path,
        data: str,
        encoding: str | None = None,
        errors: str | None = None,
        newline: str | None = "\n",
    ):
        # Default newline to LF so shell scripts and unified diffs stay POSIX.
        if encoding is None:
            encoding = "utf-8"
        return original(self, data, encoding=encoding, errors=errors, newline=newline)

    pathlib.Path.write_text = write_text_lf  # type: ignore[method-assign]


def _install_lf_copy_to_container() -> None:
    """Defense in depth: strip CR when copying host files into the task container."""
    try:
        import swebench.harness.docker_utils as docker_utils
    except ImportError:
        return

    original = docker_utils.copy_to_container

    def copy_to_container_lf(container, src, dst):
        src_path = Path(src)
        raw = src_path.read_bytes()
        if b"\r" not in raw:
            return original(container, src, dst)
        fixed = src_path.with_name(src_path.name + ".lf")
        fixed.write_bytes(raw.replace(b"\r\n", b"\n").replace(b"\r", b"\n"))
        try:
            return original(container, fixed, dst)
        finally:
            fixed.unlink(missing_ok=True)

    docker_utils.copy_to_container = copy_to_container_lf


def main() -> None:
    _prefer_installed_swebench()
    _install_lf_path_writes()
    # Import after Path monkeypatch so any module-level helpers see LF writes.
    _install_lf_copy_to_container()
    sys.argv[0] = "swebench.harness.run_evaluation"
    runpy.run_module(
        "swebench.harness.run_evaluation",
        run_name="__main__",
        alter_sys=True,
    )


if __name__ == "__main__":
    main()
