#!/bin/sh
# Select a hotel binary by service name.
#
#   <service>                 frontend geo profile rate recommendation
#                             reservation review search user
#                             -> hotel_<service> --config $HOTEL_CONFIG
#   client-bench [args...]    load generator driven by $HOTEL_GEN_CONFIG
#   probe [args...]           policy-liveness probe (see hotel_policy_probe)
#   loadgen [args...]         simple fixed-rate load generator
#   mode                      print the policy mode baked into this image
set -eu

export RUST_LOG="${RUST_LOG:-${LOG_LEVEL:-info}}"
svc="${1:-}"
[ $# -gt 0 ] && shift

case "$svc" in
  frontend|geo|profile|rate|recommendation|reservation|review|search|user)
    echo "hotel: starting hotel_$svc ($(cat /etc/hotel-mode)) with $HOTEL_CONFIG" >&2
    exec "/usr/local/bin/hotel_$svc" --config "$HOTEL_CONFIG" "$@"
    ;;
  client-bench)
    out="${HOTEL_OUTPUT_PATH:-/tmp/hotel-load-gen}"
    status=0
    "/usr/local/bin/hotel_client_bench" --gen-config "$HOTEL_GEN_CONFIG" --output-path "$out" "$@" || status=$?
    # Emit the per-request traces so callers can collect them from the logs.
    echo "---BEGIN TRACES---"
    for f in "$out"/*.csv; do
      [ -f "$f" ] || continue
      echo "FILE: $(basename "$f")"
      cat "$f"
      echo "---END FILE---"
    done
    echo "---END TRACES---"
    exit $status
    ;;
  probe)
    exec /usr/local/bin/hotel_policy_probe "$@"
    ;;
  loadgen)
    exec /usr/local/bin/loadgen "$@"
    ;;
  mode)
    cat /etc/hotel-mode
    ;;
  *)
    echo "usage: hotel-entrypoint <service>|client-bench|probe|loadgen|mode [args...]" >&2
    exit 2
    ;;
esac
