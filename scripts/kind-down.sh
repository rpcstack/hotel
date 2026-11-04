#!/usr/bin/env bash
# Delete the kind cluster and local registry created by scripts/kind-up.sh.
set -euo pipefail
CLUSTER="${KIND_CLUSTER:-rpcstack-hotel}"
REG_NAME="${REGISTRY_NAME:-rpcstack-kind-registry}"
kind delete cluster --name "$CLUSTER" || true
docker rm -f "$REG_NAME" > /dev/null 2>&1 || true
echo "deleted kind cluster $CLUSTER and registry $REG_NAME"
