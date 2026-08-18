# Connectome

<p align="center">
  <strong>Fast semantic navigation for Java and Clojure coding agents.</strong><br />
  A compact local MCP that replaces broad repository search with precise symbols, source ranges, and bounded call paths.
</p>

<p align="center">
  <a href="https://github.com/brcosta/connectome/actions/workflows/ci.yml"><img src="https://github.com/brcosta/connectome/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
  <a href="https://github.com/brcosta/connectome/releases"><img src="https://img.shields.io/github/v/release/brcosta/connectome?display_name=tag" alt="Latest release" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT license" /></a>
  <img src="https://img.shields.io/badge/languages-Java%20%7C%20Clojure-6f42c1" alt="Supported languages" />
</p>

> **Search less. Navigate with evidence. Spend fewer tokens.**

Connectome is a small, local code-intelligence MCP server built for coding
agents. It turns Java and Clojure repositories into a compact semantic index,
then answers navigation questions with the smallest useful payload.

`mcp` · `code-intelligence` · `java` · `clojure` · `tree-sitter` · `jdtls` · `clojure-lsp` · `coding-agents`

## Why Connectome

- **Token-efficient by design** — discovery returns names, signatures, and precise locations; source is retrieved only when needed.
- **Fast local navigation** — one compact snapshot, in-memory lookups, bounded traversals, no database or background daemon.
- **Semantic when available, resilient when not** — JDT LS and clojure-lsp enrich definitions and call paths; Tree-sitter remains a dependable fallback.
- **Correctly handles real code structure** — line-qualified Java overload selectors and Clojure multimethod dispatch preserve meaningful navigation paths.
- **Agent-native MCP tools** — search symbols, trace callers/callees, fetch exact source ranges, and inspect the index without broad shell search.
- **Release-ready** — CI verifies format, linting, tests, and release builds across Linux, macOS, and Windows.

## What agents can ask

| Need | Connectome tool | Response shape |
| --- | --- | --- |
| Find a definition | `search_symbols` | compact name, kind, file:line, signature |
| Select an overload | `search_symbols` → `trace_calls` | line-qualified symbol selector |
| Understand a path | `trace_calls` | bounded, source-lined call edges |
| Read evidence | `get_symbol` | exact source range only |
| Inspect scope | `get_overview` | language, symbol, call, and LSP counts |

## Quick start

```bash
git clone https://github.com/brcosta/connectome.git
cd connectome
cargo build --release

# Build a semantic index for a repository
./target/release/connectome index /path/to/repository

# Start the local stdio MCP server
./target/release/connectome serve
```

The first release focuses on the operations that replace the most expensive file-by-file exploration:

- `index_repository` — parallel Tree-sitter indexing into `.connectome/index.bin`, reusing unchanged file fragments
- `search_symbols` — ranked symbol names, locations, and compact signatures
- `get_symbol` — the exact source range for one symbol
- `trace_calls` — bounded inbound or outbound call traversal
- `get_overview` — language, symbol, and call counts

LSPs are first-class semantic providers with a safe fallback. The default `auto` mode discovers `jdtls` and `clojure-lsp` on `PATH` (or uses `CONNECTOME_JDTLS_COMMAND` / `CONNECTOME_CLOJURE_LSP_COMMAND`), starts them over stdio, opens indexed documents, and uses `textDocument/definition` plus `callHierarchy/outgoingCalls` when advertised. If a server is unavailable, lacks a capability, or times out, the Tree-sitter/name-based index remains usable.

```bash
./target/release/connectome index /path/to/repository \
  --jdtls-command 'jdtls -data .connectome/jdtls' \
  --clojure-lsp-command 'clojure-lsp listen'
```

The same fields are available on the MCP `index_repository` tool as `jdtls_command`, `clojure_lsp_command`, and `lsp_timeout_ms`. Commands run from the repository root through `sh -c`; JDT LS normally needs a separate `-data` directory per repository. Use `--lsp-mode off` (or MCP `lsp_mode: "off"`) for parser-only indexing, or `--lsp-mode on` to request semantic providers and report missing-server warnings while retaining the fallback.

It intentionally does not include a daemon, UI, embeddings, Cypher, infrastructure indexing, or dozens of language grammars. The graph is loaded directly from one binary snapshot, so queries are in-memory scans or adjacency traversals with no service dependency.

## Build and use

```bash
cargo build --release
./target/release/connectome index /path/to/repository
./target/release/connectome search greet --path /path/to/repository
./target/release/connectome overview /path/to/repository
```

Running `connectome` with no subcommand starts an MCP server over standard input/output. Example client configuration:

```json
{
  "mcpServers": {
    "connectome": {
      "command": "/absolute/path/to/connectome"
    }
  }
}
```

Every query tool accepts an optional `path`. If omitted, it uses the MCP process working directory. Indexes live inside the target repository and should normally remain uncommitted.

## Install in coding agents

Build the release binary first and substitute its absolute path below:

```bash
cargo build --release
CONNECTOME_BIN="$(pwd)/target/release/connectome"
```

### Codex

Add Connectome to your Codex user configuration, then verify that it is registered:

```bash
codex mcp add connectome -- "$CONNECTOME_BIN"
codex mcp get connectome
```

To make the server project-specific instead, add the following to `.codex/config.toml`:

```toml
[mcp_servers.connectome]
command = "/absolute/path/to/connectome"
cwd = "/absolute/path/to/repository-to-analyze"
```

### Claude Code

Install it for the current project (shared through `.mcp.json`) or for your user account:

```bash
claude mcp add --scope project connectome -- "$CONNECTOME_BIN"
# Or: claude mcp add --scope user connectome -- "$CONNECTOME_BIN"
claude mcp get connectome
```

Both clients start Connectome as a local stdio MCP server. Use the client's
normal MCP view (`/mcp` in Claude Code) to confirm the server is connected.

## Releases and versioning

Connectome follows semantic versioning. The package version in `Cargo.toml`
and the release tag must agree: `v0.1.0` corresponds to version `0.1.0`.

- Every push and pull request runs formatting, Clippy, tests, and a release build.
- Pushing a tag matching `v*` validates that version and creates a GitHub Release.
- Releases include Linux, macOS (Intel and Apple Silicon), and Windows archives with SHA-256 checksums.

To publish a release after updating `Cargo.toml` and `CHANGELOG.md`:

```bash
git tag -a v0.1.0 -m "Connectome v0.1.0"
git push origin v0.1.0
```

## Design for low token use

Search responses use four fields only: qualified name, kind, `file:line`, and a whitespace-compacted signature. Source is returned only by `get_symbol`. All listing and traversal tools have conservative default limits and hard caps. This makes the cheap discovery response the default and turns source retrieval into an explicit second step.

The language boundary is isolated under `src/languages/`. Adding another Tree-sitter grammar requires extension detection plus an extractor that emits the language-neutral `LocalSymbol` and `LocalCall` records; storage, resolution, MCP, and queries remain unchanged. LSP discovery, transport, capability checks, and fallback behavior are isolated under `src/lsp.rs`.

## Current resolution limits

Without LSP, call resolution is intentionally conservative. A call resolves when its name has one indexed definition, or when one candidate is in the caller's file. With JDT LS or clojure-lsp enabled, Connectome asks the server for definitions at call positions and enriches the graph with outgoing call-hierarchy edges. This improves Java overload/type resolution and Clojure namespace/alias resolution where the server supports them. Unresolved call sites remain in the index and are included in overview counts, but are omitted from graph traversal.

## Performance checks

The semantic strategy is covered by an integration test with a deterministic local LSP. It exercises ambiguous Java overloads, duplicate Clojure definitions resolved through a namespace alias, warm incremental reuse, and unavailable-server fallback:

```bash
cargo test --all-targets
```

The test lives in [tests/semantic_resolution.rs](tests/semantic_resolution.rs); its fake server is [tests/semantic_fake_lsp.py](tests/semantic_fake_lsp.py). Real JDT LS and clojure-lsp are intentionally not required for CI.

Use the shell's `time` command around a release build index operation, then inspect the emitted `index_ms` and snapshot size:

```bash
time ./target/release/connectome index /path/to/repository
du -h /path/to/repository/.connectome/index.bin
```

The indexer already skips parsing files whose size and nanosecond modification time match the previous snapshot. The acceptance target for the next milestone is to benchmark cold full indexing, warm incremental indexing, snapshot size, query p50/p95, and serialized response bytes on representative Java and Clojure repositories.
