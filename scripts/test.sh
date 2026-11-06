#!/usr/bin/env bash
# Unit and regression tests in every policy mode. One mode at a time with
# explicit packages: see the note in scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
for features in baseline masa; do
  echo "== cargo test ($features)"
  cargo test --locked -p hotel -p app-utils --features "hotel/$features"
done
echo "== cargo test (policy, with the policy crate)"
cargo test --locked -p hotel -p app-utils -p hotel-policy --features hotel/policy
