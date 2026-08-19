#!/usr/bin/env bash
set -euo pipefail

benchmark_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "$benchmark_dir/.." && pwd)"
run_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
output_root="$benchmark_dir/runs/persistent-with-codememory/$run_stamp"
codememory_command="${CODEMEMORY_COMMAND:-/Users/bruno/.local/bin/codebase-memory-mcp}"

[[ -x "$codememory_command" ]] || { echo "codebase-memory MCP not executable: $codememory_command" >&2; exit 69; }

"$benchmark_dir/prepare-fixtures.py"
cargo build --release --manifest-path "$repository_root/Cargo.toml"

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
  --connectome "$repository_root/target/release/connectome"
  --session-mode fresh
  --repetitions "${BENCHMARK_REPETITIONS:-3}"
  --jobs 1
  --require-mcp-success
  --require-connectome-win
)

for fixture in algorithms-javascript algorithms-typescript; do
  "$benchmark_dir/run.sh" \
    --target "$benchmark_dir/fixtures/slices/$fixture" \
    --tasks "$benchmark_dir/tasks/$fixture.jsonl" \
    --output "$output_root/$fixture" \
    "${common[@]}" "$@"
done
