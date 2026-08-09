"""Minimal Unix `resource` stub for running swebench harness imports on Windows.

Official swebench 4.1.0 still imports `resource` from prepare_images via
harness/__init__.py. run_evaluation itself already guards setrlimit with
platform.system() == \"Linux\"; this stub only needs to satisfy the import.
"""

from __future__ import annotations


class error(Exception):
    """Placeholder matching resource.error."""


RLIMIT_NOFILE = 7
RLIMIT_CPU = 0
RLIMIT_AS = 9


def getrlimit(_resource: int) -> tuple[int, int]:
    return (512, 512)


def setrlimit(_resource: int, _limits: tuple[int, int]) -> None:
    return None
