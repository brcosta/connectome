#!/usr/bin/env bash
set -euo pipefail

benchmark_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "$benchmark_dir/.." && pwd)"
run_stamp="$(date -u +%Y%m%dT%H%M%SZ)"
output_root="$benchmark_dir/runs/persistent/$run_stamp"

"$benchmark_dir/prepare-fixtures.py"
cargo build --release --manifest-path "$repository_root/Cargo.toml"

common=(
  --conditions native,connectome
  --connectome "$repository_root/target/release/connectome"
  --session-mode persistent
  --repetitions "${BENCHMARK_REPETITIONS:-3}"
  --jobs 1
  --require-mcp-success
  --require-connectome-win
)

for fixture in algorithms-javascript algorithms-typescript; do
  echo "Benchmarking $fixture persistent session" >&2
  "$benchmark_dir/run.sh" \
    --target "$benchmark_dir/fixtures/slices/$fixture" \
    --tasks "$benchmark_dir/tasks/$fixture.jsonl" \
    --output "$output_root/$fixture" \
    "${common[@]}" "$@"
done

echo "$output_root/algorithms-javascript"/*/report.md
echo "$output_root/algorithms-typescript"/*/report.md
