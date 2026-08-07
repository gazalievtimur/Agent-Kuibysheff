#!/usr/bin/env python3
"""Offline unit checks for scripts/coverage_ratchet.py."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from coverage_ratchet import changed_line_coverage, parse_lcov


class CoverageRatchetTests(unittest.TestCase):
    def test_parse_lcov_totals(self) -> None:
        raw = """\
TN:
SF:src/foo.rs
DA:1,1
DA:2,0
DA:3,5
end_of_record
SF:src/bar.rs
DA:10,0
end_of_record
"""
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "lcov.info"
            path.write_text(raw, encoding="utf-8")
            files, percent, hits, found = parse_lcov(path)
        self.assertEqual(found, 4)
        self.assertEqual(hits, 2)
        self.assertAlmostEqual(percent, 50.0)
        self.assertEqual(files["src/foo.rs"][2], 0)

    def test_changed_line_coverage(self) -> None:
        lcov = {
            "src/foo.rs": {1: 1, 2: 0, 3: 1},
            "/abs/src/bar.rs": {4: 1},
        }
        from collections import defaultdict

        changed = defaultdict(set)
        changed["src/foo.rs"].update({1, 2, 99})  # 99 not executable
        changed["src/bar.rs"].update({4})
        percent, covered, total, details = changed_line_coverage(lcov, changed)
        self.assertEqual(total, 3)
        self.assertEqual(covered, 2)
        self.assertAlmostEqual(percent, 200.0 / 3.0)
        self.assertTrue(any("src/foo.rs:2" in d for d in details))


if __name__ == "__main__":
    unittest.main()
