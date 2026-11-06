#!/usr/bin/env bash
# Remove the hotel deployment; with --cluster also delete the kind cluster and
# registry (scripts/kind-down.sh).
set -euo pipefail
CLUSTER="${KIND_CLUSTER:-rpcstack-hotel}"
kubectl --context "kind-$CLUSTER" delete namespace "${HOTEL_NAMESPACE:-rpcstack-hotel}" --ignore-not-found --wait=true || true
if [ "${1:-}" = --cluster ]; then "$(dirname "$0")/kind-down.sh"; fi
