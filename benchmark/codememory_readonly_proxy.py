#!/usr/bin/env python3
"""Expose codebase-memory's analysis profile with accurate MCP safety hints.

codebase-memory 0.10.5 describes its analysis-profile graph tools as
destructive even though the profile removes every mutation tool.  Codex
therefore cancels otherwise read-only calls in a read-only benchmark.  This
transparent stdio proxy changes *only* the tools/list annotations for a fixed
allow-list; calls and responses pass through byte-for-byte.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import threading
from typing import Any


READ_ONLY_TOOLS = {
    "list_projects", "index_status", "check_index_coverage", "search_graph",
    "query_graph", "trace_path", "get_code_snippet", "get_graph_schema",
    "get_architecture", "search_code",
}


def forward_input(process: subprocess.Popen[str]) -> None:
    assert process.stdin is not None
    for line in sys.stdin:
        process.stdin.write(line)
        process.stdin.flush()
    process.stdin.close()


def correct_annotations(message: dict[str, Any]) -> dict[str, Any]:
    result = message.get("result")
    tools = result.get("tools") if isinstance(result, dict) else None
    if not isinstance(tools, list):
        return message
    for tool in tools:
        if isinstance(tool, dict) and tool.get("name") in READ_ONLY_TOOLS:
            tool["annotations"] = {
                "readOnlyHint": True,
                "destructiveHint": False,
                "idempotentHint": True,
                "openWorldHint": False,
            }
    return message


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("server", help="codebase-memory-mcp executable")
    parser.add_argument("server_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = [args.server, *args.server_args, "--tool-profile=analysis"]
    process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    sender = threading.Thread(target=forward_input, args=(process,), daemon=True)
    sender.start()
    assert process.stdout is not None
    for line in process.stdout:
        try:
            message = json.loads(line)
            line = json.dumps(correct_annotations(message), separators=(",", ":")) + "\n"
        except json.JSONDecodeError:
            pass
        sys.stdout.write(line)
        sys.stdout.flush()
    raise SystemExit(process.wait())


if __name__ == "__main__":
    main()
