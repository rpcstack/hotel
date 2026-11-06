#!/usr/bin/env bash
# Unit and regression tests in every policy mode. One mode at a time with
# explicit packages: see the note in scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "== cargo test (baseline)"
cargo test --locked -p hotel -p app-utils --features hotel/baseline
echo "== cargo test (policy, with the policy crate)"
cargo test --locked -p hotel -p app-utils -p hotel-policy --features hotel/policy
