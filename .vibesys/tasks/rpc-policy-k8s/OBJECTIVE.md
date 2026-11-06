# Hotel reservation on rpcstack: an RPC policy for throughput and p99 latency

## Goal

Write the RPC policy of the hotel reservation application (DeathStarBench's hotel, ported to
Rust on the rpcstack framework) that raises its throughput and lowers its tail latency. Both
are measured on every sample, against a Kubernetes cluster, with the same request mix:

| Objective | Direction | How it is measured |
| --- | --- | --- |
| `operations_per_second` | max | Closed loop: 128 concurrent clients, each sending its next request when the last one answered; 2 trials of 60 s after a 15 s warmup; the median. |
| `latency_ms.p99` | min | Open loop: 600 requests per second, evenly spaced, whatever the answers; 3 trials of 60 s after a 10 s warmup; the median of their p99 latencies (from each request's scheduled send time to its answer). |

Together they form a Pareto frontier: raising throughput and lowering the p99 is a win;
trading one for the other is a tradeoff. The base (the no-op policy) scores about
1,230 ops/s and 114 ms.

The mix is DeathStarBench's hotel workload (`wrk2/scripts/hotel-reservation/
mixed-workload_type_1.lua`), sent to the frontend's HTTP API (port 5000): 60% searches
(`GET /hotels`, a stay of 1 to 15 nights in April 2015 around San Francisco), 39%
recommendations (`GET /recommendations` by distance, rate or price), 0.5% logins
(`GET /user`) and 0.5% reservations of one room (`GET /reservation`). A search calls
search → geo and rate, then reservation (availability) and profile; a recommendation calls
recommendation and profile; a reservation calls user, then reservation.

## What you edit

Only `policy/`: the run-queue scheduler (`policy/queue/queue.rs` and files next to it) and
the module stack that runs in the hooks of every gRPC client and server stub
(`policy/src/stack.rs` and the modules you add under `policy/src/`), plus `policy/Cargo.toml`
for the dependencies your modules need. `policy/README.md` explains the two slots, how to
build and test a change, and the invariants a policy keeps; the `rpcstack/` submodule holds
the framework source and its published policies (masa, breakwater, dagor, rajomon,
tailclipper, c3, hedged, ...), `docs/PAPERS.md` and `docs/VIBESYS.md`.

Everything else that goes into the image is the task's, and a candidate that changes it is
refused before anything is built (the benchmark records an error, the accuracy check fails):
the services (`crates/`), `Cargo.toml` and the rpcstack pin, `.cargo/`, `config/` (the
per-route SLOs among it), `docker/`, `rust-toolchain.toml`, `Dockerfile` and
`.dockerignore`. `Cargo.lock` may gain the crates your policy's new dependencies need, and
the `hotel-policy` entry its new dependency list; no other entry may change. The policy
crate may not have a build script (`policy/build.rs`, `package.build`) or be a procedural
macro: code that runs at build time could rewrite the application. The task's
`scope.json` pins all of it.

The task builds one image from the root `Dockerfile` (its default `MODE` is `policy`: the
`policy/` directory compiled in), and deploys the repository's manifests (`k8s/base`, every
service with one replica, no resource limits) into a namespace of its own. The deployment,
the replica counts, the databases and caches, the request mix, the loads and the oracle are
fixed.

## Validity

A phase counts only if no request in its window failed (any status but 200, a timeout or a
transport error), every operation of the mix completed at least once, and the open loop
offered at least 95% of its rate. A policy may reorder, delay, prioritise or route work, but
not shed it by failing requests: a rejected or aborted request (`503`, `504`) makes the sample
an error. The application enforces no deadline of its own; requests carry their route's SLO
(200 ms, `config/hotel.json` `frontend.sloMs`) as `grpc-timeout`, which a module may read.

## Correctness

The accuracy check deploys the candidate as a sample does and sends a different mix
(searches, reservations, recommendations, logins) through the frontend at 30 requests per
second, in four phases of 60 s, with every answer recorded. Between the phases it restarts
the reservation service; empties the three Redis caches in place; and restarts the
reservation cache's pod, the frontend and the user service. The task's oracle then judges the
whole history and the check passes only if:

- every reservation with the right password books its rooms exactly when the night has that
  many rooms left, and is refused ("Failed. Already reserved. ") otherwise: no lost, doubled
  or invented reservation, across the restarts and flushes;
- every search shows each hotel with a rate plan exactly while its night has a room left,
  given every reservation acknowledged before the search started (searches are not allowed
  to be stale);
- recommendations, logins, wrong-password reservations and every hotel's GeoJSON profile are
  exactly the application's answers over its seeded data;
- at most 1% of the answers are lost or 5xx (their outcome is unknown); every other answer is
  200 with the frontend's body.

## Rules

- Do not fabricate, reuse or cache answers that the application would compute differently:
  a cached availability, an empty or default reply in place of a real one, or a success for
  work that failed. A response must come from the handler that owns it.
- Do not duplicate a non-idempotent call (a reservation) or report failure for one that took
  effect; retries and hedging are for calls that are safe to repeat.
- Do not detect the benchmark or the accuracy check (by request pattern, dates, users,
  hotels, namespace or environment) and behave differently.
- Do not depend on anything outside the policy crate and the deployment: no reading or
  writing of the databases or caches from a module, no files, no network calls of a module's
  own beyond the application's RPCs.
- Keep the invariants of `policy/README.md` (hooks stay wired, the queue marker, the
  run-queue contract, light and locked dependencies).

## Running it

`.vibesys/tasks/rpc-policy-k8s/README.md` ("Run it") shows how to run the benchmark and the
accuracy check on a Kubernetes cluster, and against a local `docker compose` deployment.
