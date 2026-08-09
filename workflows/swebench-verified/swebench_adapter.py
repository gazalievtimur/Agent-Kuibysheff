"""Pinned SWE-bench Verified dataset adapter and safe projection."""

from __future__ import annotations

import hashlib
import importlib.metadata
import sys
from contextlib import contextmanager
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Iterable, Iterator, Optional, Sequence

_ADAPTER_DIR = Path(__file__).resolve().parent

DATASET_NAME = "SWE-bench/SWE-bench_Verified"
DATASET_SPLIT = "test"
GOLD_SMOKE_INSTANCE_ID = "sympy__sympy-20590"

# Fields that must never reach the agent prompt, MCP, or generation container.
ORACLE_FIELDS = frozenset(
    {
        "patch",
        "test_patch",
        "FAIL_TO_PASS",
        "PASS_TO_PASS",
        "hints_text",
    }
)

SAFE_FIELDS = ("instance_id", "repo", "base_commit", "problem_statement")


@dataclass(frozen=True)
class SafeInstance:
    instance_id: str
    repo: str
    base_commit: str
    problem_statement: str

    def to_dict(self) -> dict[str, str]:
        return asdict(self)


@contextmanager
def _installed_swebench_import_path() -> Iterator[None]:
    """Prefer the pip package over the local workflow CLI file ``swebench.py``."""
    here = _ADAPTER_DIR.resolve()
    stubs = here / "win_stubs"
    saved_path = sys.path[:]
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
    try:
        yield
    finally:
        sys.path[:] = saved_path


def swebench_version() -> str:
    try:
        return importlib.metadata.version("swebench")
    except importlib.metadata.PackageNotFoundError:
        return "unavailable"


def project_safe(raw: dict[str, Any]) -> SafeInstance:
    """Project a dataset row to the oracle-free generation view."""
    missing = [k for k in SAFE_FIELDS if not str(raw.get(k) or "").strip()]
    if missing:
        raise ValueError(f"instance missing required safe fields: {missing}")
    return SafeInstance(
        instance_id=str(raw["instance_id"]).strip(),
        repo=str(raw["repo"]).strip(),
        base_commit=str(raw["base_commit"]).strip(),
        problem_statement=str(raw["problem_statement"]),
    )


def assert_no_oracle_leak(payload: dict[str, Any]) -> None:
    leaked = sorted(set(payload) & ORACLE_FIELDS)
    if leaked:
        raise ValueError(f"oracle fields must not be projected: {leaked}")


def fingerprint_rows(rows: Sequence[dict[str, Any]]) -> str:
    """Stable fingerprint over selected instance_ids + base_commits."""
    h = hashlib.sha256()
    for row in sorted(rows, key=lambda r: str(r.get("instance_id") or "")):
        iid = str(row.get("instance_id") or "")
        commit = str(row.get("base_commit") or "")
        h.update(f"{iid}\0{commit}\n".encode("utf-8"))
    return h.hexdigest()


def parse_slice(spec: str) -> tuple[int, Optional[int]]:
    """Parse START:END (END optional). Negative indices are rejected."""
    if ":" not in spec:
        raise ValueError(f"slice must be START:END, got {spec!r}")
    start_s, end_s = spec.split(":", 1)
    if not start_s.strip():
        raise ValueError("slice START is required")
    start = int(start_s)
    end: Optional[int]
    if end_s.strip() == "":
        end = None
    else:
        end = int(end_s)
    if start < 0 or (end is not None and end < 0):
        raise ValueError("negative slice indices are not allowed")
    if end is not None and end < start:
        raise ValueError(f"slice END ({end}) must be >= START ({start})")
    return start, end


def select_instances(
    rows: Sequence[dict[str, Any]],
    *,
    instance_ids: Optional[Sequence[str]] = None,
    slice_spec: Optional[str] = None,
) -> list[dict[str, Any]]:
    """Filter dataset rows; reject duplicate requested IDs."""
    if instance_ids and slice_spec:
        raise ValueError("pass either --instance-id or --slice, not both")

    if instance_ids:
        wanted = list(instance_ids)
        if len(wanted) != len(set(wanted)):
            dupes = sorted({x for x in wanted if wanted.count(x) > 1})
            raise ValueError(f"duplicate instance ids: {dupes}")
        by_id = {str(r["instance_id"]): r for r in rows}
        missing = [i for i in wanted if i not in by_id]
        if missing:
            raise ValueError(f"unknown instance ids: {missing}")
        return [by_id[i] for i in wanted]

    ordered = list(rows)
    if slice_spec:
        start, end = parse_slice(slice_spec)
        ordered = ordered[start:end]
    return ordered


def load_verified_dataset(
    *,
    dataset_name: str = DATASET_NAME,
    split: str = DATASET_SPLIT,
) -> list[dict[str, Any]]:
    """Load the pinned SWE-bench Verified split via the swebench package."""
    try:
        with _installed_swebench_import_path():
            from swebench.harness.utils import load_swebench_dataset

            dataset = load_swebench_dataset(dataset_name, split)
    except ImportError as exc:  # pragma: no cover - env dependent
        raise RuntimeError(
            "swebench package is required; pip install -r "
            "workflows/swebench-verified/requirements.txt"
        ) from exc

    return [dict(row) for row in dataset]


def instance_image_key(raw: dict[str, Any], *, arch: str = "x86_64") -> str:
    """Resolve the official instance image key (no hand-rolled names)."""
    try:
        with _installed_swebench_import_path():
            from swebench.harness.test_spec.test_spec import make_test_spec

            spec = make_test_spec(raw, namespace="swebench", arch=arch)
    except ImportError as exc:  # pragma: no cover
        raise RuntimeError("swebench package is required for image keys") from exc

    return str(spec.instance_image_key)


def dataset_revision_info(rows: Sequence[dict[str, Any]]) -> dict[str, Any]:
    return {
        "dataset_name": DATASET_NAME,
        "split": DATASET_SPLIT,
        "count": len(rows),
        "fingerprint": fingerprint_rows(rows),
        "swebench_version": swebench_version(),
    }


def iter_safe_projections(rows: Iterable[dict[str, Any]]) -> list[SafeInstance]:
    out: list[SafeInstance] = []
    for row in rows:
        safe = project_safe(row)
        assert_no_oracle_leak(safe.to_dict())
        out.append(safe)
    return out
