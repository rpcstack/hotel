#!/usr/bin/env bash
# Static checks: formatting, rpcstack pin consistency (Cargo.toml, Cargo.lock
# and the rpcstack/ submodule), a single Tokio, and
# warning-free compilation in every policy mode (baseline, policy).
#
# Modes are built one at a time with explicit packages, never `--workspace`
# with one mode's feature: `hotel-policy` (the policy/ crate) turns on rpcstack
# features (sched_custom, priority passing) that Cargo would unify into a
# baseline build.
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

echo "== rpcstack submodule is at the pinned rev"
rev=$(grep -o 'rpcstack/rpcstack", rev = "[0-9a-f]*"' Cargo.toml | sort -u | sed 's/.*rev = "\(.*\)"/\1/')
sub=$(git ls-files -s rpcstack | awk '$1 == "160000" {print $2}')
if [ -z "$sub" ]; then echo "rpcstack/ is not a submodule (git submodule add ... rpcstack)"; exit 1; fi
if [ "$sub" != "$rev" ]; then
  echo "rpcstack submodule is at $sub but Cargo.toml pins $rev: update one to match the other"; exit 1
fi

echo "== single tokio / hyper"
for crate in tokio hyper; do
  n=$(cargo tree -p hotel --features policy -i "$crate" --prefix none -e normal 2>/dev/null | grep -c "^$crate v" || true)
  if [ "$n" -ne 1 ]; then echo "expected one $crate in the graph, found $n"; cargo tree -d; exit 1; fi
done
if cargo tree -p hotel --features policy -d -e normal | grep -E '^(tokio|hyper) v'; then
  echo "duplicate tokio/hyper versions"; exit 1
fi

echo "== cargo check (baseline)"
RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check --locked -p hotel -p app-utils --all-targets --features hotel/baseline
echo "== cargo check (policy, with the policy crate)"
RUSTFLAGS="${RUSTFLAGS:-} -D warnings" cargo check --locked -p hotel -p app-utils -p hotel-policy --all-targets --features hotel/policy
