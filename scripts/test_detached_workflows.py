#!/usr/bin/env python3
"""Detached copy-unit smoke tests (offline; no network / Docker / live AoC)."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
WORKFLOWS = REPO / "workflows"


def _copy_tree(src: Path, dst: Path) -> None:
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(
        src,
        dst,
        ignore=shutil.ignore_patterns(
            "runs",
            "__pycache__",
            "*.pyc",
            ".pytest_cache",
        ),
    )


def test_1c_detached_scaffold() -> None:
    src = REPO / "workflows" / "1c-dev"
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        dest = root / "1c-dev"
        elsewhere = root / "elsewhere"
        elsewhere.mkdir()
        project = root / "product"
        project.mkdir()
        # Example configs grant workspace.root → ../../../src/cf relative to profile.
        (project / "src" / "cf").mkdir(parents=True)
        _copy_tree(src, dest)
        ps = shutil.which("pwsh") or shutil.which("powershell")
        if not ps:
            print("skip: 1c scaffold requires PowerShell")
            return
        proc = subprocess.run(
            [
                ps,
                "-NoProfile",
                "-File",
                str(dest / "scaffold-project.ps1"),
                "-ProjectRoot",
                str(project),
                "-Force",
            ],
            cwd=str(elsewhere),
            capture_output=True,
            text=True,
            check=False,
        )
        assert proc.returncode == 0, proc.stdout + proc.stderr
        assert (
            project / ".kuibysheff" / "protected" / "agents" / "1c-analyst"
        ).is_dir()
        assert (project / ".kuibysheff" / "product.yaml").is_file()


def test_security_scan_exfil_offline() -> None:
    src = REPO / "workflows" / "security-sandbox"
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        dest = root / "security-sandbox"
        elsewhere = root / "elsewhere"
        elsewhere.mkdir()
        _copy_tree(src, dest)
        env = os.environ.copy()
        env["PYTHONPATH"] = str(dest)
        proc = subprocess.run(
            [sys.executable, str(dest / "test_scan_exfil_offline.py")],
            cwd=str(elsewhere),
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        assert proc.returncode == 0, proc.stdout + proc.stderr
        assert "OK:" in proc.stdout


def main() -> int:
    if not WORKFLOWS.is_dir():
        print(
            "skip: workflows/ not present "
            "(gitignored copy-units; restore from git history for local testing)"
        )
        return 0
    test_1c_detached_scaffold()
    test_security_scan_exfil_offline()
    print("OK: detached workflow copy-unit smokes passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
