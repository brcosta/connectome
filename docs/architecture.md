# Architecture

The pipeline has five narrow stages:

1. `ignore` walks Git-aware Java and Clojure source paths.
2. Rayon parses files in parallel with the matching Tree-sitter adapter.
3. Unchanged files reuse their prior fragments; adapters emit symbols and unresolved call names for changed files.
4. The indexer assigns dense integer IDs and conservatively resolves calls.
5. Bincode atomically publishes one snapshot that query processes load into memory.

The semantic provider is a first-class optional sixth step. In `auto` mode it discovers installed servers; `on` accepts explicit commands or environment variables and records missing-server warnings; `off` skips all process work. A provider receives `initialize` and `textDocument/didOpen`, then contributes `textDocument/definition` targets and, when supported, `callHierarchy/outgoingCalls` edges. The result updates the same dense call edges; no LSP dependency is required for the baseline index.

Dense IDs keep edges small and make traversal array-friendly. Files and symbols are sorted deterministically before IDs are assigned. The persisted model is versioned so its implementation can later move behind a `GraphStore` abstraction without changing MCP contracts.

## Planned evolution

- Optional content hashes for filesystems whose modification times are unreliable.
- Java import/type/overload resolution and Clojure namespace alias resolution.
- Precomputed name and inbound/outbound adjacency indexes in the snapshot.
- Optional language features that compile parsers into the same binary.
- Optional workspace-symbol and call-hierarchy queries over the same LSP transport.
- A benchmark corpus and response-byte regression budget.

The project should resist broad features until measurements show that they reduce end-to-end agent tokens or latency for Java/Clojure work.
