#!/usr/bin/env bash
# End-to-end smoke test of the Docker Compose deployment.
#
#   scripts/smoke-compose.sh [baseline|policy]
#
# 1. Build the image for MODE and bring up all services (no load generator).
# 2. Stack probe (hotel_policy_probe): readiness wait, control Searches, one
#    ordinary request to every service.
#    HTTP API check (hotel_http_check): every HTTP route answers with the
#    status and JSON shape of DeathStarBench's Go frontend.
# 3. Moderate load (config/smoke/moderate.json, ~30 s): zero transport errors,
#    ok rate >= SMOKE_MIN_OK_RATE (default 0.99), no rejected requests.
# 4. Overload burst (config/smoke/overload.json, timeouts expected), then a
#    recovery run: >= SMOKE_MIN_OK_RATE of both Reservation and Search over its
#    last 5 s (regression check for the reservation MongoDB wedge, README).
#    The initial `policy` mode is a no-op, so it is judged exactly like
#    `baseline`; a policy that sheds load needs its own thresholds.
# 5. The probe runs again, then: no panic in any log, no container restart.
#
# Environment: COMPOSE_PROJECT_NAME (default rpcstack-hotel-<mode>),
# KEEP_UP=1 to leave the stack running, SKIP_BUILD=1 to reuse the image.
set -euo pipefail

MODE="${1:-${MODE:-baseline}}"
case "$MODE" in baseline|policy) ;; *) echo "usage: $0 [baseline|policy]" >&2; exit 2 ;; esac
export MODE
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-rpcstack-hotel-$MODE}"
# Publish the frontend on a random host port so parallel projects never clash.
export FRONTEND_PORT="${FRONTEND_PORT:-0}"
export FRONTEND_HTTP_PORT="${FRONTEND_HTTP_PORT:-0}"
MIN_OK="${SMOKE_MIN_OK_RATE:-0.99}"
OUT="${SMOKE_OUT:-$ROOT/out/smoke-compose-$MODE}"
mkdir -p "$OUT"
dc() { docker compose "$@"; }

cleanup() {
  status=$?
  dc logs --no-color > "$OUT/compose.log" 2>&1 || true
  if [ "${KEEP_UP:-0}" != 1 ]; then dc down -v --remove-orphans > /dev/null 2>&1 || true; fi
  [ $status -eq 0 ] && echo "SMOKE PASS ($MODE)" || echo "SMOKE FAIL ($MODE), logs in $OUT"
  exit $status
}
trap cleanup EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  echo "== build ($MODE)"
  dc build frontend
fi
echo "== up"
dc up -d --scale client-bench=0
echo "image mode: $(dc run --rm --no-deps client-bench mode)"

probe() {
  dc run --rm --no-deps client-bench probe --wait-secs 300 \
    --frontend http://frontend:8660 \
    --target search=http://search:8668 --target geo=http://geo:8661 \
    --target rate=http://rate:8663 --target profile=http://profile:8662 \
    --target reservation=http://reservation:8666 --target user=http://user:8669 \
    --target recommendation=http://recommendation:8664 --target review=http://review:8667 "$@"
}

echo "== stack probe"
probe | tee "$OUT/probe.txt"

echo "== HTTP API check"
dc run --rm --no-deps client-bench http-check --http http://frontend:5000 --wait-secs 300 | tee "$OUT/http-check.txt"
grep -q "HTTP CHECK PASS" "$OUT/http-check.txt"

load() { # name -> runs config/smoke/<name>.json, traces in $OUT/<name>.txt
  dc run --rm --no-deps -e HOTEL_GEN_CONFIG="/etc/hotel/smoke/$1.json" client-bench client-bench \
    > "$OUT/$1.txt" 2> "$OUT/$1.stderr" || { tail -20 "$OUT/$1.stderr"; return 1; }
}
check() { python3 "$ROOT/scripts/lib/check_traces.py" "$@"; }

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
probe | tee "$OUT/probe-after-overload.txt"

echo "== crash check"
dc logs --no-color > "$OUT/compose.log" 2>&1
if grep -n "panicked" "$OUT/compose.log"; then echo "FAIL: panic in service logs"; exit 1; fi
restarts=$(docker inspect -f '{{.Name}} {{.RestartCount}}' $(dc ps -q) | awk '$2 > 0')
if [ -n "$restarts" ]; then echo "FAIL: restarted containers: $restarts"; exit 1; fi
echo "no panics, no restarts"
