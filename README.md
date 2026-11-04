# hotel on rpcstack

The DeathStarBench **hotel reservation** microservice benchmark, written in Rust
on [rpcstack](https://github.com/rpcstack/rpcstack): Tonic gRPC services whose
generated stubs run an rpcstack policy stack. Two policy modes are built from the
same source: `baseline` (no policy) and `masa` (the Masa composed policy).

![Hotel call graph](assets/hotel_callgraph.jpg)

Each rectangular box is a microservice. Coloured rounded boxes on the frontend
are its public APIs; edge numbers give the order in which a parent calls its
children. Services with a cache check Redis first, fall back to MongoDB, and
fill the cache from MongoDB ("MC" in the figure; this port uses Redis).

| Service | Port | Backends | Called by |
| --- | --- | --- | --- |
| frontend | 8660 | | load generator |
| search | 8668 | | frontend |
| geo | 8661 | in-memory k-d tree | search |
| rate | 8663 | rate-mongo, rate-redis | search |
| profile | 8662 | profile-mongo, profile-redis | frontend |
| reservation | 8666 | reservation-mongo, reservation-redis | frontend |
| user | 8669 | user-mongo | frontend |
| recommendation | 8664 | in-memory | (standalone) |
| review | 8667 | review-mongo, review-redis | (standalone; frontend's review API is a stub) |

The load generator (`hotel_client_bench`) sends `Search` and `Reservation`
requests at a fixed rate, each carrying a Masa root context with its SLO, and
writes one CSV row per request.

## Quick start: Docker Compose

Requires Docker with Compose v2. Images are built locally; nothing is pulled
except `rust`, `debian`, `mongo:7.0` and `redis:7.2`.

```sh
docker compose up --build                 # baseline
MODE=masa docker compose up --build       # Masa policy
```

This builds one image (`rpcstack-hotel:<mode>`, all binaries), starts the 9
MongoDB/Redis containers, the 9 hotel services and the load generator. The load
generator runs `config/gen_config.json` (100 rps for 30 s after 5 s warm-up),
prints per-second counters, dumps its traces between `---BEGIN TRACES---` and
`---END TRACES---`, and exits; the services keep running. Run it again with
`docker compose run --rm client-bench`. The frontend is published on
`localhost:${FRONTEND_PORT:-8660}`. Stop with `docker compose down -v`.

The first build compiles the pinned nightly toolchain's dependencies and takes
several minutes; later builds reuse BuildKit caches.

## Quick start: Kubernetes (kind)

Requires Docker, [kind](https://kind.sigs.k8s.io/) and kubectl.

```sh
scripts/kind-up.sh                 # kind cluster "rpcstack-hotel" + registry:2 on localhost:5001
scripts/k8s-deploy.sh masa         # build, push to the local registry, kubectl apply
kubectl -n rpcstack-hotel get pods
scripts/k8s-teardown.sh --cluster  # delete namespace, cluster and registry
```

`k8s/base` holds plain manifests (a Deployment and Service per process, a
ConfigMap generated from `config/`); `k8s/overlays/masa` only switches the image
tag. `k8s-deploy.sh` renders them with
`kubectl kustomize --load-restrictor LoadRestrictionsNone` because the ConfigMap
reads `config/` outside the kustomization. Set `REGISTRY_PORT` / `KIND_CLUSTER`
to avoid clashes; any cluster that can pull from `$REGISTRY` works.

## Policy modes

| Mode | Cargo features of `hotel` | Generated stub hooks |
| --- | --- | --- |
| `baseline` | `baseline` (none) | `masa::DefaultHooks` = `tonic::rpcstack::noop::NoopHooks` |
| `masa` | `sched_slo,abort_slo,signal_slack,est_mean_var,ac_pred` | `masa_policy::PolicyHooks` (Masa stack) |

The mode is chosen at build time: Cargo feature → Docker build arg `MODE` →
`MODE` env for Compose, or overlay for Kubernetes. Each individual Masa feature
(`sched_fifo`, `abort_slack`, `trace_queue_latency`, `ac_rajomon`, ...) is also
exposed by the `hotel` crate; build an image with an explicit list via
`docker build --build-arg FEATURES=sched_slo,abort_slo .` (or `FEATURES=...`
with Compose). `docker run --rm rpcstack-hotel:<mode> mode` prints what an image
was built with.

The masa preset is the one documented in rpcstack's
`examples/masa/README.md`. Scheduling is cooperative: rpcstack reorders and
rejects work between future polls; it does not preempt a running handler.

## Configuration

* `config/hotel.json`: service addresses (Compose service names, which are also
  the Kubernetes Service names), ports, data sizes, cache-miss and user-check
  probabilities. With `replicas: 1` a service is dialled by name; with
  `replicas: N > 1` Masa's client-side round robin dials `<ip>-1 .. <ip>-N`.
* `config/gen_config.json`: load generator (APIs, SLOs in µs, timeouts in ms,
  rps list, warm-up and duration, optional `ApiWeights` and `MaxInFlight`).
* `config/smoke/*.json`: the smoke test's moderate, overload and recovery runs.
* `LOG_LEVEL` / `RUST_LOG` control logging. `MASA_POLICY_PARAMS_PATH` points
  Masa at a policy-parameters file if you mount one.

## Testing

```sh
scripts/test.sh                 # cargo test, both modes (incl. hooks regression test)
scripts/check.sh                # fmt, rpcstack pin, single tokio/hyper, -D warnings, both modes
scripts/smoke-compose.sh masa   # full stack under Compose (also: baseline)
scripts/smoke-k8s.sh masa       # full stack on kind (also: baseline)
```

The smoke scripts bring the stack up, then:

1. **Policy-liveness probe** (`hotel_policy_probe`): waits for an end-to-end
   Search, sends 20 control Searches with a 5 s SLO (all must succeed), then
   sends each of the 9 services one request whose Masa context deadline has
   already expired. In `masa` mode every service must reject it with
   `DEADLINE_EXCEEDED "/EarlyReturn?src=<that service>…"`, produced by Masa's
   deadline guard in that service's server hooks before the handler runs. In
   `baseline` none may: the no-op hooks ignore the context and the handler
   answers. So the masa check fails on a baseline build, and on any service
   whose stubs fell back to no-op hooks.
2. **Moderate load** (200 rps, 30 s, SLO 200 ms): zero transport errors and
   ≥ 99 % of requests completed.
3. **Overload burst** (masa only; 4000 rps, 10 s, SLO 20 ms): at least one
   request must be rejected or aborted with `/EarlyReturn`.
4. **Recovery** (masa only; 100 rps, 20 s): ≥ 99 % completed and zero
   transport errors over the last 5 s. Masa's predictive admission is an AIMD
   controller that reopens only as admitted requests complete, so the first
   seconds after an overload still shed load by design.
5. The probe runs again; no service log may contain a panic and no
   container/pod may restart during the run.

`SMOKE_OVERLOAD=1` also runs the burst in baseline, report only. Without
admission control the burst reaches the backends; in our runs baseline
`Reservation` requests kept timing out (~80 %) for at least 20 s afterwards
while `Search` recovered, so baseline is not held to the recovery check.

Results land in `out/smoke-<compose|k8s>-<mode>/`.

## Repository layout

```
Cargo.toml            workspace; pins rpcstack (git rev) and patches tokio/hyper
crates/hotel          services, load generator, probe, protos, build.rs
crates/app-utils      load-generator runtime, logging, latency stats
config/               runtime configuration (mounted into containers)
Dockerfile            one builder, one runtime image; docker/entrypoint.sh selects the binary
docker-compose.yaml   full stack + load generator
k8s/                  kustomize base (baseline) and masa overlay
scripts/              test/check/format, smoke, kind and deploy helpers
```

## Provenance

Ported from [vic-lsh/masa](https://github.com/vic-lsh/masa) `apps/hotel` and
`apps/app-utils` at commit `c8486eb`, itself a port of DeathStarBench's
hotelReservation. Built on rpcstack at
`de0cedc080882469b316c8125b7ad17a7c93e7fb`. Changes from the source:
dependencies trimmed to those used (the tonic-example feature set, unused
memcached pool and `app-util-macros` were dropped); `build.rs` sets
`default_hooks_path("masa::DefaultHooks")` for every proto (rpcstack stubs
otherwise default to no-op hooks); single-replica services are dialled by DNS
name; added the policy probe, Docker/Compose/Kubernetes deployment and smoke
tests. The masa experiment runner and its configs are not included. See
[TEMPLATE.md](TEMPLATE.md) for the layout other application repositories reuse.
