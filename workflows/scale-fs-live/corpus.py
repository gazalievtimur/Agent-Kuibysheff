#!/usr/bin/env python3
"""Generate Scale-FS corpus trees for live LLM regression tasks."""

from __future__ import annotations

import hashlib
import json
import random
import re
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Optional

# Align with src/tools/fs_home.rs / local_tools.rs window defaults.
HOME_READ_DEFAULT_CHARS = 50_000
HOME_READ_MAX_CHARS = 200_000
# Large progressive-read stress size (well above a single default window).
OVERSIZE_MIN_BYTES = 800_000

_NOISE_LINES = (
    "status=ok subsystem=ingest latency_ms={n}",
    "status=warn subsystem=cache miss_ratio={n}",
    "note=routine_check batch={n} region=eu-west",
    "Project Atlas clearance deferred pending review #{n}",
    "Project Vega checkpoint hash={hex}",
    "heartbeat seq={n} worker=pool-{n}",
)


@dataclass(frozen=True)
class PlantResult:
    task_id: str
    kind: str
    needle: str
    paths: list[str]
    meta: dict[str, Any]

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def make_needle(task_id: str, seed: int) -> str:
    digest = hashlib.sha256(f"{task_id}:{seed}".encode("utf-8")).hexdigest()[:12].upper()
    safe = re.sub(r"[^A-Za-z0-9_-]", "_", task_id)
    return f"SF_NEEDLE_{safe}_{digest}"


def _rng(seed: int) -> random.Random:
    return random.Random(seed)


def _noise_line(rng: random.Random) -> str:
    template = rng.choice(_NOISE_LINES)
    return template.format(n=rng.randint(1, 99999), hex=rng.randbytes(4).hex())


def plant_many_files(
    *,
    workspace_root: Path,
    task_id: str,
    file_count: int = 400,
    seed: int = 42,
    needle_hint: str = "Project Orion",
) -> PlantResult:
    """Write ~file_count small text files under workspace_root/corpus/."""
    if file_count < 10:
        raise ValueError("file_count must be >= 10")
    rng = _rng(seed)
    needle = make_needle(task_id, seed)
    corpus = workspace_root / "corpus"
    if corpus.exists():
        for child in corpus.rglob("*"):
            if child.is_file():
                child.unlink()
        for child in sorted(corpus.rglob("*"), reverse=True):
            if child.is_dir():
                child.rmdir()
    corpus.mkdir(parents=True, exist_ok=True)

    needle_index = rng.randint(file_count // 4, file_count - 1)
    paths: list[str] = []
    for i in range(file_count):
        bucket = f"bucket-{i % 20:02d}"
        sub = f"batch-{i // 50:02d}"
        rel = Path("corpus") / bucket / sub / f"doc-{i:04d}.txt"
        path = workspace_root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        lines = [_noise_line(rng) for _ in range(rng.randint(8, 24))]
        if i == needle_index:
            lines.insert(
                rng.randint(0, len(lines)),
                f"{needle_hint} clearance_code={needle}",
            )
        elif rng.random() < 0.08:
            # Distractors: similar wording without the real token.
            lines.append(f"{needle_hint} clearance pending (no code assigned)")
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        paths.append(rel.as_posix())

    return PlantResult(
        task_id=task_id,
        kind="many_files",
        needle=needle,
        paths=paths,
        meta={
            "file_count": file_count,
            "needle_file": paths[needle_index],
            "needle_hint": needle_hint,
            "seed": seed,
        },
    )


def plant_large_read(
    *,
    home_dir: Path,
    task_id: str,
    total_chars: int = 100_000,
    seed: int = 7,
    relative_path: str = "in/app.log",
) -> PlantResult:
    """Write a home file larger than default home.read max_chars with needle near the end."""
    if total_chars <= HOME_READ_DEFAULT_CHARS + 5_000:
        raise ValueError(
            f"total_chars must exceed default read window ({HOME_READ_DEFAULT_CHARS}) "
            "by at least 5k"
        )
    if total_chars >= HOME_READ_MAX_CHARS:
        raise ValueError(f"total_chars must be < HOME_READ_MAX_CHARS ({HOME_READ_MAX_CHARS})")

    rng = _rng(seed)
    needle = make_needle(task_id, seed)
    path = home_dir / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)

    prefix_target = total_chars - 2_000
    chunks: list[str] = []
    while sum(len(c) for c in chunks) < prefix_target:
        chunks.append(_noise_line(rng) + "\n")
    body = "".join(chunks)
    if len(body) > prefix_target:
        body = body[:prefix_target]
    suffix = (
        f"checkpoint ok\nTRACE_ID={needle}\n"
        + "\n".join(_noise_line(rng) for _ in range(20))
        + "\n"
    )
    content = body + suffix
    path.write_text(content, encoding="utf-8")

    return PlantResult(
        task_id=task_id,
        kind="large_read",
        needle=needle,
        paths=[relative_path.replace("\\", "/")],
        meta={
            "char_count": len(content),
            "byte_count": len(content.encode("utf-8")),
            "needle_offset_chars": len(body),
            "default_read_chars": HOME_READ_DEFAULT_CHARS,
            "seed": seed,
        },
    )


def plant_oversize(
    *,
    home_dir: Path,
    task_id: str,
    target_bytes: int = 900_000,
    seed: int = 99,
    relative_path: str = "in/huge.log",
) -> PlantResult:
    """Write a home file far larger than one home.read window; needle mid-file."""
    if target_bytes <= OVERSIZE_MIN_BYTES:
        raise ValueError(
            f"target_bytes must be > OVERSIZE_MIN_BYTES ({OVERSIZE_MIN_BYTES})"
        )
    rng = _rng(seed)
    needle = make_needle(task_id, seed)
    path = home_dir / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)

    line = (_noise_line(rng) + "\n").encode("utf-8")
    repeats = max(1, (target_bytes - 256) // len(line))
    with path.open("wb") as fh:
        for i in range(repeats):
            fh.write(line)
            if i == repeats // 2:
                fh.write(f"SECRET_TOKEN={needle}\n".encode("utf-8"))
        # Pad to target size with a final noise line block if short.
        while path.stat().st_size < target_bytes:
            fh.write(line)

    size = path.stat().st_size
    if size <= OVERSIZE_MIN_BYTES:
        raise RuntimeError(f"oversize plant too small: {size} bytes")

    return PlantResult(
        task_id=task_id,
        kind="oversize",
        needle=needle,
        paths=[relative_path.replace("\\", "/")],
        meta={
            "byte_count": size,
            "default_read_chars": HOME_READ_DEFAULT_CHARS,
            "seed": seed,
        },
    )


def plant_from_task(
    task: dict[str, Any],
    *,
    workspace_root: Path,
    home_dir: Path,
) -> PlantResult:
    task_id = str(task.get("id") or "task")
    corpus = task.get("corpus") if isinstance(task.get("corpus"), dict) else {}
    kind = str(corpus.get("kind") or "")
    seed = int(corpus.get("seed") or 1)

    if kind == "many_files":
        return plant_many_files(
            workspace_root=workspace_root,
            task_id=task_id,
            file_count=int(corpus.get("file_count") or 400),
            seed=seed,
            needle_hint=str(corpus.get("needle_hint") or "Project Orion"),
        )
    if kind == "large_read":
        return plant_large_read(
            home_dir=home_dir,
            task_id=task_id,
            total_chars=int(corpus.get("total_chars") or 100_000),
            seed=seed,
            relative_path=str(corpus.get("path") or "in/app.log"),
        )
    if kind == "oversize":
        return plant_oversize(
            home_dir=home_dir,
            task_id=task_id,
            target_bytes=int(corpus.get("target_bytes") or 900_000),
            seed=seed,
            relative_path=str(corpus.get("path") or "in/huge.log"),
        )
    raise ValueError(f"unknown corpus.kind: {kind!r}")


def write_needle_sidecar(path: Path, planted: PlantResult) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(planted.to_dict(), indent=2), encoding="utf-8")


def load_needle_sidecar(path: Path) -> PlantResult:
    data = json.loads(path.read_text(encoding="utf-8-sig"))
    return PlantResult(
        task_id=str(data["task_id"]),
        kind=str(data["kind"]),
        needle=str(data["needle"]),
        paths=list(data.get("paths") or []),
        meta=dict(data.get("meta") or {}),
    )


def verify_planted(planted: PlantResult, *, workspace_root: Path, home_dir: Path) -> None:
    """Raise AssertionError if planted artifacts do not match expectations."""
    if not planted.needle.startswith("SF_NEEDLE_"):
        raise AssertionError(f"bad needle: {planted.needle}")

    if planted.kind == "many_files":
        needle_file = planted.meta.get("needle_file")
        if not isinstance(needle_file, str):
            raise AssertionError("missing needle_file meta")
        path = workspace_root / needle_file
        text = path.read_text(encoding="utf-8")
        if planted.needle not in text:
            raise AssertionError("needle missing from many_files corpus")
        count = int(planted.meta.get("file_count") or 0)
        found = sum(1 for p in planted.paths if (workspace_root / p).is_file())
        if found != count:
            raise AssertionError(f"expected {count} files, found {found}")
        return

    if planted.kind == "large_read":
        rel = planted.paths[0]
        path = home_dir / rel
        text = path.read_text(encoding="utf-8")
        if planted.needle not in text:
            raise AssertionError("needle missing from large_read file")
        offset = text.index(f"TRACE_ID={planted.needle}")
        if offset < HOME_READ_DEFAULT_CHARS:
            raise AssertionError("needle must sit past default home.read window")
        if len(text) >= HOME_READ_MAX_CHARS:
            raise AssertionError("large_read must stay under per-call max_chars")
        return

    if planted.kind == "oversize":
        rel = planted.paths[0]
        path = home_dir / rel
        size = path.stat().st_size
        if size <= OVERSIZE_MIN_BYTES:
            raise AssertionError(f"oversize file too small: {size}")
        # Streaming scan without loading entire file into a huge str if possible.
        found = False
        needle_offset = 0
        with path.open("r", encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if planted.needle in line:
                    found = True
                    break
                needle_offset += len(line)
        if not found:
            raise AssertionError("needle missing from oversize file")
        if needle_offset < HOME_READ_DEFAULT_CHARS:
            raise AssertionError("oversize needle must sit past default home.read window")
        return

    raise AssertionError(f"unknown kind: {planted.kind}")


def main(argv: Optional[list[str]] = None) -> int:
    import argparse
    import tempfile

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kind", choices=("many_files", "large_read", "oversize"), required=True)
    parser.add_argument("--task-id", default="manual")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--out", type=Path, default=None, help="Workspace or home root")
    args = parser.parse_args(argv)

    root = args.out
    if root is None:
        root = Path(tempfile.mkdtemp(prefix="scale-fs-"))
    root.mkdir(parents=True, exist_ok=True)

    if args.kind == "many_files":
        planted = plant_many_files(workspace_root=root, task_id=args.task_id, seed=args.seed)
        verify_planted(planted, workspace_root=root, home_dir=root)
    elif args.kind == "large_read":
        planted = plant_large_read(home_dir=root, task_id=args.task_id, seed=args.seed)
        verify_planted(planted, workspace_root=root, home_dir=root)
    else:
        planted = plant_oversize(home_dir=root, task_id=args.task_id, seed=args.seed)
        verify_planted(planted, workspace_root=root, home_dir=root)

    print(json.dumps(planted.to_dict(), indent=2))
    print(f"root={root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
