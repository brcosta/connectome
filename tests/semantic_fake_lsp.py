#!/usr/bin/env python3
"""Deterministic LSP used by semantic_resolution.rs; no external LSP is required."""
import json
import os
import sys


def read_message():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    return json.loads(sys.stdin.buffer.read(length))


def reply(message, result):
    body = json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


def location(uri, line):
    return {"uri": uri, "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}}}


def file_uri(path):
    path = path.replace("\\", "/")
    if path.startswith("//?/"):
        path = path[4:]
    return "file://" + path if path.startswith("/") else "file:///" + path


def target_for(uri, line):
    root = os.getcwd()
    if "/java/" in uri:
        target = file_uri(os.path.join(root, "java/app/Formatter.java"))
        return target, (1 if line == 4 else 5)
    target = file_uri(os.path.join(root, "clojure/app/impl.clj"))
    return target, 1


while True:
    message = read_message()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize":
        reply(message, {"capabilities": {"definitionProvider": True, "callHierarchyProvider": True}})
    elif method == "textDocument/definition":
        params = message["params"]
        uri = params["textDocument"]["uri"]
        if uri.endswith("other.clj"):
            reply(message, [])
            continue
        target, line = target_for(uri, params["position"]["line"])
        reply(message, [location(target, line)])
    elif method == "textDocument/prepareCallHierarchy":
        params = message["params"]
        position = params["position"]
        reply(message, [{"name": "caller", "kind": 12, "uri": params["textDocument"]["uri"], "range": {"start": position, "end": position}, "selectionRange": {"start": position, "end": position}}])
    elif method == "callHierarchy/outgoingCalls":
        item = message["params"]["item"]
        target, line = target_for(item["uri"], item["range"]["start"]["line"])
        reply(message, [{"to": {"name": "target", "kind": 12, "uri": target, "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}}}, "fromRanges": [{"start": item["range"]["start"], "end": item["range"]["start"]}]}])
