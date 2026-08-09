#!/usr/bin/env python3
"""SWE-bench Verified workflow CLI: preflight | generate | grade | report | run."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Optional, Sequence

WORKFLOW_DIR = Path(__file__).resolve().parent
if str(WORKFLOW_DIR) not in sys.path:
    sys.path.insert(0, str(WORKFLOW_DIR))

from runtime import (  # noqa: E402
    DEFAULT_AGENT_ID,
    DEFAULT_HOME_REL,
    build_manifest,
    build_report,
    check_docker_linux,
    default_run_id,
    docker_from_env,
    ensure_windows_resource_stub_on_path,
    generate_batch,
    instance_paths,
    link_harness_logs,
    load_dotenv,
    provider_api_key_available,
    pull_instance_image,
    read_status,
    reduce_predictions,
    resolve_agent_binary,
    run_official_grade,
    write_json_atomic,
    yaml_scalar,
)
from swebench_adapter import (  # noqa: E402
    DATASET_NAME,
    DATASET_SPLIT,
    GOLD_SMOKE_INSTANCE_ID,
    dataset_revision_info,
    instance_image_key,
    load_verified_dataset,
    project_safe,
    select_instances,
    swebench_version,
)

DEFAULT_CONFIG = str(WORKFLOW_DIR / "solver" / "agent-config.example.yaml")
DEFAULT_SETTINGS = str(WORKFLOW_DIR / "solver")


def _repo_guess() -> str:
    """Optional monorepo root when package lives under workflows/<name>/."""
    if WORKFLOW_DIR.parent.name == "workflows":
        return str(WORKFLOW_DIR.parent.parent)
    return ""


def add_common_filters(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--instance-id",
        action="append",
        default=[],
        dest="instance_ids",
        help="Instance id (repeatable)",
    )
    parser.add_argument(
        "--slice",
        default="",
        dest="slice_spec",
        help="Deterministic range START:END over dataset order",
    )
    parser.add_argument("--workers", type=int, default=1, help="Parallelism")
    parser.add_argument("--run-id", default="", help="Explicit run id")
    parser.add_argument(
        "--resume",
        action="store_true",
        help="Skip terminal ok instances with valid patches",
    )
    parser.add_argument("--agent-bin", default="", help="Path to agent_Kuibysheff")
    parser.add_argument(
        "--agent",
        default=DEFAULT_AGENT_ID,
        help=f"Agent id under protected/agents/ (default: {DEFAULT_AGENT_ID})",
    )
    parser.add_argument(
        "--project-root",
        default="",
        help="Unused for generate (each instance dir is the project); kept for CLI symmetry",
    )
    parser.add_argument(
        "--home",
        default=DEFAULT_HOME_REL,
        help=f"Relative home under .kuibysheff/ (default: {DEFAULT_HOME_REL})",
    )
    parser.add_argument(
        "--config",
        default=DEFAULT_CONFIG,
        help="Provider config template (import/render source only; not passed to agent)",
    )
    parser.add_argument(
        "--settings-dir",
        default=DEFAULT_SETTINGS,
        help="Solver template dir imported into the protected profile (not passed to agent)",
    )
    parser.add_argument(
        "--repo-root",
        default=_repo_guess(),
        help="Optional monorepo root for Cargo binary / .env fallback",
    )
    parser.add_argument(
        "--model-name",
        default="",
        help="model_name_or_path for predictions.jsonl (default: provider.model)",
    )
    parser.add_argument("-v", "--verbose", action="store_true")


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="SWE-bench Verified workflow for agent_Kuibysheff"
    )
    sub = parser.add_subparsers(dest="command", required=True)

    for name in ("preflight", "generate", "grade", "report", "run"):
        p = sub.add_parser(name)
        add_common_filters(p)
        if name == "preflight":
            p.add_argument(
                "--skip-gold",
                action="store_true",
                help="Skip official gold harness smoke",
            )
            p.add_argument(
                "--gold-instance",
                default=GOLD_SMOKE_INSTANCE_ID,
                help="Instance id for gold smoke",
            )

    return parser.parse_args(argv)


def _paths(args: argparse.Namespace) -> tuple[Path, Path, Path, Path]:
    unit_root = WORKFLOW_DIR
    load_dotenv(unit_root / ".env")
    load_dotenv(Path.cwd() / ".env")
    repo_root = Path(args.repo_root).resolve() if args.repo_root else unit_root
    if args.repo_root:
        load_dotenv(repo_root / ".env")
    config = Path(args.config)
    if not config.is_absolute():
        config = (unit_root / config).resolve()
    settings = Path(args.settings_dir)
    if not settings.is_absolute():
        settings = (unit_root / settings).resolve()
    runs_root = WORKFLOW_DIR / "runs"
    return repo_root, config, settings, runs_root


def _model_name(args: argparse.Namespace, config: Path) -> str:
    if args.model_name.strip():
        return args.model_name.strip()
    text = config.read_text(encoding="utf-8") if config.is_file() else ""
    return yaml_scalar(text, "model", "agent_Kuibysheff")


def cmd_preflight(args: argparse.Namespace) -> int:
    repo_root, config, settings, _runs = _paths(args)
    print("== preflight: Docker ==")
    info = check_docker_linux()
    print(json.dumps(info, indent=2))

    print("== preflight: agent binary ==")
    override = Path(args.agent_bin) if args.agent_bin else None
    agent_bin = resolve_agent_binary(repo_root, override)
    print(f"agent_bin={agent_bin}")

    print("== preflight: API key ==")
    if not config.is_file():
        raise SystemExit(f"config not found: {config}")
    if not provider_api_key_available(config.read_text(encoding="utf-8")):
        raise SystemExit(
            "provider API key missing: set the env named by provider.api_key_env "
            "(inline provider.api_key is rejected)"
        )
    print("API key present via api_key_env (value not logged)")

    print("== preflight: import template (settings) ==")
    for name in ("master_prompt.md", "skills.dsl", "rules.md"):
        path = settings / name
        if not path.is_file():
            raise SystemExit(f"missing settings file: {path}")
        print(f"ok {path.name}")
    print(f"agent_id={args.agent} home_rel={args.home}")

    print("== preflight: dataset ==")
    rows = load_verified_dataset()
    meta = dataset_revision_info(rows)
    print(json.dumps(meta, indent=2))
    print(f"swebench_version={swebench_version()}")

    # Resolve one official image
    smoke_id = args.gold_instance if hasattr(args, "gold_instance") else GOLD_SMOKE_INSTANCE_ID
    selected = select_instances(rows, instance_ids=[smoke_id])
    image = instance_image_key(selected[0])
    print(f"== preflight: instance image {image} ==")
    client = docker_from_env()
    digest = pull_instance_image(client, image)
    print(f"image_digest={digest}")

    if getattr(args, "skip_gold", False):
        print("skipping gold harness smoke (--skip-gold)")
        return 0

    print("== preflight: gold harness smoke ==")
    gold_run_id = f"validate-gold-{default_run_id()}"
    proc = run_official_grade(
        predictions_path="gold",
        run_id=gold_run_id,
        max_workers=1,
        instance_ids=[smoke_id],
        cwd=repo_root,
    )
    if args.verbose:
        sys.stdout.write(proc.stdout)
        sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"gold harness failed with exit {proc.returncode}")
    print("gold harness smoke OK")
    return 0


def _load_selected(args: argparse.Namespace) -> list[dict[str, Any]]:
    rows = load_verified_dataset()
    return select_instances(
        rows,
        instance_ids=args.instance_ids or None,
        slice_spec=args.slice_spec or None,
    )


def cmd_generate(args: argparse.Namespace) -> int:
    repo_root, config, settings, runs_root = _paths(args)
    run_id = args.run_id.strip() or default_run_id()
    run_dir = runs_root / run_id
    override = Path(args.agent_bin) if args.agent_bin else None
    agent_bin = resolve_agent_binary(repo_root, override)
    model = _model_name(args, config)

    selected = _load_selected(args)
    if not selected:
        raise SystemExit("no instances selected")
    # Validate projections early
    for row in selected:
        project_safe(row)

    print(f"generate run_id={run_id} instances={len(selected)} workers={args.workers}")
    statuses = generate_batch(
        rows=selected,
        run_dir=run_dir,
        run_id=run_id,
        repo_root=repo_root,
        base_config=config,
        settings_dir=settings,
        agent_bin=agent_bin,
        model_name_or_path=model,
        workers=args.workers,
        resume=args.resume,
        agent_id=args.agent,
        home_rel=args.home,
    )
    pred = reduce_predictions(run_dir, model_name_or_path=model)
    print(f"predictions={pred} statuses={len(statuses)}")

    image_digests = {
        str(s.get("instance_id")): str(s.get("image_digest") or "")
        for s in statuses
        if s.get("image_digest")
    }
    manifest = build_manifest(
        run_id=run_id,
        repo_root=repo_root,
        settings_dir=settings,
        base_config=config,
        agent_bin=agent_bin,
        cli_args=vars(args),
        dataset_info=dataset_revision_info(selected),
        image_digests=image_digests,
    )
    write_json_atomic(run_dir / "manifest.json", manifest)
    ok = sum(1 for s in statuses if s.get("status") == "ok")
    print(f"generated_ok={ok}/{len(statuses)}")
    return 0 if ok > 0 or args.resume else 1


def cmd_grade(args: argparse.Namespace) -> int:
    repo_root, config, _settings, runs_root = _paths(args)
    run_id = args.run_id.strip()
    if not run_id:
        raise SystemExit("--run-id is required for grade")
    run_dir = runs_root / run_id
    predictions = run_dir / "predictions.jsonl"
    if not predictions.is_file():
        raise SystemExit(f"missing predictions: {predictions}")
    pred_text = predictions.read_text(encoding="utf-8").strip()
    if not pred_text:
        print(
            "grade skipped: predictions.jsonl is empty "
            "(no successful patches to evaluate)",
            file=sys.stderr,
        )
        return 1
    model = _model_name(args, config)

    instance_ids = args.instance_ids or None
    print(f"grade run_id={run_id} predictions={predictions}")
    proc = run_official_grade(
        predictions_path=predictions,
        run_id=run_id,
        max_workers=max(1, args.workers),
        instance_ids=instance_ids,
        cwd=repo_root,
    )
    (run_dir / "harness-stdout.txt").write_text(proc.stdout, encoding="utf-8")
    (run_dir / "harness-stderr.txt").write_text(proc.stderr, encoding="utf-8")
    link_harness_logs(run_dir, run_id, model)
    if args.verbose:
        sys.stdout.write(proc.stdout)
        sys.stderr.write(proc.stderr)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        return proc.returncode
    print("official harness finished")
    return 0


def _collect_statuses(run_dir: Path, selected_ids: Sequence[str]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for iid in selected_ids:
        paths = instance_paths(run_dir, iid)
        st = read_status(paths)
        if st:
            out.append(st)
        else:
            out.append({"instance_id": iid, "status": "missing"})
    return out


def cmd_report(args: argparse.Namespace) -> int:
    repo_root, config, settings, runs_root = _paths(args)
    run_id = args.run_id.strip()
    if not run_id:
        raise SystemExit("--run-id is required for report")
    run_dir = runs_root / run_id
    if not run_dir.is_dir():
        raise SystemExit(f"run dir missing: {run_dir}")

    # Prefer instances present on disk; optionally intersect with filters.
    instances_root = run_dir / "instances"
    if args.instance_ids:
        selected_ids = list(args.instance_ids)
    elif instances_root.is_dir():
        selected_ids = sorted(p.name for p in instances_root.iterdir() if p.is_dir())
    else:
        selected_ids = []

    statuses = _collect_statuses(run_dir, selected_ids)
    report = build_report(
        run_dir=run_dir,
        run_id=run_id,
        selected_ids=selected_ids,
        statuses=statuses,
    )
    write_json_atomic(run_dir / "report.json", report)

    override = Path(args.agent_bin) if args.agent_bin else None
    try:
        agent_bin = resolve_agent_binary(repo_root, override)
    except FileNotFoundError:
        agent_bin = Path("unavailable")

    image_digests = {
        str(s.get("instance_id")): str(s.get("image_digest") or "")
        for s in statuses
        if s.get("image_digest")
    }
    if not (run_dir / "manifest.json").is_file():
        write_json_atomic(
            run_dir / "manifest.json",
            build_manifest(
                run_id=run_id,
                repo_root=repo_root,
                settings_dir=settings,
                base_config=config,
                agent_bin=agent_bin,
                cli_args=vars(args),
                dataset_info={
                    "dataset_name": DATASET_NAME,
                    "split": DATASET_SPLIT,
                    "count": len(selected_ids),
                },
                image_digests=image_digests,
            ),
        )
    print(json.dumps({k: report[k] for k in report if k != "per_instance"}, indent=2))
    print(f"wrote {run_dir / 'report.json'}")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    """generate → grade → report with a shared run_id."""
    if not args.run_id.strip():
        args.run_id = default_run_id()
    rc = cmd_generate(args)
    rc_g = cmd_grade(args)
    rc_r = cmd_report(args)
    if rc != 0:
        return rc
    if rc_g != 0:
        return rc_g
    return rc_r


def main(argv: Optional[Sequence[str]] = None) -> int:
    ensure_windows_resource_stub_on_path()
    args = parse_args(argv)
    commands = {
        "preflight": cmd_preflight,
        "generate": cmd_generate,
        "grade": cmd_grade,
        "report": cmd_report,
        "run": cmd_run,
    }
    try:
        return commands[args.command](args)
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
