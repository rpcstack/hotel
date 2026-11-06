#!/usr/bin/env bash
# End-to-end smoke test of the Kubernetes deployment (same checks as
# scripts/smoke-compose.sh, run as Jobs inside the cluster).
#
#   scripts/smoke-k8s.sh [baseline|policy]
#
# Creates the kind cluster + registry if needed (scripts/kind-up.sh), deploys
# MODE (scripts/k8s-deploy.sh), then runs the stack probe, the HTTP API check,
# moderate load, an overload burst and a recovery run (the initial no-op
# `policy` mode is judged exactly like `baseline`). Environment: KIND_CLUSTER,
# REGISTRY_PORT, HOTEL_NAMESPACE, HOTEL_IMAGE_NAME, BUILDKIT_ADDR (see k8s-deploy.sh), SKIP_DEPLOY=1,
# SKIP_KIND_UP=1 (use an existing cluster; do not run kind-up.sh), KEEP_UP=1
# (otherwise the namespace is deleted; the cluster is kept, remove it with
# scripts/kind-down.sh).
set -euo pipefail
MODE="${1:-baseline}"
case "$MODE" in baseline|policy) ;; *) echo "usage: $0 [baseline|policy]" >&2; exit 2 ;; esac
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLUSTER="${KIND_CLUSTER:-rpcstack-hotel}"
REGISTRY="${REGISTRY:-localhost:${REGISTRY_PORT:-5001}}"
IMAGE="$REGISTRY/${HOTEL_IMAGE_NAME:-rpcstack-hotel}:$MODE"
NS="${HOTEL_NAMESPACE:-rpcstack-hotel}"
MIN_OK="${SMOKE_MIN_OK_RATE:-0.99}"
OUT="${SMOKE_OUT:-$ROOT/out/smoke-k8s-$MODE}"
mkdir -p "$OUT"
kc() { kubectl --context "kind-$CLUSTER" "$@"; }

cleanup() {
  status=$?
  kc -n "$NS" logs -l app.kubernetes.io/component=hotel --all-containers --tail=-1 --prefix > "$OUT/pods.log" 2>&1 || true
  if [ "${KEEP_UP:-0}" != 1 ]; then kc delete namespace "$NS" --ignore-not-found --wait=true > /dev/null 2>&1 || true; fi
  [ $status -eq 0 ] && echo "SMOKE PASS ($MODE, k8s)" || echo "SMOKE FAIL ($MODE, k8s), logs in $OUT"
  exit $status
}

if [ "${SKIP_DEPLOY:-0}" != 1 ]; then
  [ "${SKIP_KIND_UP:-0}" = 1 ] || "$ROOT/scripts/kind-up.sh"
  "$ROOT/scripts/k8s-deploy.sh" "$MODE"
fi
trap cleanup EXIT

# run_job NAME ARGS... : run the hotel image as a Job, print its log to stdout.
run_job() {
  local name="$1"; shift
  local args; args=$(printf '"%s",' "$@"); args="[${args%,}]"
  kc -n "$NS" delete job "$name" --ignore-not-found --wait=true > /dev/null
  kc -n "$NS" apply -f - > /dev/null <<YAML
apiVersion: batch/v1
kind: Job
metadata:
  name: $name
spec:
  backoffLimit: 0
  template:
    spec:
      restartPolicy: Never
      containers:
        - name: $name
          image: $IMAGE
          imagePullPolicy: Always
          args: $args
          env: [{name: HOTEL_GEN_CONFIG, value: "${GEN_CONFIG:-/etc/hotel/gen_config.json}"}]
          volumeMounts: [{name: config, mountPath: /etc/hotel, readOnly: true}]
      volumes:
        - name: config
          configMap: {name: hotel-config}
YAML
  local phase=""
  for _ in $(seq 1 600); do
    phase=$(kc -n "$NS" get job "$name" -o jsonpath='{.status.conditions[?(@.status=="True")].type}' 2>/dev/null || true)
    case "$phase" in *Complete*|*Failed*) break ;; esac
    sleep 1
  done
  kc -n "$NS" logs "job/$name"
  case "$phase" in *Complete*) return 0 ;; *) echo "job $name: ${phase:-timed out}" >&2; return 1 ;; esac
}
check() { python3 "$ROOT/scripts/lib/check_traces.py" "$@"; }
restarts() {
  kc -n "$NS" get pods -l app.kubernetes.io/component=hotel \
    -o jsonpath='{range .items[*]}{.metadata.name} {.status.containerStatuses[0].restartCount}{"\n"}{end}' | sort
}

echo "image mode: $(run_job hotel-mode mode)"
echo "== stack probe"
run_probe() { # run_probe JOB
  run_job "$1" probe --wait-secs 300 \
  --frontend http://frontend:8660 \
  --target search=http://search:8668 --target geo=http://geo:8661 \
  --target rate=http://rate:8663 --target profile=http://profile:8662 \
  --target reservation=http://reservation:8666 --target user=http://user:8669 \
  --target recommendation=http://recommendation:8664 --target review=http://review:8667
}
run_probe hotel-probe | tee "$OUT/probe.txt"
echo "== HTTP API check"
run_job hotel-http-check http-check --http http://frontend:5000 --wait-secs 300 | tee "$OUT/http-check.txt"
grep -q "HTTP CHECK PASS" "$OUT/http-check.txt"
# Pods may restart while their databases come up; only restarts during the
# smoke itself count.
restarts > "$OUT/restarts.before"

load() { GEN_CONFIG="/etc/hotel/smoke-$1.json" run_job "hotel-load-$1" client-bench > "$OUT/$1.txt"; }

echo "== moderate load"
load moderate
check "$OUT/moderate.txt" --label moderate --min-requests 1000 --min-ok-rate "$MIN_OK" --max-transport 0 --max-rejected 0

# No admission control: the burst reaches the backends and
# requests time out while it lasts. What must hold is that the stack
# recovers afterwards. Regression check for the reservation MongoDB wedge
# (see README, "Known issue"): ~80 % of Reservation requests used to hang
# forever after a mixed Search + Reservation overload.
echo "== overload burst (timeouts and resets are expected)"
load overload
check "$OUT/overload.txt" --label overload --max-rejected 0

echo "== recovery: Reservation must recover after the overload"
load recovery
check "$OUT/recovery.txt" --label recovery-reservation --api Reservation --tail-secs 5 --min-requests 50 --min-ok-rate "$MIN_OK" --max-transport 0
check "$OUT/recovery.txt" --label recovery-search --api Search --tail-secs 5 --min-requests 50 --min-ok-rate "$MIN_OK" --max-transport 0

echo "== probe again (policy state and control requests after the load runs)"
run_probe hotel-probe-after | tee "$OUT/probe-after-overload.txt"

echo "== crash check"
kc -n "$NS" logs -l app.kubernetes.io/component=hotel --all-containers --tail=-1 --prefix > "$OUT/pods.log"
if grep -n "panicked" "$OUT/pods.log"; then echo "FAIL: panic in pod logs"; exit 1; fi
restarts > "$OUT/restarts.after"
if ! diff "$OUT/restarts.before" "$OUT/restarts.after"; then echo "FAIL: pods restarted during the smoke"; exit 1; fi
echo "no panics, no restarts"
