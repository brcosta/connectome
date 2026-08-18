#!/usr/bin/env bash
set -euo pipefail

benchmark_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "$benchmark_dir/.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
output_root="$benchmark_dir/runs/standard-with-codememory/$timestamp"
codememory_command="${CODEMEMORY_COMMAND:-}"

if [[ "${1:-}" == "--codebase-memory-command" ]]; then
  [[ $# -ge 2 ]] || { echo "--codebase-memory-command needs a value" >&2; exit 64; }
  codememory_command="$2"
  shift 2
fi

for option in "$@"; do
  case "$option" in
    --conditions|--conditions=*|--target|--tasks|--connectome|--output|--output=*)
      echo "This runner fixes fixtures, targets, output, and all three conditions." >&2
      exit 64
      ;;
  esac
done

if [[ -z "$codememory_command" ]] && command -v codex >/dev/null 2>&1; then
  codememory_command="$(codex mcp get codebase-memory-mcp 2>/dev/null | sed -n 's/^  command: //p' | head -n 1)"
fi
if [[ -z "$codememory_command" || ! -x "$codememory_command" ]]; then
  echo "Could not find the codebase-memory MCP executable." >&2
  echo "Set CODEMEMORY_COMMAND or pass --codebase-memory-command /absolute/path/to/codebase-memory-mcp." >&2
  exit 69
fi

"$benchmark_dir/prepare-fixtures.py"
cargo build --release --manifest-path "$repository_root/Cargo.toml"

if ! command -v jdtls >/dev/null 2>&1; then
  echo "Warning: jdtls is not on PATH; Spring Boot uses Connectome's parser fallback." >&2
fi
if ! command -v clojure-lsp >/dev/null 2>&1; then
  echo "Warning: clojure-lsp is not on PATH; Kit uses Connectome's parser fallback." >&2
fi

common=(
  --conditions native,legacy,connectome
  --legacy-label codebase-memory
  --legacy-name codebase_memory
  --legacy-command python3
  --legacy-arg "$benchmark_dir/codememory_readonly_proxy.py"
  --legacy-arg "$codememory_command"
  --legacy-index-command "$codememory_command"
  --legacy-index once
  --legacy-index-mode moderate
  --sandbox read-only
  --require-mcp-success
  --require-connectome-win
  --connectome "$repository_root/target/release/connectome"
  --repetitions "${BENCHMARK_REPETITIONS:-3}"
  --jobs 1
)

echo "Benchmarking Spring Boot slice" >&2
"$benchmark_dir/run.sh" \
  --target "$benchmark_dir/fixtures/slices/spring-boot" \
  --tasks "$benchmark_dir/tasks/spring-boot.jsonl" \
  --output "$output_root/spring-boot" \
  "${common[@]}" "$@"

echo "Benchmarking Kit slice" >&2
"$benchmark_dir/run.sh" \
  --target "$benchmark_dir/fixtures/slices/kit" \
  --tasks "$benchmark_dir/tasks/kit.jsonl" \
  --output "$output_root/kit" \
  "${common[@]}" "$@"

echo "Reports:" >&2
if [[ " $* " == *" --dry-run "* ]]; then
  echo "Dry run completed; no reports were created."
else
  echo "$output_root/spring-boot"/*/report.md
  echo "$output_root/kit"/*/report.md
fi
