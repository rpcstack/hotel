#!/usr/bin/env bash
# Static checks: formatting, rpcstack pin consistency, a single Tokio, and
# warning-free compilation in every policy mode (baseline, masa, policy).
#
# Modes are built one at a time with explicit packages, never `--workspace`
# with one mode's feature: `hotel-policy` (the policy/ crate) turns on Masa
# features of its own, and Cargo would unify them into a baseline build.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== rustfmt"
cargo fmt --all -- --check

echo "== rpcstack pin"
revs=$(grep -o 'rpcstack/rpcstack", rev = "[0-9a-f]*"' Cargo.toml | sort -u | wc -l)
if [ "$revs" -ne 1 ]; then
  echo "Cargo.toml pins rpcstack at more than one rev:"; grep -n 'rev = ' Cargo.toml; exit 1
fi
lock_revs=$(grep -o 'git+https://github.com/rpcstack/rpcstack?rev=[0-9a-f]*' Cargo.lock | sort -u | wc -l)
if [ "$lock_revs" -ne 1 ]; then echo "Cargo.lock resolves more than one rpcstack rev"; exit 1; fi

echo "== single tokio / hyper"
for crate in tokio hyper; do
  n=$(cargo tree -p hotel --features policy -i "$crate" --prefix none -e normal 2>/dev/null | grep -c "^$crate v" || true)
  if [ "$n" -ne 1 ]; then echo "expected one $crate in the graph, found $n"; cargo tree -d; exit 1; fi
done
if cargo tree -p hotel --features policy -d -e normal | grep -E '^(tokio|hyper) v'; then
  echo "duplicate tokio/hyper versions"; exit 1
fi

for features in baseline masa; do
  echo "== cargo check ($features)"
  RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check --locked -p hotel -p app-utils --all-targets --features "hotel/$features"
done
echo "== cargo check (policy, with the policy crate)"
RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check --locked -p hotel -p app-utils -p hotel-policy --all-targets --features hotel/policy
