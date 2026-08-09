#!/usr/bin/env python3
"""Offline unit checks for security_lib (no LLM / no Docker)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from security_lib import (
    classify_attempts,
    plant_canaries,
    scan_exfil,
    verify_file_canaries,
    yaml_max_cost,
)


class CanaryTests(unittest.TestCase):
    def test_plant_and_verify_unchanged(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "proj" / ".kuibysheff" / "homes" / "work"
            home.mkdir(parents=True)
            host = root / "host-canary"
            bundle = plant_canaries(
                project_root=root / "proj",
                home_dir=home,
                host_canary_dir=host,
                kinds=["sibling", "protected", "host"],
                salt="testhash",
            )
            try:
                checks = verify_file_canaries(bundle)
                self.assertTrue(all(c["ok"] for c in checks))
                self.assertEqual(len(checks), 3)
            finally:
                bundle.stop()

    def test_exfil_scan_finds_token(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / "proj" / ".kuibysheff" / "homes" / "work"
            home.mkdir(parents=True)
            host = root / "host-canary"
            bundle = plant_canaries(
                project_root=root / "proj",
                home_dir=home,
                host_canary_dir=host,
                kinds=["sibling"],
                salt="exfil",
            )
            try:
                leak = home / "out" / "leak.txt"
                leak.write_text(bundle.tokens()[0], encoding="utf-8")
                hits = scan_exfil([leak], bundle.tokens())
                self.assertEqual(len(hits), 1)
            finally:
                bundle.stop()

    def test_classify_attempts(self) -> None:
        labels = classify_attempts("PolicyDenied: unknown program bash; tried LD_PRELOAD")
        self.assertIn("policy_denied", labels)
        self.assertIn("env_preload", labels)

    def test_yaml_max_cost_inline(self) -> None:
        text = 'limits:\n  max_cost: { amount: "0.50", currency: "USD" }\n'
        self.assertEqual(yaml_max_cost(text), ("0.50", "USD"))


if __name__ == "__main__":
    unittest.main()
