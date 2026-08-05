#!/usr/bin/env python3
"""Offline smoke checks for aoc-live helpers (no network, no agent)."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

WORKFLOW_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(WORKFLOW_DIR))

from aoc_http import (  # noqa: E402
    SubmitVerdict,
    classify_submit_html,
)
from runtime import prepare_runtime, write_report  # noqa: E402
from aoc_http import PuzzlePage  # noqa: E402


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
    repo = WORKFLOW_DIR.parent.parent
    base = repo / "test-agents" / "referent" / "agent-config.aoc.example.yaml"
    if not base.is_file():
        print("SKIP prepare_runtime (missing base config)")
        return
    puzzle = PuzzlePage(
        year=2024,
        day=1,
        title="Day 1",
        url="https://adventofcode.com/2024/day/1",
        text="sample text",
        raw_html="<html></html>",
    )
    with tempfile.TemporaryDirectory() as tmp:
        runs = Path(tmp)
        paths = prepare_runtime(
            repo_root=repo,
            runs_root=runs,
            base_config=base,
            puzzle=puzzle,
            puzzle_input="1 2\n",
            part=1,
            run_id="smoke",
        )
        assert paths.task_id == "2024-01-1"
        assert (paths.bank_dir / "2024-01-1.json").is_file()
        assert (paths.home / "input.txt").read_text(encoding="utf-8") == "1 2\n"
        assert paths.config_path.is_file()
        write_report(paths.run_dir / "report.json", {"status": "smoke", "attempts": []})
        assert (paths.run_dir / "report.json").is_file()


def main() -> int:
    test_classify()
    test_prepare_runtime_writes_bank()
    print("OK: offline smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
