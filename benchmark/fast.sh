#!/usr/bin/env bash
set -euo pipefail

benchmark_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "$benchmark_dir/.." && pwd)"

exec "$benchmark_dir/run.py" \
  --target "$benchmark_dir/fixtures/slices/kit" \
  --tasks "$benchmark_dir/tasks/kit.jsonl" \
  --conditions native,connectome \
  --task-id kit-generator-actions \
  --connectome "$repository_root/target/release/connectome" \
  --repetitions "${BENCHMARK_REPETITIONS:-1}" \
  --jobs 1 \
  --require-mcp-success \
  "$@"
