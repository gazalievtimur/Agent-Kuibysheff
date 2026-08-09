#!/usr/bin/env python3
"""Offline unit/smoke checks for the SWE-bench Verified workflow (no Docker/LLM)."""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

WORKFLOW_DIR = Path(__file__).resolve().parent
if str(WORKFLOW_DIR) not in sys.path:
    sys.path.insert(0, str(WORKFLOW_DIR))

from docker_workspace_mcp import WorkspaceError, resolve_testbed_path  # noqa: E402
from runtime import (  # noqa: E402
    STATUS_EMPTY_PATCH,
    STATUS_INFRA_ERROR,
    STATUS_OK,
    classify_agent_result,
    escape_yaml_double,
    extract_json_object,
    instance_paths,
    reduce_predictions,
    render_agent_prompt,
    render_run_config,
    should_skip_instance,
    write_json_atomic,
    write_text_atomic,
)
from swebench_adapter import (  # noqa: E402
    ORACLE_FIELDS,
    SafeInstance,
    assert_no_oracle_leak,
    fingerprint_rows,
    parse_slice,
    project_safe,
    select_instances,
)


def test_projection_strips_oracle() -> None:
    raw = {
        "instance_id": "demo__demo-1",
        "repo": "demo/demo",
        "base_commit": "abc123",
        "problem_statement": "Fix the bug",
        "patch": "SECRET_GOLD",
        "test_patch": "SECRET_TEST",
        "FAIL_TO_PASS": '["t1"]',
        "PASS_TO_PASS": '["t2"]',
        "hints_text": "hint",
    }
    safe = project_safe(raw)
    payload = safe.to_dict()
    assert_no_oracle_leak(payload)
    for field in ORACLE_FIELDS:
        assert field not in payload
    prompt = render_agent_prompt(safe)
    for field in ("SECRET_GOLD", "SECRET_TEST", "FAIL_TO_PASS", "PASS_TO_PASS", "hints_text"):
        assert field not in prompt


def test_duplicate_instance_ids_rejected() -> None:
    rows = [
        {
            "instance_id": "a",
            "repo": "r",
            "base_commit": "c",
            "problem_statement": "p",
        }
    ]
    try:
        select_instances(rows, instance_ids=["a", "a"])
        raise AssertionError("expected duplicate rejection")
    except ValueError as exc:
        assert "duplicate" in str(exc)


def test_slice_and_fingerprint() -> None:
    rows = [
        {"instance_id": f"i{i}", "base_commit": f"c{i}", "repo": "r", "problem_statement": "p"}
        for i in range(5)
    ]
    selected = select_instances(rows, slice_spec="1:3")
    assert [r["instance_id"] for r in selected] == ["i1", "i2"]
    start, end = parse_slice("0:2")
    assert (start, end) == (0, 2)
    fp1 = fingerprint_rows(rows)
    fp2 = fingerprint_rows(list(reversed(rows)))
    assert fp1 == fp2


def test_yaml_escape_and_config_render() -> None:
    escaped = escape_yaml_double('say "hi"\\path')
    assert '\\"' in escaped
    text = render_run_config(
        base_config_text='model: "gpt-test"\napi_key_env: "OPENAI_API_KEY"\n',
        mcp_script=WORKFLOW_DIR / "docker_workspace_mcp.py",
        container_id="cid123",
        log_dir=Path("/tmp/logs"),
        python_exe=Path(sys.executable),
    )
    assert "name: \"workspace\"" in text
    assert "SWEBENCH_CONTAINER_ID: \"cid123\"" in text
    assert "builtins: []" in text
    assert "home.run" not in text
    assert "\n  api_key:" not in text
    assert "api_key_env:" in text
    # Stdio MCP children get a cleared env; site-packages must be forwarded.
    assert "PYTHONPATH:" in text


def test_reducer_deterministic() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        run_dir = Path(tmp)
        for iid, patch in (("b-id", "diff b\n"), ("a-id", "diff a\n")):
            paths = instance_paths(run_dir, iid)
            paths.instance_dir.mkdir(parents=True)
            write_text_atomic(paths.patch_path, patch)
            write_json_atomic(
                paths.status_path,
                {"status": STATUS_OK, "instance_id": iid, "patch_path": str(paths.patch_path)},
            )
        # Extra non-ok should be skipped
        bad = instance_paths(run_dir, "c-id")
        bad.instance_dir.mkdir(parents=True)
        write_json_atomic(bad.status_path, {"status": STATUS_EMPTY_PATCH, "instance_id": "c-id"})

        out1 = reduce_predictions(run_dir, model_name_or_path="m")
        lines1 = out1.read_text(encoding="utf-8").splitlines()
        assert [json.loads(l)["instance_id"] for l in lines1] == ["a-id", "b-id"]
        out2 = reduce_predictions(run_dir, model_name_or_path="m")
        assert out1.read_text(encoding="utf-8") == out2.read_text(encoding="utf-8")


def test_resume_skip_logic() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        paths = instance_paths(Path(tmp), "x")
        paths.instance_dir.mkdir(parents=True)
        write_text_atomic(paths.patch_path, "diff --git a/f b/f\n")
        ok = {
            "status": STATUS_OK,
            "instance_id": "x",
            "patch_path": str(paths.patch_path),
        }
        assert should_skip_instance(ok, resume=True) is True
        assert should_skip_instance(ok, resume=False) is False
        infra = {"status": STATUS_INFRA_ERROR, "instance_id": "x"}
        assert should_skip_instance(infra, resume=True) is False
        empty_ok = {"status": STATUS_OK, "instance_id": "x", "patch_path": str(paths.instance_dir / "missing.patch")}
        assert should_skip_instance(empty_ok, resume=True) is False


def test_runoutput_parser_nonzero_exit() -> None:
    raw = 'noise\n{"run_id":"r","result":"summary","stop_reason":"error","usage":{"cost":{"status":"unavailable"}}}\n'
    obj = extract_json_object(raw)
    assert obj["stop_reason"] == "error"
    status, parsed = classify_agent_result(1, raw)
    assert status == "agent_error"
    assert parsed is not None
    assert parsed["usage"]["cost"]["status"] == "unavailable"

    ok_raw = '{"run_id":"r","result":"ok","stop_reason":"goal_reached","usage":{}}'
    status2, parsed2 = classify_agent_result(0, ok_raw)
    assert status2 == STATUS_OK
    assert parsed2["stop_reason"] == "goal_reached"


def test_mcp_path_guards() -> None:
    assert str(resolve_testbed_path("src/main.py")) == "/testbed/src/main.py"
    assert str(resolve_testbed_path(".")) == "/testbed"
    for bad in ("/etc/passwd", "../escape", "..\\escape", "C:\\windows", "~/secret"):
        try:
            resolve_testbed_path(bad)
            raise AssertionError(f"expected reject for {bad!r}")
        except WorkspaceError:
            pass


def test_prompt_escaping_values() -> None:
    safe = SafeInstance(
        instance_id='id"quote',
        repo="org/repo",
        base_commit="deadbeef",
        problem_statement='line with "quotes" and \\ backslash',
    )
    prompt = render_agent_prompt(safe)
    assert 'id"quote' in prompt
    assert "deadbeef" in prompt


def test_harness_bootstrap_lf_writes() -> None:
    """Path.write_text after bootstrap patch must emit LF-only bytes on all OSes."""
    import importlib.util

    bootstrap_path = WORKFLOW_DIR / "harness_bootstrap.py"
    spec = importlib.util.spec_from_file_location("harness_bootstrap", bootstrap_path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    mod._install_lf_path_writes()

    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "eval.sh"
        path.write_text("#!/bin/bash\nset -uxo pipefail\necho ok\n", encoding="utf-8")
        raw = path.read_bytes()
        assert b"\r" not in raw
        assert raw.startswith(b"#!/bin/bash\n")


def test_assert_regression_gate() -> None:
    from assert_regression import main as assert_main

    with tempfile.TemporaryDirectory() as tmp:
        report_path = Path(tmp) / "report.json"
        report_path.write_text(
            json.dumps(
                {
                    "run_id": "regression-test",
                    "resolved": 1,
                    "graded": 1,
                    "generated_patches": 1,
                    "agent_errors": 0,
                    "infrastructure_errors": 0,
                    "per_instance": [
                        {
                            "instance_id": "sympy__sympy-20590",
                            "status": "ok",
                            "stop_reason": "goal_reached",
                            "harness_resolved": True,
                            "elapsed_sec": 12.5,
                            "usage": {
                                "total_tokens": 100,
                                "cost": {
                                    "status": "complete",
                                    "amount": 0.01,
                                    "currency": "USD",
                                },
                            },
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        assert assert_main([str(report_path), "--instance-id", "sympy__sympy-20590"]) == 0

        fail_path = Path(tmp) / "fail.json"
        fail_path.write_text(
            json.dumps(
                {
                    "run_id": "regression-fail",
                    "resolved": 0,
                    "graded": 1,
                    "generated_patches": 1,
                    "agent_errors": 0,
                    "infrastructure_errors": 0,
                    "per_instance": [
                        {
                            "instance_id": "sympy__sympy-20590",
                            "status": "ok",
                            "stop_reason": "goal_reached",
                            "harness_resolved": False,
                            "elapsed_sec": 9,
                            "usage": {},
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        assert assert_main([str(fail_path), "--instance-id", "sympy__sympy-20590"]) == 1


def main() -> int:
    test_projection_strips_oracle()
    test_duplicate_instance_ids_rejected()
    test_slice_and_fingerprint()
    test_yaml_escape_and_config_render()
    test_reducer_deterministic()
    test_resume_skip_logic()
    test_runoutput_parser_nonzero_exit()
    test_mcp_path_guards()
    test_prompt_escaping_values()
    test_harness_bootstrap_lf_writes()
    test_assert_regression_gate()
    print("OK: offline smoke checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
