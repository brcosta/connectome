# Automated Codex benchmark

This harness compares a clean Codex CLI baseline with Connectome, and can also
include another MCP server. Every task is a new non-interactive Codex session;
the runner ignores normal user configuration, uses `gpt-5.6-luna` with `high`
reasoning, and captures raw JSONL events plus a structured final answer.

## Quick start

Build Connectome first:

```bash
cargo build --release
```

## Standard six-language suite

The standard suite uses pinned shallow clones of Spring Boot, Kit, Flutter,
TheAlgorithms/JavaScript, and TheAlgorithms/TypeScript, plus a fixed Rust slice
from Connectome itself. It copies only the selected source files into
`benchmark/fixtures/slices/`.
It compares only the native baseline with Connectome, using the included
repository-specific task packs. The first invocation downloads the public
repositories into ignored `benchmark/fixtures/` directories; later invocations
verify their commit and reuse them. Connectome's generated `.connectome` index
is intentionally kept in the slices and does not make a fixture invalid.

```bash
./benchmark/standard.sh
```

The default is three serialized repetitions (twenty-four fresh Luna-high runs
for the original Java/Clojure tasks plus twenty-four for Dart, JavaScript,
TypeScript, and Rust). Serial execution avoids one condition competing with another for the
same Codex service or local MCP process. Set `BENCHMARK_REPETITIONS=1` for a
smoke test:

```bash
BENCHMARK_REPETITIONS=1 ./benchmark/standard.sh
```

This produces one detailed report per repository beneath
`benchmark/runs/standard/`. The pinned fixture commits and source URLs are in
[`fixtures.json`](fixtures.json). Do not use `--conditions` with this command:
the standard suite intentionally measures only native versus Connectome.

## Three-way suite with codebase-memory

Use the separate runner to compare all three isolated conditions: native Codex,
codebase-memory, and Connectome. It discovers the locally registered
`codebase-memory-mcp` command automatically, or you can specify it explicitly.

```bash
./benchmark/standard-with-codememory.sh
```

If it is not registered in this Codex CLI installation:

```bash
./benchmark/standard-with-codememory.sh \
  --codebase-memory-command /absolute/path/to/codebase-memory-mcp
```

The default is three serialized repetitions (thirty-six Luna-high runs). Set
`BENCHMARK_REPETITIONS=1` for a smoke test. The report calls the third condition
`codebase-memory`, rather than the internal `legacy` configuration name. Both
Connectome and codebase-memory are indexed once before their agent runs, so
warm task execution—not index creation—is compared.

The three-way runner keeps all conditions in the same `read-only` sandbox.
codebase-memory 0.10.5 incorrectly marks analysis-profile graph tools as
destructive, so the runner uses a transparent stdio adapter that corrects only
those tool annotations; requests and tool results are otherwise unchanged.

For a fast MCP smoke test, select one task and serialize the two conditions:

```bash
./benchmark/run.sh \
  --target benchmark/fixtures/slices/spring-boot \
  --tasks benchmark/tasks/spring-boot.jsonl \
  --conditions native,connectome \
  --task-id spring-application-run \
  --repetitions 1 \
  --jobs 1 \
  --require-mcp-success
```

The same optimized test is available as:

```bash
./benchmark/fast.sh
```

It uses the cross-file Kit generator task, which is designed to benefit from a
single bounded semantic call graph query. Set `BENCHMARK_REPETITIONS=3` for a
stable median.

Copy and tailor the example tasks for the repository under test. The
`required_facts` must be independently verified facts; the score is simply the
fraction of those exact facts represented in the returned answer.

```bash
cp benchmark/tasks.example.jsonl /tmp/my-tasks.jsonl
./benchmark/run.sh --target /absolute/path/to/java-clojure-repository --tasks /tmp/my-tasks.jsonl
```

The command runs `native` and `connectome` three times per task. It creates a
time-stamped directory under `benchmark/runs/` and prints the path to its human
readable `report.md`.

Connectome is indexed once before warm runs. This writes the usual
`.connectome/index.bin` to the target repository. Use `--index never` if an
index is already ready, or `--index per-connectome-run` to include repeated
incremental indexing in the measurement.

## Compare an existing MCP server

Pass its executable and each argument separately. This condition is optional,
so the normal quick start remains a clean with/without-Connectome comparison.

```bash
./benchmark/run.sh \
  --target /absolute/path/to/repository \
  --tasks /tmp/my-tasks.jsonl \
  --conditions native,legacy,connectome \
  --legacy-name existing_navigation \
  --legacy-command node \
  --legacy-arg /absolute/path/to/legacy-server/dist/index.js
```

The runner never enables your desktop-configured MCP servers: `--ignore-user-config`
means only the condition's server is present. It also uses `--ephemeral`, so a
previous response cannot affect a later result.

Codex CLI must be able to write its normal local state under its configured
Codex home directory. If an external sandbox denies that access, the runner
still writes diagnostics and exits non-zero; run it from your usual terminal
instead of the restricted sandbox.

## Artifacts and interpretation

Each run produces:

- `report.md`: median comparison and quality gate.
- `results.csv` and `results.json`: one result per condition/task/repetition.
- `*.jsonl`: raw Codex CLI events, retained for audit.
- `*.answer.json`: machine-shaped final answer.
- `*.stderr.log`: Codex and MCP diagnostics.

Any failed agent run makes the command exit non-zero after preserving the full
report. A result with failures is not a valid benchmark.

`standard-with-codememory.sh` also exits non-zero unless Connectome: has equal
or higher median quality, completes semantic navigation in every MCP run, and
beats both baselines by at least 5% on median total tokens and median latency.
This is intentionally strict: the script is a performance gate, not a report
generator that can turn a marginal run into a win.

The runner extracts exact token fields when this Codex CLI exposes them in its
JSONL events. If the event schema lacks usage, the report says so explicitly;
event-log bytes and duration remain available but are not labelled as tokens.

Run a small pilot first:

```bash
./benchmark/run.sh --target /absolute/path/to/repository --tasks /tmp/my-tasks.jsonl --repetitions 1
```

Then use three to five repetitions. Treat a token reduction as valid only when
the structured answer has equal-or-better quality than the native baseline.

## Options

```text
--model gpt-5.6-luna       Default and recommended benchmark model
--reasoning high           Default and recommended reasoning effort
--conditions native,connectome,legacy
--index once|never|per-connectome-run
--lsp-mode auto|on|off
--sandbox read-only|workspace-write|danger-full-access
--require-connectome-win  Require quality-preserving wins on tokens and latency
--minimum-improvement 0.05 Required fractional margin for that gate
--output /path/to/results
--dry-run                  Validate task/configuration parsing without calling Codex
```
