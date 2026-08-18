#!/usr/bin/env bash
set -euo pipefail

benchmark_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "$benchmark_dir/.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
output_root="$benchmark_dir/runs/standard/$timestamp"

for option in "$@"; do
  case "$option" in
    --conditions|--conditions=*|--target|--tasks|--connectome|--output|--output=*)
      echo "standard.sh fixes fixtures, targets, output, and conditions to native,connectome" >&2
      exit 64
      ;;
  esac
done

"$benchmark_dir/prepare-fixtures.py"
cargo build --release --manifest-path "$repository_root/Cargo.toml"

if ! command -v jdtls >/dev/null 2>&1; then
  echo "Warning: jdtls is not on PATH; Spring Boot uses Connectome's parser fallback." >&2
fi
if ! command -v clojure-lsp >/dev/null 2>&1; then
  echo "Warning: clojure-lsp is not on PATH; Kit uses Connectome's parser fallback." >&2
fi

common=(
  --conditions native,connectome
  --connectome "$repository_root/target/release/connectome"
  --repetitions "${BENCHMARK_REPETITIONS:-3}"
  --jobs 1
)

echo "Benchmarking Spring Boot" >&2
"$benchmark_dir/run.sh" \
  --target "$benchmark_dir/fixtures/slices/spring-boot" \
  --tasks "$benchmark_dir/tasks/spring-boot.jsonl" \
  --output "$output_root/spring-boot" \
  "${common[@]}" "$@"

echo "Benchmarking Kit" >&2
"$benchmark_dir/run.sh" \
  --target "$benchmark_dir/fixtures/slices/kit" \
  --tasks "$benchmark_dir/tasks/kit.jsonl" \
  --output "$output_root/kit" \
  "${common[@]}" "$@"

for fixture in flutter-view algorithms-javascript algorithms-typescript connectome-rust; do
  echo "Benchmarking $fixture slice" >&2
  "$benchmark_dir/run.sh" \
    --target "$benchmark_dir/fixtures/slices/$fixture" \
    --tasks "$benchmark_dir/tasks/$fixture.jsonl" \
    --output "$output_root/$fixture" \
    "${common[@]}" "$@"
done

echo "Reports:" >&2
if [[ " $* " == *" --dry-run "* ]]; then
  echo "Dry run completed; no reports were created."
else
  echo "$output_root/spring-boot"/*/report.md
  echo "$output_root/kit"/*/report.md
  echo "$output_root/flutter-view"/*/report.md
  echo "$output_root/algorithms-javascript"/*/report.md
  echo "$output_root/algorithms-typescript"/*/report.md
  echo "$output_root/connectome-rust"/*/report.md
fi
