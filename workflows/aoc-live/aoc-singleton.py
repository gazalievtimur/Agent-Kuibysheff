#!/usr/bin/env python3
"""Live Advent of Code solver orchestrator over Kuibyshev ACP stdio.

Workflow example: download puzzle + input from adventofcode.com, drive a
long-lived `agent_Kuibyshev acp` child through up to 5 solve/submit iterations,
and retry with AoC feedback when the answer is wrong.

Usage (from repo root):

  python workflows/aoc-live/aoc-singleton.py --year 2024 --day 1 --part 1

Or via launchers:

  .\\workflows\\aoc-live\\run.ps1 -Year 2024 -Day 1 -Part 1
  ./workflows/aoc-live/run.sh --year 2024 --day 1 --part 1
"""

from __future__ import annotations

import argparse
import asyncio
import logging
import os
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Optional

WORKFLOW_DIR = Path(__file__).resolve().parent
if str(WORKFLOW_DIR) not in sys.path:
    sys.path.insert(0, str(WORKFLOW_DIR))

from acp_bridge import AcpAgentSession  # noqa: E402
from aoc_http import AocHttpClient, SubmitResult, SubmitVerdict  # noqa: E402
from runtime import (  # noqa: E402
    load_dotenv,
    prepare_runtime,
    resolve_agent_binary,
    write_report,
)

MAX_ATTEMPTS_HARD_CAP = 5
DEFAULT_MAX_ATTEMPTS = 5
DEFAULT_CONFIG = "test-agents/referent/agent-config.aoc.example.yaml"
DEFAULT_SETTINGS = "test-agents/referent"

logger = logging.getLogger("aoc-live")


@dataclass
class AttemptRecord:
    attempt: int
    candidate: Optional[str]
    acp_stop_reason: Optional[str]
    verdict: Optional[str]
    message: Optional[str]
    hint: Optional[str]
    elapsed_ms: int
    error: Optional[str] = None


class SingletonLock:
    """Process-wide lock so only one aoc-live singleton runs at a time."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self._fh: Any = None

    def acquire(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self._fh = open(self.path, "a+", encoding="utf-8")
        try:
            if os.name == "nt":
                import msvcrt

                self._fh.seek(0)
                if self._fh.read(1) == "":
                    self._fh.write("0")
                    self._fh.flush()
                self._fh.seek(0)
                msvcrt.locking(self._fh.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl

                fcntl.flock(self._fh.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as err:
            raise RuntimeError(
                f"Another aoc-live singleton is already running (lock: {self.path})"
            ) from err
        self._fh.seek(0)
        self._fh.truncate()
        self._fh.write(f"{os.getpid()}\n")
        self._fh.flush()

    def release(self) -> None:
        if self._fh is None:
            return
        try:
            if os.name == "nt":
                import msvcrt

                self._fh.seek(0)
                msvcrt.locking(self._fh.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(self._fh.fileno(), fcntl.LOCK_UN)
        except OSError:
            pass
        try:
            self._fh.close()
        finally:
            self._fh = None
            try:
                self.path.unlink(missing_ok=True)
            except OSError:
                pass

    def __enter__(self) -> "SingletonLock":
        self.acquire()
        return self

    def __exit__(self, *args: Any) -> None:
        self.release()


def build_solve_prompt(task_id: str, part: int) -> str:
    return (
        f"Solve AoC task {task_id} (part {part}). "
        "Work one turn at a time: each reply must be exactly one JSON object "
        "(never multiple JSON objects). Do not pre-emit future turns. "
        "Steps across turns: "
        "1) Fetch statement with aoc_get_task and call aoc_get_input "
        "(writes/confirm home/input.txt; do not paste the full input into thoughts). "
        "input.txt is already present under home. "
        "2) Write solution.py that reads input.txt, then home.run with program=python. "
        "Debug until stdout shows the correct answer. "
        "3) Final response: done=true with result equal to only the final answer string. "
        "Do not guess. Return JSON only on every turn."
    )


def build_retry_prompt(
    task_id: str,
    part: int,
    previous_answer: str,
    submit: SubmitResult,
) -> str:
    hint = f" Hint from AoC: answer was {submit.hint}." if submit.hint else ""
    return (
        f"Solve AoC task {task_id} (part {part}) again. "
        f"Your previous final answer `{previous_answer}` was REJECTED by Advent of Code. "
        f"Server message: {submit.message}.{hint} "
        "Do not repeat the same wrong answer. "
        "Inspect/fix solution.py under home, re-run with home.run (program=python), "
        "and finish with done=true and result equal to only the new answer string. "
        "Each reply must be exactly one JSON object. Return JSON only on every turn."
    )


async def sleep_rate_limit(wait_seconds: Optional[int]) -> None:
    seconds = wait_seconds if wait_seconds and wait_seconds > 0 else 60
    seconds = min(seconds, 300)
    logger.warning("AoC rate limit: sleeping %ss before re-submit", seconds)
    await asyncio.sleep(seconds)


async def run_workflow(args: argparse.Namespace) -> int:
    repo_root = Path(args.repo_root).resolve()
    load_dotenv(repo_root / ".env")

    session_cookie = (os.environ.get("AOC_SESSION") or "").strip()
    if not session_cookie:
        raise SystemExit(
            "AOC_SESSION env var is required (AoC session cookie from browser)."
        )

    max_attempts = min(int(args.max_attempts), MAX_ATTEMPTS_HARD_CAP)
    if max_attempts < 1:
        raise SystemExit("--max-attempts must be >= 1")

    settings_dir = Path(args.settings_dir)
    if not settings_dir.is_absolute():
        settings_dir = (repo_root / settings_dir).resolve()
    base_config = Path(args.config)
    if not base_config.is_absolute():
        base_config = (repo_root / base_config).resolve()
    if not settings_dir.is_dir():
        raise SystemExit(f"settings dir not found: {settings_dir}")
    if not base_config.is_file():
        raise SystemExit(f"config not found: {base_config}")

    runs_root = Path(args.home_root) if args.home_root else (WORKFLOW_DIR / "runs")
    if not runs_root.is_absolute():
        runs_root = (repo_root / runs_root).resolve()
    runs_root.mkdir(parents=True, exist_ok=True)

    lock = SingletonLock(runs_root / ".aoc-singleton.lock")
    with lock:
        return await _run_locked(
            args=args,
            repo_root=repo_root,
            settings_dir=settings_dir,
            base_config=base_config,
            runs_root=runs_root,
            session_cookie=session_cookie,
            max_attempts=max_attempts,
        )


async def _run_locked(
    *,
    args: argparse.Namespace,
    repo_root: Path,
    settings_dir: Path,
    base_config: Path,
    runs_root: Path,
    session_cookie: str,
    max_attempts: int,
) -> int:
    year = int(args.year)
    day = int(args.day)
    part = int(args.part)
    if part not in (1, 2):
        raise SystemExit("--part must be 1 or 2")
    if day < 1 or day > 25:
        raise SystemExit("--day must be 1..25")

    run_id = args.run_id or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    aoc = AocHttpClient(session_cookie)

    logger.info("Fetching AoC puzzle %s-day-%s part %s", year, day, part)
    puzzle = aoc.fetch_puzzle(year, day, part)
    puzzle_input = aoc.fetch_input(year, day)

    paths = prepare_runtime(
        repo_root=repo_root,
        runs_root=runs_root,
        base_config=base_config,
        puzzle=puzzle,
        puzzle_input=puzzle_input,
        part=part,
        run_id=run_id,
    )
    logger.info("Runtime ready task_id=%s home=%s", paths.task_id, paths.home)

    agent_bin = resolve_agent_binary(
        repo_root, Path(args.agent_bin) if args.agent_bin else None
    )

    attempts: list[AttemptRecord] = []
    final_status = "failed"
    final_answer: Optional[str] = None

    async with AcpAgentSession(
        agent_bin=agent_bin,
        config=paths.config_path,
        settings_dir=settings_dir,
        home=paths.home,
        cwd=repo_root,
        save_chat_history=True,
    ) as session:
        previous_submit: Optional[SubmitResult] = None
        previous_answer = ""

        for attempt in range(1, max_attempts + 1):
            started = time.perf_counter()
            if attempt == 1 or previous_submit is None:
                prompt = build_solve_prompt(paths.task_id, part)
            else:
                prompt = build_retry_prompt(
                    paths.task_id, part, previous_answer, previous_submit
                )

            logger.info("=== attempt %s/%s: agent solve ===", attempt, max_attempts)
            try:
                outcome = await session.prompt(prompt)
            except Exception as err:  # noqa: BLE001
                elapsed_ms = int((time.perf_counter() - started) * 1000)
                attempts.append(
                    AttemptRecord(
                        attempt=attempt,
                        candidate=None,
                        acp_stop_reason=None,
                        verdict=None,
                        message=None,
                        hint=None,
                        elapsed_ms=elapsed_ms,
                        error=str(err),
                    )
                )
                logger.error("ACP prompt failed: %s", err)
                break

            candidate = outcome.answer.strip()
            if not candidate:
                elapsed_ms = int((time.perf_counter() - started) * 1000)
                attempts.append(
                    AttemptRecord(
                        attempt=attempt,
                        candidate="",
                        acp_stop_reason=outcome.stop_reason,
                        verdict=None,
                        message=None,
                        hint=None,
                        elapsed_ms=elapsed_ms,
                        error="agent returned empty answer",
                    )
                )
                logger.error(
                    "Empty answer from agent (stop_reason=%s)", outcome.stop_reason
                )
                # Still count as an iteration; continue to next attempt with feedback.
                previous_answer = ""
                previous_submit = SubmitResult(
                    verdict=SubmitVerdict.WRONG,
                    message="empty agent answer (not submitted)",
                    raw_html="",
                )
                continue

            logger.info(
                "Agent answer=%r stop_reason=%s — submitting to AoC",
                candidate,
                outcome.stop_reason,
            )

            submit = aoc.submit_answer(year, day, part, candidate)

            # Rate-limit: wait and re-submit the same candidate without burning
            # an extra agent iteration (still within this attempt slot).
            while submit.verdict == SubmitVerdict.TOO_RECENT:
                await sleep_rate_limit(submit.wait_seconds)
                submit = aoc.submit_answer(year, day, part, candidate)

            elapsed_ms = int((time.perf_counter() - started) * 1000)
            attempts.append(
                AttemptRecord(
                    attempt=attempt,
                    candidate=candidate,
                    acp_stop_reason=outcome.stop_reason,
                    verdict=submit.verdict.value,
                    message=submit.message,
                    hint=submit.hint,
                    elapsed_ms=elapsed_ms,
                )
            )
            (paths.run_dir / f"submit-attempt-{attempt}.html").write_text(
                submit.raw_html, encoding="utf-8"
            )

            if submit.verdict in (
                SubmitVerdict.CORRECT,
                SubmitVerdict.ALREADY_SOLVED,
            ):
                final_status = submit.verdict.value
                final_answer = candidate
                logger.info("SUCCESS (%s): %s", final_status, candidate)
                break

            if submit.verdict == SubmitVerdict.AUTH_REQUIRED:
                final_status = "auth_required"
                logger.error("AoC auth failed — check AOC_SESSION")
                break

            if submit.verdict == SubmitVerdict.WRONG_LEVEL:
                final_status = "wrong_level"
                logger.error("Wrong AoC level for part=%s", part)
                break

            if submit.verdict == SubmitVerdict.UNKNOWN:
                final_status = "unknown_submit"
                logger.error("Unrecognized AoC submit response: %s", submit.message)
                break

            # Wrong answer — optional cooldown before next agent attempt.
            if submit.wait_seconds:
                await sleep_rate_limit(submit.wait_seconds)

            previous_answer = candidate
            previous_submit = submit
            logger.warning(
                "Wrong answer on attempt %s (%s). Remaining attempts: %s",
                attempt,
                submit.hint or "no hint",
                max_attempts - attempt,
            )
        else:
            final_status = "max_attempts_exhausted"

    report = {
        "workflow": "aoc-live",
        "run_id": run_id,
        "year": year,
        "day": day,
        "part": part,
        "task_id": paths.task_id,
        "status": final_status,
        "final_answer": final_answer,
        "max_attempts": max_attempts,
        "attempts": [asdict(a) for a in attempts],
        "home": str(paths.home),
        "log_dir": str(paths.log_dir),
        "config": str(paths.config_path),
        "settings": str(settings_dir),
        "puzzle_url": puzzle.url,
    }
    report_path = paths.run_dir / "report.json"
    write_report(report_path, report)
    logger.info("Report: %s", report_path)
    logger.info("Status: %s attempts=%s", final_status, len(attempts))

    if final_status in ("correct", "already_solved"):
        return 0
    return 1


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    repo_guess = WORKFLOW_DIR.parent.parent
    parser = argparse.ArgumentParser(
        description="Live AoC singleton orchestrator over agent_Kuibyshev ACP"
    )
    parser.add_argument("--year", type=int, required=True)
    parser.add_argument("--day", type=int, required=True)
    parser.add_argument("--part", type=int, default=1)
    parser.add_argument(
        "--max-attempts",
        type=int,
        default=DEFAULT_MAX_ATTEMPTS,
        help=f"Full solve/submit iterations (default {DEFAULT_MAX_ATTEMPTS}, hard cap {MAX_ATTEMPTS_HARD_CAP})",
    )
    parser.add_argument(
        "--config",
        default=DEFAULT_CONFIG,
        help="Base agent config (provider settings template)",
    )
    parser.add_argument(
        "--settings-dir",
        default=DEFAULT_SETTINGS,
        help="Agent settings directory (master_prompt / skills / rules)",
    )
    parser.add_argument(
        "--home-root",
        default="",
        help="Root for run artifacts (default: workflows/aoc-live/runs)",
    )
    parser.add_argument("--run-id", default="", help="Optional run id (default: UTC timestamp)")
    parser.add_argument(
        "--repo-root",
        default=str(repo_guess),
        help="Repository root (for binary, mcp-aoc-tasks.js, .env)",
    )
    parser.add_argument("--agent-bin", default="", help="Path to agent_Kuibyshev binary")
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Debug logging (includes drained ACP stderr)",
    )
    return parser.parse_args(argv)


def main(argv: Optional[list[str]] = None) -> int:
    args = parse_args(argv)
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )
    try:
        return asyncio.run(run_workflow(args))
    except KeyboardInterrupt:
        logger.error("Interrupted")
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
