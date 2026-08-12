#!/usr/bin/env python3
"""Offline unit checks for Scale-FS corpus planting (no LLM)."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from corpus import (  # noqa: E402
    HOME_READ_DEFAULT_CHARS,
    OVERSIZE_MIN_BYTES,
    make_needle,
    plant_from_task,
    plant_large_read,
    plant_many_files,
    plant_oversize,
    verify_planted,
)


class CorpusTests(unittest.TestCase):
    def test_needle_stable_for_seed(self) -> None:
        a = make_needle("search-many-01", 42)
        b = make_needle("search-many-01", 42)
        c = make_needle("search-many-01", 43)
        self.assertEqual(a, b)
        self.assertNotEqual(a, c)
        self.assertTrue(a.startswith("SF_NEEDLE_"))

    def test_many_files_plant(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            planted = plant_many_files(
                workspace_root=root,
                task_id="search-many-01",
                file_count=40,
                seed=42,
            )
            verify_planted(planted, workspace_root=root, home_dir=root)
            self.assertEqual(planted.meta["file_count"], 40)
            needle_path = root / str(planted.meta["needle_file"])
            self.assertIn(planted.needle, needle_path.read_text(encoding="utf-8"))

    def test_large_read_past_default_window(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            planted = plant_large_read(
                home_dir=home,
                task_id="large-read-01",
                total_chars=100_000,
                seed=7,
            )
            verify_planted(planted, workspace_root=home, home_dir=home)
            text = (home / planted.paths[0]).read_text(encoding="utf-8")
            self.assertGreater(len(text), HOME_READ_DEFAULT_CHARS)
            self.assertGreater(text.index(f"TRACE_ID={planted.needle}"), HOME_READ_DEFAULT_CHARS)

    def test_oversize_past_single_window(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            home = Path(tmp)
            planted = plant_oversize(
                home_dir=home,
                task_id="oversize-run-01",
                target_bytes=OVERSIZE_MIN_BYTES + 50_000,
                seed=99,
            )
            verify_planted(planted, workspace_root=home, home_dir=home)
            self.assertGreater(planted.meta["byte_count"], OVERSIZE_MIN_BYTES)
            text = (home / planted.paths[0]).read_text(encoding="utf-8")
            needle_at = text.index(f"SECRET_TOKEN={planted.needle}")
            self.assertGreater(needle_at, HOME_READ_DEFAULT_CHARS)

    def test_plant_from_task_dispatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "home"
            home.mkdir()
            task = {
                "id": "search-many-01",
                "corpus": {
                    "kind": "many_files",
                    "file_count": 20,
                    "seed": 3,
                    "needle_hint": "Project Orion",
                },
            }
            planted = plant_from_task(task, workspace_root=root, home_dir=home)
            verify_planted(planted, workspace_root=root, home_dir=home)
            self.assertEqual(planted.kind, "many_files")


if __name__ == "__main__":
    unittest.main()
