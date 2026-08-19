#!/usr/bin/env python3
"""Run a reproducible Codex MCP benchmark and create a Markdown report.

The runner deliberately starts each task with ``codex exec --ephemeral`` and
``--ignore-user-config``.  This keeps desktop settings and previous task
context out of the experiment.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import statistics
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
DEFAULT_BINARY = ROOT.parent / "target" / "release" / "connectome"
DEFAULT_SCHEMA = ROOT / "schemas" / "answer.schema.json"
DEFAULT_TASKS = ROOT / "tasks.example.jsonl"


@dataclass
class Task:
    id: str
    prompt: str
    required_facts: list[str]
    expected_files: list[str]
    expected_symbols: list[str]


def toml_string(value: str) -> str:
    """JSON basic strings are also valid TOML basic strings."""
    return json.dumps(value)


def load_tasks(path: Path) -> list[Task]:
    tasks: list[Task] = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        try:
            raw = json.loads(line)
        except json.JSONDecodeError as error:
            raise SystemExit(f"{path}:{number}: invalid JSON: {error}") from error
        if not isinstance(raw.get("id"), str) or not isinstance(raw.get("prompt"), str):
            raise SystemExit(f"{path}:{number}: each task needs string id and prompt fields")
        tasks.append(
            Task(
                id=raw["id"],
                prompt=raw["prompt"],
                required_facts=list(raw.get("required_facts", [])),
                expected_files=list(raw.get("expected_files", [])),
                expected_symbols=list(raw.get("expected_symbols", [])),
            )
        )
    if not tasks:
        raise SystemExit(f"{path}: no tasks found")
    return tasks


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for line in path.read_text(errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            events.append(value)
    return events


def walk(value: Any):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def extract_usage(events: list[dict[str, Any]]) -> dict[str, int | None]:
    """Pick the final event carrying a recognizable token-usage object."""
    chosen: dict[str, Any] | None = None
    for event in events:
        for item in walk(event):
            usage = item.get("usage")
            if isinstance(usage, dict) and any("token" in str(key).lower() for key in usage):
                chosen = usage
    if chosen is None:
        return {
            "input_tokens": None,
            "cached_input_tokens": None,
            "uncached_input_tokens": None,
            "output_tokens": None,
            "reasoning_tokens": None,
            "total_tokens": None,
        }

    def number(*names: str) -> int | None:
        for name in names:
            value = chosen.get(name)
            if isinstance(value, (int, float)):
                return int(value)
        return None

    input_tokens = number("input_tokens", "input")
    cached_tokens = number("cached_input_tokens", "cached_input")
    output_tokens = number("output_tokens", "output")
    reasoning_tokens = number("reasoning_tokens", "reasoning_output_tokens", "reasoning")
    total_tokens = number("total_tokens", "total")
    if total_tokens is None and input_tokens is not None and output_tokens is not None:
        total_tokens = input_tokens + output_tokens
    return {
        "input_tokens": input_tokens,
        "cached_input_tokens": cached_tokens,
        "uncached_input_tokens": (
            max(0, input_tokens - (cached_tokens or 0)) if input_tokens is not None else None
        ),
        "output_tokens": output_tokens,
        "reasoning_tokens": reasoning_tokens,
        "total_tokens": total_tokens,
    }


def mcp_call_counts(events: list[dict[str, Any]]) -> dict[str, int]:
    started = successful = failed = navigation_successful = 0
    non_navigation_tools = {"list_projects", "index_repository", "index_status", "check_index_coverage"}
    for event in events:
        item = event.get("item")
        if not isinstance(item, dict) or item.get("type") != "mcp_tool_call":
            continue
        if event.get("type") == "item.started":
            started += 1
        elif event.get("type") == "item.completed":
            if item.get("status") == "completed" and item.get("error") is None:
                successful += 1
                if item.get("tool") not in non_navigation_tools:
                    navigation_successful += 1
            else:
                failed += 1
    return {
        "mcp_tool_calls": started,
        "mcp_successful_calls": successful,
        "mcp_navigation_successful_calls": navigation_successful,
        "mcp_failed_calls": failed,
    }


def score_answer(answer: str, task: Task) -> dict[str, float | None]:
    normalized = answer.lower()
    try:
        parsed = json.loads(answer)
    except json.JSONDecodeError:
        parsed = {}
    evidence = parsed.get("evidence", []) if isinstance(parsed, dict) else []
    evidence = evidence if isinstance(evidence, list) else []
    paths = [str(item.get("path", "")).replace("\\", "/").lower() for item in evidence if isinstance(item, dict)]
    symbols = [str(item.get("symbol", "")).lower() for item in evidence if isinstance(item, dict)]

    def coverage(expected: list[str], values: list[str], suffix: bool = False) -> tuple[float | None, int | None, int | None]:
        if not expected:
            return None, None, None
        normalized_expected = [value.lower() for value in expected]
        if suffix:
            found = sum(1 for expected_value in normalized_expected if any(value.endswith(expected_value) for value in values))
        else:
            found = sum(1 for expected_value in normalized_expected if any(expected_value in value for value in values))
        return found / len(expected), found, len(expected)

    required = [fact.lower() for fact in task.required_facts]
    facts_found = sum(1 for fact in required if fact in normalized) if required else None
    fact_score = facts_found / len(required) if required else None
    symbol_score, symbols_found, symbols_expected = coverage(task.expected_symbols, symbols)
    file_score, files_found, files_expected = coverage(task.expected_files, paths, suffix=True)
    scores = [score for score in (fact_score, symbol_score, file_score) if score is not None]
    return {
        "quality_score": sum(scores) / len(scores) if scores else None,
        "facts_found": facts_found,
        "facts_expected": len(required) if required else None,
        "symbols_found": symbols_found,
        "symbols_expected": symbols_expected,
        "files_found": files_found,
        "files_expected": files_expected,
    }


def condition_overrides(args: argparse.Namespace, condition: str) -> list[str]:
    if condition == "native":
        return []
    if condition == "connectome":
        return [
            "-c",
            f"mcp_servers.connectome.command={toml_string(str(args.connectome))}",
            "-c",
            f"mcp_servers.connectome.cwd={toml_string(str(args.target))}",
            "-c",
            "mcp_servers.connectome.required=true",
            "-c",
            "mcp_servers.connectome.startup_timeout_sec=30",
            "-c",
            "mcp_servers.connectome.tool_timeout_sec=120",
        ]
    if condition == "legacy":
        if not args.legacy_command:
            raise SystemExit("legacy condition requires --legacy-command")
        name = args.legacy_name
        overrides = [
            "-c",
            f"mcp_servers.{name}.command={toml_string(args.legacy_command)}",
            "-c",
            f"mcp_servers.{name}.cwd={toml_string(str(args.target))}",
            "-c",
            f"mcp_servers.{name}.required=true",
            "-c",
            f"mcp_servers.{name}.startup_timeout_sec=30",
            "-c",
            f"mcp_servers.{name}.tool_timeout_sec=120",
        ]
        if args.legacy_arg:
            rendered = ",".join(toml_string(item) for item in args.legacy_arg)
            overrides += ["-c", f"mcp_servers.{name}.args=[{rendered}]"]
        return overrides
    raise AssertionError(condition)


def run_index(args: argparse.Namespace, output: Path) -> dict[str, Any]:
    command = [str(args.connectome), "index", str(args.target), "--lsp-mode", args.lsp_mode]
    started = time.monotonic()
    completed = subprocess.run(command, capture_output=True, text=True)
    elapsed_ms = round((time.monotonic() - started) * 1000)
    result = {
        "command": command,
        "exit_code": completed.returncode,
        "elapsed_ms": elapsed_ms,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
    }
    try:
        result["overview"] = json.loads(completed.stdout)
    except json.JSONDecodeError:
        pass
    output.write_text(json.dumps(result, indent=2) + "\n")
    if completed.returncode:
        raise SystemExit(f"Connectome indexing failed; see {output}")
    return result


def run_legacy_index(args: argparse.Namespace, output: Path) -> dict[str, Any]:
    if not args.legacy_command:
        raise SystemExit("legacy indexing requires --legacy-command")
    command = [
        args.legacy_index_command or args.legacy_command,
        "cli",
        "index_repository",
        "--repo-path",
        str(args.target),
        "--mode",
        args.legacy_index_mode,
    ]
    started = time.monotonic()
    completed = subprocess.run(command, capture_output=True, text=True)
    elapsed_ms = round((time.monotonic() - started) * 1000)
    result = {
        "command": command,
        "exit_code": completed.returncode,
        "elapsed_ms": elapsed_ms,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
        "mode": args.legacy_index_mode,
    }
    output.write_text(json.dumps(result, indent=2) + "\n")
    if completed.returncode:
        raise SystemExit(f"Legacy MCP indexing failed; see {output}")
    return result


def git_revision(path: Path) -> str | None:
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=path, capture_output=True, text=True
    )
    return completed.stdout.strip() if completed.returncode == 0 else None


def thread_id(events: list[dict[str, Any]]) -> str | None:
    for event in events:
        if event.get("type") == "thread.started" and isinstance(event.get("thread_id"), str):
            return event["thread_id"]
    return None


def run_one(args: argparse.Namespace, condition: str, task: Task, repetition: int, run_dir: Path, session_id: str | None = None) -> tuple[dict[str, Any], str | None]:
    prefix = f"{condition}__{task.id}__r{repetition}"
    events_path = run_dir / f"{prefix}.jsonl"
    stderr_path = run_dir / f"{prefix}.stderr.log"
    answer_path = run_dir / f"{prefix}.answer.json"
    prompt = (
        "Analyze the repository without modifying it. Use the most appropriate available "
        "code-navigation tools and minimize source text retrieval. Return only the required JSON "
        "object. Give exact file paths and line numbers as evidence. For an MCP condition, make "
        "exactly two semantic navigation calls: first discover the named start symbol with a "
        "simple search term, then make one bounded call trace using the exact identifier returned. "
        "Treat the returned path rows as sufficient evidence and do not make further searches or "
        "fetch symbol bodies. If either call fails or cannot answer the question, use one focused "
        "shell search instead.\n\n"
        "This is an independent benchmark question. Even if a prior turn discussed related "
        "code, perform the two required semantic navigation calls again and do not reuse a "
        "prior conclusion.\n\n"
        f"Question:\n{task.prompt}"
    )
    command = [args.codex, "exec"]
    if session_id:
        command += ["resume", session_id]
    command += [
        "--ignore-user-config",
        "--json",
        "--output-schema",
        str(args.schema),
        "--output-last-message",
        str(answer_path),
        "-m",
        args.model,
        "-c",
        f"model_reasoning_effort={toml_string(args.reasoning)}",
        "-c",
        "approval_policy=\"never\"",
        "--skip-git-repo-check",
    ]
    if not session_id:
        if args.session_mode == "fresh":
            command.append("--ephemeral")
        command += ["-s", args.sandbox, "-C", str(args.target)]
        command += condition_overrides(args, condition)
    command.append(prompt)
    started = time.monotonic()
    with events_path.open("w") as stdout, stderr_path.open("w") as stderr:
        completed = subprocess.run(command, stdout=stdout, stderr=stderr, text=True)
    duration_ms = round((time.monotonic() - started) * 1000)
    events = read_jsonl(events_path)
    answer = answer_path.read_text(errors="replace") if answer_path.exists() else ""
    metrics: dict[str, Any] = {
        "condition": args.legacy_label if condition == "legacy" else condition,
        "task": task.id,
        "repetition": repetition,
        "exit_code": completed.returncode,
        "duration_ms": duration_ms,
        "events": len(events),
        "event_log_bytes": events_path.stat().st_size,
        "answer_bytes": len(answer.encode()),
        "answer_path": str(answer_path),
        "event_log_path": str(events_path),
        "stderr_path": str(stderr_path),
        "model": args.model,
        "reasoning": args.reasoning,
    }
    metrics.update(mcp_call_counts(events))
    metrics.update(extract_usage(events))
    metrics.update(score_answer(answer, task))
    return metrics, thread_id(events)


def median(values: list[int | float | None]) -> float | None:
    valid = [value for value in values if value is not None]
    return float(statistics.median(valid)) if valid else None


def render_number(value: float | int | None, digits: int = 0) -> str:
    if value is None:
        return "n/a"
    return f"{value:.{digits}f}" if digits else f"{value:.0f}"


def performance_gate(
    results: list[dict[str, Any]], conditions: list[str], legacy_label: str, improvement: float
) -> tuple[bool, list[str]]:
    """Return a strict, quality-preserving Connectome win verdict.

    The gate uses medians to reduce outlier influence.  It deliberately requires
    a margin above zero because a one-token or one-millisecond difference is not
    meaningful on a remote model service.
    """
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in results:
        grouped.setdefault(row["condition"], []).append(row)
    connectome = grouped.get("connectome", [])
    baselines = ["native"] + ([legacy_label] if "legacy" in conditions else [])
    issues: list[str] = []
    if not connectome:
        return False, ["Connectome produced no result rows."]
    for label in ["connectome", *baselines]:
        rows = grouped.get(label, [])
        if not rows:
            issues.append(f"`{label}` produced no result rows.")
            continue
        if any(row["exit_code"] != 0 for row in rows):
            issues.append(f"`{label}` has failed Codex runs.")
        if label != "native" and any(row["mcp_navigation_successful_calls"] < 1 for row in rows):
            issues.append(f"`{label}` has a run without successful semantic navigation.")
    for baseline in baselines:
        c_quality = median([row["quality_score"] for row in connectome])
        b_quality = median([row["quality_score"] for row in grouped.get(baseline, [])])
        if c_quality is None or b_quality is None or c_quality < b_quality:
            issues.append(f"Connectome quality is below `{baseline}`.")
        for metric, title in (("total_tokens", "total tokens"), ("duration_ms", "latency")):
            c_value = median([row[metric] for row in connectome])
            b_value = median([row[metric] for row in grouped.get(baseline, [])])
            if c_value is None or b_value is None:
                issues.append(f"Missing {title} telemetry for `{baseline}` comparison.")
            elif c_value > b_value * (1 - improvement):
                issues.append(
                    f"Connectome does not beat `{baseline}` by {improvement:.0%} on median {title}."
                )
    return not issues, issues


def write_report(run_dir: Path, results: list[dict[str, Any]], manifest: dict[str, Any]) -> None:
    (run_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (run_dir / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    columns = list(results[0]) if results else []
    with (run_dir / "results.csv").open("w") as handle:
        handle.write(",".join(json.dumps(column) for column in columns) + "\n")
        for result in results:
            handle.write(",".join(json.dumps(result.get(column)) for column in columns) + "\n")

    grouped: dict[str, list[dict[str, Any]]] = {}
    for result in results:
        grouped.setdefault(result["condition"], []).append(result)
    baseline = median([row["total_tokens"] for row in grouped.get("native", [])])
    preparation_lines: list[str] = []
    index = manifest.get("index")
    if isinstance(index, dict):
        overview = index.get("overview") if isinstance(index.get("overview"), dict) else {}
        lsp = overview.get("lsp") if isinstance(overview.get("lsp"), dict) else {}
        preparation_lines = [
            "## Connectome preparation",
            "",
            f"- Index duration: {render_number(index.get('elapsed_ms'))} ms",
            f"- Indexed files: {render_number(overview.get('files'))}; symbols: {render_number(overview.get('symbols'))}",
            f"- LSP mode: `{lsp.get('mode', manifest.get('lsp_mode'))}`; servers: {', '.join(lsp.get('servers', [])) or 'none'}",
        ]
        warnings = lsp.get("warnings", [])
        if warnings:
            preparation_lines.append(f"- LSP warnings: {'; '.join(warnings)}")
        preparation_lines.append("")
    legacy_index = manifest.get("legacy_index")
    if isinstance(legacy_index, dict):
        preparation_lines += [
            "## codebase-memory preparation",
            "",
            f"- Index duration: {render_number(legacy_index.get('elapsed_ms'))} ms",
            f"- Index mode: `{legacy_index.get('mode', 'unknown')}`",
            "",
        ]
    mcp_validation = manifest.get("mcp_validation", [])
    if mcp_validation:
        preparation_lines += ["## MCP validation", ""]
        for item in mcp_validation:
            missing_runs = item.get("missing_runs", 0)
            status = "passed" if item["successful_calls"] and not missing_runs else "failed"
            preparation_lines.append(
                f"- `{item['condition']}`: {status} ({item['successful_calls']} successful navigation calls; {missing_runs} task(s) without navigation)."
            )
        preparation_lines.append("")
    lines = [
        "# Connectome Codex benchmark",
        "",
        f"Run: `{manifest['started_at']}`  ",
        f"Target: `{manifest['target']}`  ",
        f"Target revision: `{manifest.get('target_revision') or 'not a Git checkout'}`  ",
        f"Model: `{manifest['model']}` with `{manifest['reasoning']}` reasoning  ",
        f"Sandbox: `{manifest['sandbox']}`  ",
        f"Tasks: {manifest['task_count']}; repetitions: {manifest['repetitions']}",
        "",
        *preparation_lines,
        "## Median results",
        "",
        "| Condition | Total tokens | Uncached input | Duration (ms) | MCP calls (ok/failed) | Quality | Successful runs |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for condition, rows in grouped.items():
        tokens = median([row["total_tokens"] for row in rows])
        quality = median([row["quality_score"] for row in rows])
        success = sum(1 for row in rows if row["exit_code"] == 0)
        lines.append(
            "| {condition} | {tokens} | {uncached} | {duration} | {mcp} | {quality} | {success}/{count} |".format(
                condition=condition,
                tokens=render_number(tokens),
                uncached=render_number(median([row["uncached_input_tokens"] for row in rows])),
                duration=render_number(median([row["duration_ms"] for row in rows])),
                mcp=f"{render_number(median([row['mcp_successful_calls'] for row in rows]))}/{render_number(median([row['mcp_failed_calls'] for row in rows]))}",
                quality=render_number(quality, 2),
                success=success,
                count=len(rows),
            )
        )
    lines += ["", "## Comparison with native baseline", ""]
    if baseline is None:
        lines.append("Exact token usage was not found in the Codex JSONL events. Do not substitute event-log bytes for tokens; inspect the raw JSONL and rerun after resolving the telemetry issue.")
    else:
        for condition, rows in grouped.items():
            if condition == "native":
                continue
            value = median([row["total_tokens"] for row in rows])
            if value is not None:
                change = (value / baseline - 1) * 100
                if change <= 0:
                    lines.append(f"- `{condition}`: {-change:.1f}% median total-token reduction relative to native.")
                else:
                    lines.append(f"- `{condition}`: {change:.1f}% median total-token increase relative to native.")
    lines += [
        "",
        "## Quality gate",
        "",
        "Quality combines required-fact coverage with evidence-symbol and evidence-file coverage. `Total tokens` is the Codex usage total; `Uncached input` is reported separately to expose cache effects. Event-log byte size is intentionally not used as a token proxy.",
        "",
        "## Artifacts",
        "",
        "- `manifest.json`: model, target, conditions, and index configuration",
        "- `results.csv` / `results.json`: one row per run",
        "- `*.jsonl`: raw Codex events",
        "- `*.answer.json`: structured agent answer",
        "- `*.stderr.log`: CLI and MCP diagnostics",
    ]
    if manifest.get("require_connectome_win"):
        passed, issues = performance_gate(
            results, manifest["conditions"], manifest["legacy_label"], manifest["minimum_improvement"]
        )
        lines += ["", "## Connectome performance gate", ""]
        if passed:
            lines.append(
                f"**PASSED** — Connectome matched or exceeded baseline quality and beat every baseline by at least {manifest['minimum_improvement']:.0%} on median total tokens and latency."
            )
        else:
            lines.append("**NOT PASSED** — " + " ".join(issues))
    failed = [row for row in results if row["exit_code"] != 0]
    if failed:
        lines += ["", "## Run failures", ""]
        for row in failed:
            lines.append(
                f"- `{row['condition']}` / `{row['task']}` / repetition {row['repetition']} "
                f"exited {row['exit_code']}; inspect `{Path(row['stderr_path']).name}`."
            )
    (run_dir / "report.md").write_text("\n".join(lines) + "\n")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", type=Path, required=True, help="Git repository to analyze")
    parser.add_argument("--tasks", type=Path, default=DEFAULT_TASKS)
    parser.add_argument("--task-id", action="append", default=[], help="Run only this task id; repeatable")
    parser.add_argument("--connectome", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--codex", default="codex")
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument("--model", default="gpt-5.6-luna")
    parser.add_argument("--reasoning", default="high")
    parser.add_argument("--sandbox", choices=["read-only", "workspace-write", "danger-full-access"], default="read-only")
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=1, help="Maximum concurrent Codex runs")
    parser.add_argument("--session-mode", choices=["fresh", "persistent"], default="fresh", help="Use a new Codex session for every task, or reuse one session per condition and repetition")
    parser.add_argument("--conditions", default="native,connectome", help="Comma-separated: native,connectome,legacy")
    parser.add_argument("--legacy-name", default="legacy_code_navigation")
    parser.add_argument("--legacy-label", default="legacy", help="Display name for the legacy condition in reports")
    parser.add_argument("--legacy-command", help="Executable for the optional comparison MCP")
    parser.add_argument("--legacy-index-command", help="Executable used for optional legacy CLI indexing")
    parser.add_argument("--legacy-arg", action="append", default=[], help="Argument for --legacy-command; repeatable")
    parser.add_argument("--legacy-index", choices=["once", "never"], default="never")
    parser.add_argument("--legacy-index-mode", default="moderate")
    parser.add_argument("--require-mcp-success", action="store_true", help="Fail if an enabled MCP condition has no successful tool call")
    parser.add_argument("--require-connectome-win", action="store_true", help="Fail unless Connectome wins every baseline on quality, tokens, and latency")
    parser.add_argument("--minimum-improvement", type=float, default=0.05, help="Required fractional token and latency improvement for the Connectome gate (default: 0.05)")
    parser.add_argument("--lsp-mode", choices=["auto", "on", "off"], default="auto")
    parser.add_argument("--index", choices=["once", "never", "per-connectome-run"], default="once")
    parser.add_argument("--output", type=Path, default=ROOT / "runs")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    args.target = args.target.resolve()
    args.tasks = args.tasks.resolve()
    args.connectome = args.connectome.resolve()
    args.schema = args.schema.resolve()
    args.output = args.output.resolve()
    args.conditions = [item.strip() for item in args.conditions.split(",") if item.strip()]
    invalid = set(args.conditions) - {"native", "connectome", "legacy"}
    if invalid or not args.conditions:
        parser.error("--conditions may contain only native, connectome, legacy")
    if args.repetitions < 1:
        parser.error("--repetitions must be positive")
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if args.session_mode == "persistent" and args.jobs != 1:
        parser.error("--session-mode persistent requires --jobs 1")
    if not 0 < args.minimum_improvement < 1:
        parser.error("--minimum-improvement must be between 0 and 1")
    return args


def main() -> None:
    args = parse_args()
    for path, label in ((args.target, "target"), (args.tasks, "tasks"), (args.schema, "schema")):
        if not path.exists():
            raise SystemExit(f"{label} does not exist: {path}")
    if "connectome" in args.conditions and not args.connectome.is_file():
        raise SystemExit(f"Connectome binary does not exist: {args.connectome}; run cargo build --release")
    tasks = load_tasks(args.tasks)
    if args.task_id:
        wanted = set(args.task_id)
        tasks = [task for task in tasks if task.id in wanted]
        missing = wanted - {task.id for task in tasks}
        if missing:
            raise SystemExit(f"Unknown task id(s): {', '.join(sorted(missing))}")
        if not tasks:
            raise SystemExit("--task-id selected no tasks")
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = args.output / timestamp
    if args.dry_run:
        print(json.dumps({"run_dir": str(run_dir), "conditions": args.conditions, "tasks": [asdict(task) for task in tasks]}, indent=2))
        return
    run_dir.mkdir(parents=True)
    manifest: dict[str, Any] = {
        "started_at": datetime.now(timezone.utc).isoformat(),
        "target": str(args.target),
        "model": args.model,
        "reasoning": args.reasoning,
        "sandbox": args.sandbox,
        "conditions": args.conditions,
        "task_count": len(tasks),
        "repetitions": args.repetitions,
        "session_mode": args.session_mode,
        "index_mode": args.index,
        "lsp_mode": args.lsp_mode,
        "connectome": str(args.connectome),
        "codex": args.codex,
        "legacy_label": args.legacy_label,
        "require_connectome_win": args.require_connectome_win,
        "minimum_improvement": args.minimum_improvement,
        "target_revision": git_revision(args.target),
    }
    if args.index == "once" and "connectome" in args.conditions:
        manifest["index"] = run_index(args, run_dir / "connectome-index.json")
    if args.legacy_index == "once" and "legacy" in args.conditions:
        manifest["legacy_index"] = run_legacy_index(args, run_dir / "legacy-index.json")
    results: list[dict[str, Any]] = []
    for repetition in range(1, args.repetitions + 1):
        # Rotate order to reduce cache/load bias without concealing the order.
        offset = (repetition - 1) % len(args.conditions)
        order = args.conditions[offset:] + args.conditions[:offset]
        work: list[tuple[str, Task]] = []
        for task in tasks:
            for condition in order:
                if args.index == "per-connectome-run" and condition == "connectome":
                    run_index(args, run_dir / f"connectome-index__{task.id}__r{repetition}.json")
                print(f"[{condition}] {task.id} repetition {repetition}", file=sys.stderr, flush=True)
                work.append((condition, task))
        if args.session_mode == "persistent":
            sessions: dict[str, str] = {}
            for condition, task in work:
                result, session = run_one(args, condition, task, repetition, run_dir, sessions.get(condition))
                if session:
                    sessions[condition] = session
                results.append(result)
                (run_dir / "results.partial.json").write_text(json.dumps(results, indent=2) + "\n")
        else:
            with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
                futures = [
                    executor.submit(run_one, args, condition, task, repetition, run_dir)
                    for condition, task in work
                ]
                for future in futures:
                    result, _ = future.result()
                    results.append(result)
                    (run_dir / "results.partial.json").write_text(json.dumps(results, indent=2) + "\n")
    validation: list[dict[str, Any]] = []
    if args.require_mcp_success:
        for condition in args.conditions:
            if condition == "native":
                continue
            label = args.legacy_label if condition == "legacy" else condition
            condition_rows = [row for row in results if row["condition"] == label]
            validation.append(
                {
                    "condition": label,
                    "successful_calls": sum(row["mcp_navigation_successful_calls"] for row in condition_rows),
                    "missing_runs": sum(
                        row["mcp_navigation_successful_calls"] < 1 for row in condition_rows
                    ),
                }
            )
    manifest["mcp_validation"] = validation
    write_report(run_dir, results, manifest)
    print(run_dir / "report.md")
    failures = [row for row in results if row["exit_code"] != 0]
    missing_mcp = [item for item in validation if not item["successful_calls"] or item.get("missing_runs")]
    if missing_mcp:
        labels = ", ".join(item["condition"] for item in missing_mcp)
        print(f"No successful MCP calls for: {labels}; benchmark is invalid.", file=sys.stderr)
        failures.extend(missing_mcp)
    if args.require_connectome_win:
        passed, issues = performance_gate(results, args.conditions, args.legacy_label, args.minimum_improvement)
        if not passed:
            print("Connectome performance gate not passed: " + " ".join(issues), file=sys.stderr)
            failures.append({"gate": "connectome-win"})
    if failures:
        print(f"{len(failures)} benchmark runs failed; see {run_dir / 'report.md'}", file=sys.stderr)
        raise SystemExit(2)


if __name__ == "__main__":
    main()
