#!/usr/bin/env bash
# Build the hotel image for MODE, push it to the local registry and deploy.
#
#   scripts/k8s-deploy.sh [baseline|masa|policy]
#
# Expects the cluster from scripts/kind-up.sh (or any cluster that can pull
# from $REGISTRY). Environment: KIND_CLUSTER (kubectl context is
# kind-$KIND_CLUSTER), REGISTRY_PORT (default 5001), REGISTRY (default
# localhost:$REGISTRY_PORT), SKIP_BUILD=1 to reuse the pushed image,
# HOTEL_NAMESPACE (default rpcstack-hotel; the whole stack is created in it),
# HOTEL_IMAGE_NAME (repository name in the registry, default rpcstack-hotel),
# BUILDKIT_ADDR (e.g. tcp://127.0.0.1:1234: build and push with `buildctl`
# against that BuildKit daemon instead of `docker build` + `docker push`; uses a
# local buildctl, else the $BUILDCTL_IMAGE container).
set -euo pipefail
MODE="${1:-baseline}"
case "$MODE" in baseline|masa|policy) ;; *) echo "usage: $0 [baseline|masa|policy]" >&2; exit 2 ;; esac
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLUSTER="${KIND_CLUSTER:-rpcstack-hotel}"
REGISTRY="${REGISTRY:-localhost:${REGISTRY_PORT:-5001}}"
IMAGE_NAME="${HOTEL_IMAGE_NAME:-rpcstack-hotel}"
IMAGE="$REGISTRY/$IMAGE_NAME:$MODE"
NS="${HOTEL_NAMESPACE:-rpcstack-hotel}"
kc() { kubectl --context "kind-$CLUSTER" "$@"; }

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  echo "== build $IMAGE"
  if [ -n "${BUILDKIT_ADDR:-}" ]; then
    opts=(--frontend dockerfile.v0 --opt "build-arg:MODE=$MODE"
      --output "type=image,name=$IMAGE,push=true,registry.insecure=true")
    [ -n "${BUILD_JOBS:-}" ] && opts+=(--opt "build-arg:BUILD_JOBS=$BUILD_JOBS")
    if command -v buildctl > /dev/null; then
      buildctl --addr "$BUILDKIT_ADDR" build --local "context=$ROOT" --local "dockerfile=$ROOT" "${opts[@]}"
    else
      docker run --rm --network host --entrypoint buildctl -v "$ROOT:/ctx:ro" \
        "${BUILDCTL_IMAGE:-moby/buildkit:v0.22.0}" --addr "$BUILDKIT_ADDR" build \
        --local context=/ctx --local dockerfile=/ctx "${opts[@]}"
    fi
  else
    docker build --build-arg MODE="$MODE" ${BUILD_JOBS:+--build-arg BUILD_JOBS=$BUILD_JOBS} \
      -t "$IMAGE" "$ROOT"
    docker push "$IMAGE"
  fi
fi

overlay="$ROOT/k8s/base"
[ "$MODE" != baseline ] && overlay="$ROOT/k8s/overlays/$MODE"
# Re-root the overlay in $NS (this also renames the Namespace object).
render="$(mktemp -d)"
trap 'rm -rf "$render"' EXIT
printf 'apiVersion: kustomize.config.k8s.io/v1beta1\nkind: Kustomization\nnamespace: %s\nresources:\n  - %s\n' \
  "$NS" "$(realpath --relative-to="$render" "$overlay")" > "$render/kustomization.yaml"
echo "== apply $overlay (namespace $NS)"
kc kustomize --load-restrictor LoadRestrictionsNone "$render" \
  | sed "s#localhost:5001/rpcstack-hotel#$REGISTRY/$IMAGE_NAME#g" \
  | kc apply -f -
# Recreate all hotel pods at once so a re-pushed tag or changed config is
# picked up and no new pod keeps a connection to an old, terminating one.
kc -n "$NS" delete pod -l app.kubernetes.io/component=hotel --wait=false > /dev/null
for d in $(kc -n "$NS" get deployment -o name); do
  kc -n "$NS" rollout status "$d" --timeout=600s > /dev/null
done
# Wait until only the current pods remain (none terminating).
want=$(kc -n "$NS" get deployment -l app.kubernetes.io/component=hotel -o name | wc -l)
for _ in $(seq 1 120); do
  have=$(kc -n "$NS" get pods -l app.kubernetes.io/component=hotel --no-headers | wc -l)
  [ "$have" -eq "$want" ] && break
  sleep 2
done
echo "deployed $IMAGE to namespace $NS"
