#!/usr/bin/env python3
"""Offline smoke checks for aoc-live helpers (no network, no agent)."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

WORKFLOW_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(WORKFLOW_DIR))

from aoc_http import (  # noqa: E402
    PuzzlePage,
    SubmitVerdict,
    classify_submit_html,
)
from runtime import (  # noqa: E402
    prepare_runtime,
    resolve_home_abs,
    resolve_mcp_js,
    write_report,
)


def test_classify() -> None:
    cases = [
        ("<article><p>That's the right answer! ...</p></article>", SubmitVerdict.CORRECT),
        (
            "<article><p>That's not the right answer; your answer is too high.</p></article>",
            SubmitVerdict.WRONG,
        ),
        (
            "<article><p>You gave an answer too recently; please wait 1 minute.</p></article>",
            SubmitVerdict.TOO_RECENT,
        ),
        (
            "<article><p>Did you already complete it?</p></article>",
            SubmitVerdict.ALREADY_SOLVED,
        ),
        (
            "<article><p>You don't seem to be solving the right level.</p></article>",
            SubmitVerdict.WRONG_LEVEL,
        ),
        ("<article><p>To play, please identify yourself</p></article>", SubmitVerdict.AUTH_REQUIRED),
    ]
    for html, expected in cases:
        got = classify_submit_html(html)
        assert got.verdict == expected, (html, got)
        if expected == SubmitVerdict.WRONG:
            assert got.hint == "too high"
        if expected == SubmitVerdict.TOO_RECENT:
            assert got.wait_seconds == 60


def test_prepare_runtime_writes_bank() -> None:
    base = WORKFLOW_DIR / "profile" / "agent-config.example.yaml"
    mcp = resolve_mcp_js(workflow_dir=WORKFLOW_DIR)
    assert mcp.is_file(), f"bundled mcp-aoc-tasks.js missing: {mcp}"
    assert base.is_file(), f"bundled profile config missing: {base}"
    puzzle = PuzzlePage(
        year=2024,
        day=1,
        title="Day 1",
        url="https://adventofcode.com/2024/day/1",
        text="sample text",
        raw_html="<html></html>",
    )
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        runs = root / "runs"
        project = root / "project"
        agent_id = "aoc-live"
        home_rel = "homes/smoke"
        # Simulate protected profile dir so prepare can write agent-config.yaml.
        profile = project / ".kuibysheff" / "protected" / "agents" / agent_id
        profile.mkdir(parents=True)
        paths = prepare_runtime(
            repo_root=WORKFLOW_DIR,
            project_root=project,
            agent_id=agent_id,
            home_rel=home_rel,
            runs_root=runs,
            base_config=base,
            puzzle=puzzle,
            puzzle_input="1 2\n",
            part=1,
            run_id="smoke",
        )
        assert paths.task_id == "2024-01-1"
        assert (paths.bank_dir / "2024-01-1.json").is_file()
        assert paths.home == resolve_home_abs(project, home_rel)
        assert (paths.home / "input.txt").read_text(encoding="utf-8") == "1 2\n"
        assert paths.config_path.is_file()
        assert "protected" in str(paths.config_path).replace("\\", "/")
        cfg = paths.config_path.read_text(encoding="utf-8")
        assert "mcp-aoc-tasks.js" in cfg
        assert "api_key:" not in cfg or "api_key_env:" in cfg
        assert "\n  api_key:" not in cfg
        write_report(paths.run_dir / "report.json", {"status": "smoke", "attempts": []})
        assert (paths.run_dir / "report.json").is_file()


def main() -> int:
    test_classify()
    test_prepare_runtime_writes_bank()
    print("OK: offline smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
