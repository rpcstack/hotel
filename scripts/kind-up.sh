#!/usr/bin/env bash
# Create a kind cluster wired to a local registry (the standard kind
# local-registry recipe: https://kind.sigs.k8s.io/docs/user/local-registry/).
#
#   scripts/kind-up.sh
#
# Environment: KIND_CLUSTER (default rpcstack-hotel), REGISTRY_NAME (default
# rpcstack-kind-registry), REGISTRY_PORT (default 5001; host port on 127.0.0.1).
# Idempotent: reuses an existing registry container and cluster.
set -euo pipefail
CLUSTER="${KIND_CLUSTER:-rpcstack-hotel}"
REG_NAME="${REGISTRY_NAME:-rpcstack-kind-registry}"
REG_PORT="${REGISTRY_PORT:-5001}"

if [ "$(docker inspect -f '{{.State.Running}}' "$REG_NAME" 2>/dev/null || true)" != true ]; then
  echo "== starting registry $REG_NAME on 127.0.0.1:$REG_PORT"
  docker run -d --restart=always -p "127.0.0.1:${REG_PORT}:5000" --network bridge \
    --name "$REG_NAME" registry:2
fi

if ! kind get clusters 2>/dev/null | grep -qx "$CLUSTER"; then
  echo "== creating kind cluster $CLUSTER"
  cat <<YAML | kind create cluster --name "$CLUSTER" --wait 120s --config=-
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
containerdConfigPatches:
- |-
  [plugins."io.containerd.grpc.v1.cri".registry]
    config_path = "/etc/containerd/certs.d"
YAML
fi

# Resolve localhost:$REG_PORT inside the nodes to the registry container.
REGISTRY_DIR="/etc/containerd/certs.d/localhost:${REG_PORT}"
for node in $(kind get nodes --name "$CLUSTER"); do
  docker exec "$node" mkdir -p "$REGISTRY_DIR"
  printf '[host."http://%s:5000"]\n' "$REG_NAME" | docker exec -i "$node" cp /dev/stdin "$REGISTRY_DIR/hosts.toml"
done

if [ "$(docker inspect -f '{{json .NetworkSettings.Networks.kind}}' "$REG_NAME")" = null ]; then
  docker network connect kind "$REG_NAME"
fi

kubectl --context "kind-$CLUSTER" apply -f - <<YAML
apiVersion: v1
kind: ConfigMap
metadata:
  name: local-registry-hosting
  namespace: kube-public
data:
  localRegistryHosting.v1: |
    host: "localhost:${REG_PORT}"
    help: "https://kind.sigs.k8s.io/docs/user/local-registry/"
YAML
echo "kind cluster kind-$CLUSTER ready; registry localhost:$REG_PORT"
