#!/usr/bin/env python3
"""Tiny stdio LSP responder used for the integration smoke test."""
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


while True:
    message = read_message()
    if message is None:
        break
    if message.get("method") == "initialize":
        reply(message, {"capabilities": {"definitionProvider": True, "callHierarchyProvider": True}})
    elif message.get("method") == "textDocument/definition":
        uri = message["params"]["textDocument"]["uri"]
        if uri.endswith(".clj"):
            target = "file://" + os.path.abspath("tests/fixtures/clojure/dev/connectome/core.clj")
            line = 2
        else:
            target = "file://" + os.path.abspath("tests/fixtures/java/dev/connectome/Greeter.java")
            line = 7
        reply(message, [{"uri": target, "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}}}])
    elif message.get("method") == "textDocument/prepareCallHierarchy":
        params = message["params"]
        reply(message, [{"name": "caller", "kind": 12, "uri": params["textDocument"]["uri"], "range": {"start": params["position"], "end": params["position"]}, "selectionRange": {"start": params["position"], "end": params["position"]}}])
    elif message.get("method") == "callHierarchy/outgoingCalls":
        item = message["params"]["item"]
        if item["uri"].endswith(".clj"):
            target = "file://" + os.path.abspath("tests/fixtures/clojure/dev/connectome/core.clj")
            line = 2
        else:
            target = "file://" + os.path.abspath("tests/fixtures/java/dev/connectome/Greeter.java")
            line = 7
        reply(message, [{"to": {"name": "target", "kind": 12, "uri": target, "range": {"start": {"line": line, "character": 0}, "end": {"line": line, "character": 1}}}, "fromRanges": [{"start": item["range"]["start"], "end": item["range"]["start"]}]}])
