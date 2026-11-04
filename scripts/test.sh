#!/usr/bin/env bash
# Unit and regression tests in both policy modes.
set -euo pipefail
cd "$(dirname "$0")/.."
for features in baseline masa; do
  echo "== cargo test ($features)"
  cargo test --locked --workspace --features "hotel/$features"
done
