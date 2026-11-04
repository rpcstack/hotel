#!/usr/bin/env python3
"""Summarise and check hotel load-generator traces.

Reads client-bench output (stdout of `hotel-entrypoint client-bench`, which
prints every per-request CSV between ---BEGIN TRACES--- / ---END TRACES---)
and classifies each request by its `error` column:

  ok         /None or /ClientMiss: the RPC completed successfully (ClientMiss
             only means it finished after its SLO)
  rejected   /EarlyReturn...: rejected or aborted by a Masa policy module
  timeout    /ClientTimeout: no answer within the client timeout
  transport  anything else: gRPC/transport failures (Unavailable, resets, ...)

Exit status 1 if any requested check fails.
"""
import argparse
import collections
import csv
import io
import re
import sys


def parse(stream):
    rows, current, buf = [], None, []
    for line in stream:
        line = line.rstrip("\n")
        if line.startswith("FILE: "):
            current, buf = line[6:], []
        elif line == "---END FILE---" and current is not None:
            rows.extend(csv.DictReader(io.StringIO("\n".join(buf) + "\n")))
            current = None
        elif current is not None:
            buf.append(line)
    return rows


def classify(err):
    if err in ("/None", "/ClientMiss"):
        return "ok"
    if err.startswith("/EarlyReturn"):
        return "rejected"
    if err == "/ClientTimeout":
        return "timeout"
    return "transport"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--label", default="run")
    ap.add_argument("--min-requests", type=int, default=1)
    ap.add_argument("--api", help="only judge requests of this API (e.g. Reservation)")
    ap.add_argument("--tail-secs", type=float,
                    help="only judge requests started in the last N seconds of the run")
    ap.add_argument("--min-ok-rate", type=float, help="minimum fraction of ok requests")
    ap.add_argument("--max-transport", type=int, help="maximum transport errors")
    ap.add_argument("--min-rejected", type=int, help="minimum Masa /EarlyReturn rejections")
    ap.add_argument("--max-rejected", type=int, help="maximum Masa /EarlyReturn rejections")
    args = ap.parse_args()

    with open(args.file) as f:
        rows = parse(f)
    if args.api:
        rows = [r for r in rows if r["api"] == args.api]
        print(f"[{args.label}] judging {args.api} requests only")
    if args.tail_secs is not None and rows:
        end = max(int(r["start_at"]) for r in rows)
        cutoff = end - args.tail_secs * 1e6  # start_at is in microseconds
        total = len(rows)
        rows = [r for r in rows if int(r["start_at"]) >= cutoff]
        print(f"[{args.label}] judging the last {args.tail_secs:g} s: {len(rows)} of {total} requests")
    kinds = collections.Counter(classify(r["error"]) for r in rows)
    sources = collections.Counter()
    for r in rows:
        if r["error"].startswith("/EarlyReturn"):
            m = re.search(r"src=([^?&]+)(?:.*reason=(\w+))?", r["error"])
            sources[(m.group(1), m.group(2) or "deadline") if m else ("?", "?")] += 1
    n = len(rows)
    ok_rate = kinds["ok"] / n if n else 0.0
    print(f"[{args.label}] requests={n} ok={kinds['ok']} ({ok_rate:.2%}) "
          f"rejected={kinds['rejected']} timeout={kinds['timeout']} transport={kinds['transport']}")
    for (src, reason), c in sources.most_common(8):
        print(f"[{args.label}]   rejected by {src} ({reason}): {c}")
    for r in rows:
        if classify(r["error"]) == "transport":
            print(f"[{args.label}]   e.g. transport error: {r['error']}")
            break

    failures = []
    if n < args.min_requests:
        failures.append(f"only {n} requests traced (< {args.min_requests})")
    if args.min_ok_rate is not None and ok_rate < args.min_ok_rate:
        failures.append(f"ok rate {ok_rate:.2%} < {args.min_ok_rate:.2%}")
    if args.max_transport is not None and kinds["transport"] > args.max_transport:
        failures.append(f"{kinds['transport']} transport errors > {args.max_transport}")
    if args.min_rejected is not None and kinds["rejected"] < args.min_rejected:
        failures.append(f"{kinds['rejected']} policy rejections < {args.min_rejected}")
    if args.max_rejected is not None and kinds["rejected"] > args.max_rejected:
        failures.append(f"{kinds['rejected']} policy rejections > {args.max_rejected}")
    for f in failures:
        print(f"[{args.label}] FAIL: {f}")
    print(f"[{args.label}] {'PASS' if not failures else 'FAIL'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
