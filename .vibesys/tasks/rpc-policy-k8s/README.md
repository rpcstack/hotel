# rpc-policy-k8s: the hotel's RPC policy, on Kubernetes

The vibesys optimization task of this repository: an agent edits only `policy/` (the run
queue and the module stack of rpcstack) and is scored on the hotel's throughput (closed loop)
and p99 latency (open loop at a fixed rate) on Kubernetes, behind an accuracy check whose
oracle judges every answer of a recorded history. The agent's brief is
[OBJECTIVE.md](OBJECTIVE.md); this file is for whoever maintains the task.

It runs on vibesys-evaluator 0.3.0rc3: the Go SDK (`github.com/uw-syfi/vibesys/sdk/
vibesys-evaluator/go v0.3.0-rc.3`, in `evaluator/go.mod`) and the Python package
(`vibesys-evaluator==0.3.0rc3` from PyPI, installed in `Dockerfile`).
Both pins move together.

## Layout

| Path | What it is |
| --- | --- |
| `vibesys.input.toml` | The task (schema 0.2): `benchmark` and `accuracy` commands, both `/task/rpc-policy-k8s` |
| `objectives.toml` | The two objectives and their noise |
| `OBJECTIVE.md` | The agent's brief: objectives, edit scope, validity, correctness, rules |
| `Dockerfile` | The task image: the evaluator image plus the task's Go programs in `/task` |
| `k8s/hotel.yaml`, `k8s/render.sh` | The deployed manifest, rendered from the repository's `k8s/base` (no Namespace, `${HOTEL_IMAGE}`); `render.sh --check` says whether it is stale |
| `scope.json` | The edit scope: digests of every file outside `policy/` the image build reads, and `Cargo.lock`'s packages (`/task/scope write` regenerates it) |
| `evaluator/cmd/rpc-policy-k8s` | The task binary: `microservices.Main` (benchmark, accuracy check, the pods' commands) behind the scope guard |
| `evaluator/cmd/oracle` | The oracle offline: `oracle --history DIR` judges a captured history again, `oracle stats DIR` says what it exercised |
| `evaluator/cmd/scope` | Writes or checks `scope.json` |
| `evaluator/cmd/explore` | The benchmark at other loads (for choosing them; not a task command) |
| `evaluator/internal/hotel` | The application as the task sees it: seeded data, messages, the recommendation reference |
| `evaluator/internal/workload` | The benchmark's mix (`bench.go`) and the accuracy workload (`accuracy.go`) |
| `evaluator/internal/oracle` | The oracle's specs |
| `evaluator/internal/task` | The wiring: the App, the benchmark, the accuracy check, the faults, the scope guard |
| `evaluator/internal/scope` | The scope check |
| `tests/mutants/*.patch`, `tests/mutants.sh` | Policy mutants (each a change of `policy/` only) that the accuracy check must fail |

## The application on Kubernetes

`k8s.App`: the manifest `k8s/hotel.yaml` (the 9 services, 5 MongoDB, 4 Redis, the config
map, as `scripts/k8s-deploy.sh` applies them; every service one replica, no resource limits),
one image built from the candidate's root `Dockerfile` (`Images: {HOTEL_IMAGE: "."}`), the
gateway `http://frontend:5000`. The root `Dockerfile`'s default `MODE` is now `policy`, since
the evaluator builds without build arguments; every other user of it (Compose, CI,
`scripts/k8s-deploy.sh`) passes `MODE` explicitly. The services seed their own databases at
start (`crates/hotel/src/*/db.rs`), so the task has no seed step: its dataset
(`internal/hotel.Seeded`) mirrors those seeds, and the base's accuracy PASS is what checks that
they agree. The readiness steps are a login, a search and a recommendation on a night no
workload uses.

## The benchmark

One sample: 2 closed-loop phases of 128 clients (60 s after 15 s warmup, the median),
then 3 open-loop phases at 600 ops/s, uniform arrivals (60 s after 10 s warmup, the median
of the p99s). Validity: no error in a window, every operation at least once, the open loop at
95% of its rate (DeathStarBench's and TrainTicket's rules).

The mix is DeathStarBench's `mixed-workload_type_1.lua`, drawn as it draws it: search 60%
(in 2015-04-09..23, out in+1..24, lat 38.0235 ± 0.2405, lon -122.095 ± 0.157 in 0.001 steps),
recommend 39% (dis/rate/price at 0.33/0.33/0.34), login 0.5% and reserve 0.5% (users 0..500,
hotel 1..80, 1 to 5 nights, one room). The old task (`hotel-goodput` on vibesys-playground's
rpcstack) sent only Search and Reservation over gRPC; this one uses the HTTP API and the
whole DeathStarBench mix. All logins use the right password (as in DeathStarBench).

### Choosing the loads

The base on vibesys-k8s, one 30 s phase per load after a 10 s warmup (`cmd/explore`, with
`EXPLORE_CLIENTS` and `EXPLORE_RATES`; two sweeps, each closed loop before its open loops):

| Closed loop, clients | 64 | 128 | 192 | 256 | 320 | 512 |
| --- | --- | --- | --- | --- | --- | --- |
| ops/s | 995 | 1595 | 1276 | 1558, 1516 | 1331 | 1209 |
| p99 ms | 116 | 175 | 355 | 348, 379 | 429 | 387 |
| timeouts in the window | 0 | 0 | 0 | 0, 23 | 413 | 1033 |

| Open loop, offered ops/s | 400 | 800 | 900 | 1000 | 1100 | 1200 | 1600 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| achieved ops/s | 399 | 798 | 899 | 980, 999 | 1099 | 1198 | 1228 |
| p50 ms | 103 | 103 | 104 | 1289, 103 | 103 | 104 | 3490 |
| p99 ms | 106 | 113 | 145 | 5358, 116 | 116 | 199 | 10756 |

The capacity is about 1,500 ops/s: the reservation service, whose CheckAvailability every
search calls, runs on one thread (a `current_thread` runtime) and is the bottleneck. Below it
most requests are searches, whose latency is CheckAvailability's 100 ms sleep plus queueing:
the p50 is 103 ms.

Near the capacity the base loses searches. What the runs show: bursts of searches, all sent
within a second or so, that do not answer within the load generator's 10 s timeout, while
every other request (and every search sent after the burst) is answered: 23 to 165 timeouts
in a 60 s window at 256 clients; 1115 in the 1000 ops/s phase above, which stayed slow; 380
sent within one second of the first open loop of a benchmark sample at 128 clients and
800 ops/s (its closed loops had none). Any of them makes the sample an error under the
no-error rule. What fits it in `crates/hotel/src/reservation/server.rs`: CheckAvailability
reads its counts from Redis under a 100 ms timeout, which includes the time the request waits
for the service's one thread; when it expires, the request queries MongoDB for every hotel
night it asked for instead (27 hotels times up to 15 nights, one query each), so a busy moment
turns every search that arrives during it into hundreds of MongoDB queries.

The loads are therefore well below the capacity: 128 closed-loop clients (the knee, the
highest throughput of the sweep, and no timeout in any of the 9 windows at 128 clients so
far) and an open loop at 600 ops/s, about 40% of the capacity. (At 192 clients and more the
first second of a closed loop also loses a burst of searches; the 15 s warmup keeps it out
of the window.) A policy that keeps the reservation service from falling behind is one way
to do better on both objectives.

### Baseline

The base on vibesys-k8s (vibesys-evaluator 0.3.0rc2), three samples one after another:

| Sample | Closed loop, ops/s (2 phases) | `operations_per_second` | Open loop p99, ms (3 phases) | `latency_ms.p99` |
| --- | --- | --- | --- | --- |
| 1 | 1270.4, 1196.2 | 1233.3 | 114.62, 109.95, 115.58 | 114.62 |
| 2 | 1308.3, 1196.2 | 1252.2 | 111.94, 114.05, 114.11 | 114.05 |
| 3 | 1264.9, 1189.3 | 1227.1 | 111.42, 111.74, 111.68 | 111.68 |

Median 1233.3 ops/s (spread 2.0% of it) and 114.05 ms (spread 2.6%); every open-loop phase
sustained 599.3 to 599.5 ops/s with no error. `objectives.toml` sets the noise at three times
the spread, rounded up: 0.06 and 0.08. The second closed-loop phase is 6 to 9% slower than the
first in every sample. Other jobs on the clusters' host move the throughput more than the
spread: the sweep above, earlier the same day, measured 1595 ops/s at 128 clients.

## The accuracy check

Four open-loop phases at 30 ops/s (uniform), 60 s each, the whole history captured; faults
between them: after phase 0 the reservation service restarts; after phase 1 the three Redis
caches are flushed in place (`microservices.Flush`); after phase 2 the reservation cache's
pod, the frontend and the user service restart. MongoDB is not restarted: its data lives in
an `emptyDir` and would go with the pod.

### The workload (`internal/workload/accuracy.go`)

Searches 40%, reservations 36%, recommendations 12%, logins 12%. The application has races of
its own that a strict oracle would blame on the candidate, so the workload avoids them:

- The reservation service checks and books a night with separate Redis reads and writes: two
  concurrent reservations of one night can both succeed against the same count. Each phase
  books its own nights (3 per phase, April 9..20) round-robin over its 90 hotel nights (the 27
  hotels a search returns and 3 others per night), so a night's next reservation comes a whole
  round later: at least 222 operations (7 s) in every phase of the tests.
- A search that misses the reservation cache fills it from MongoDB after reading it, which can
  overwrite a concurrent reservation's newer count. Each phase starts with 90 operations (3 s)
  without a reservation of a night (its reservations use a wrong password instead, which
  never reaches the reservation service), whose searches go round every night booked so far
  and fill the cache again after a fault emptied it.
- The availability check of a multi-night stay answers from whichever night's database read
  finishes last when the cache misses (a `JoinSet` overwriting a map entry), so searches and
  reservations are of one night.
- Recommendations by distance avoid locations two hotels are nearly equally near to.

Each night gets a plan of four reservations (rooms drawn from the sample's model stream):
`exact` (a + b + rest: full, then 1 more refused), `partial` (one room short, 2 more refused,
1 more: full), `over` (one room too many refused, then exactly full, then 1 more refused) or
`light` (four small ones). After its plans a phase probes its nights again with one-room
reservations, and from the second phase on also the nights of earlier phases (15% of the
reservations), across the faults. The seeded booking (hotel 4, April 9, one room) is always
planned. 5% of the reservations and 12.5% of the logins use a wrong password.

### The oracle (`internal/oracle`)

Two specs of vibesys-evaluator's `check` package:

- `rooms`: one key per hotel night, the state the rooms booked (from the seeded count). A
  reservation with the right password is a write: booked iff the night has the rooms left,
  refused otherwise; lost or 5xx answers may or may not have booked (within a 10 s effect
  window). A search is a read per hotel with a rate plan (27 per search): shown iff the night
  has a room left. Reads are strict: the application's Redis is written before a reservation
  answers. No fault changes the state (MongoDB holds it; Redis is a cache).
- `responses`: every call once, unknown when lost or 5xx (so the 1% limit on unknown outcomes
  counts calls, not the 27 pieces of a search); and every answer checked on its own: status
  200, the body's exact shape, each hotel's GeoJSON profile (float32 coordinates), searches
  showing only hotels with rate plans, recommendations equal to the reference
  (`hotel.Recommend`: geo 0.28's haversine as the service computes it), logins and
  wrong-password reservations.

### Results

On vibesys-k8s-2 the base passes and every mutant fails (`tests/mutants.sh`; each run takes
6 to 6.5 minutes from start to verdict: the image build, mostly cached, the deployment, the
four phases, the seven faults at 4 to 9 s each, and the check):

| Candidate | Verdict | What the oracle reports |
| --- | --- | --- |
| base (two runs) | PASS | 7200 calls, none lost or 5xx; `rooms`: 360 hotel nights, 81445 and 81938 pieces, every night linearizable; `responses`: every answer right |
| `stale-availability-cache` | FAIL | `rooms`: 242 of 360 nights, a search shows a hotel all of whose rooms are booked |
| `empty-reservation` | FAIL | `rooms`: 202 of 360 nights, a reservation refused while the night has the rooms |
| `abort-after-commit` | FAIL | `responses`: 772 of 7200 calls (10.7%) answered 504, unknown outcomes above the 1% limit (`rooms` passes: a 504 may have booked) |

What the base's history exercises (`oracle stats`, the later run): 2931 searches (39526
hotels shown, 39611 left out because their night was full), 2308 reservations with the right
password (1231 booked, 1077 refused; by plan: exact 315 booked / 105 refused, partial
327 / 109, over 142 / 142, light 300 / 0, probes 113 / 490, earlier phases' nights 34 / 231),
259 with a wrong password, 857 logins (103 with a wrong password), 845 recommendations. No
call was lost or answered 5xx across the faults (restarting the reservation service, the
frontend or the user service takes 8.6 s, the reservation cache's pod 6.4 s, a flush 4.1 s).
At most 2 operations of one hotel night were ever in flight together (the workload's
spacing; the checker's cost grows with it); the check takes about 0.5 s and 40 MB.

## Run it

On a cluster (the environment vibesys gives a job: `KUBECONFIG`, `VIBESYS_K8S_REGISTRY`,
`VIBESYS_K8S_BUILDER`), from the repository root. The runner rebuilds the task image in the
cluster from `/task` for its pods; that build needs no registry credentials (its bases are public
and the evaluator comes from PyPI):

```sh
docker build -t rpc-policy-k8s-task .vibesys/tasks/rpc-policy-k8s
run() { docker run --rm --init --network host --user "$(id -u):$(id -g)" \
  -v "$PWD:/candidate:ro" -v "$PWD/out:/vibesys/artifacts" -v "$KUBECONFIG:/kubeconfig:ro" \
  -e KUBECONFIG=/kubeconfig -e VIBESYS_K8S_REGISTRY -e VIBESYS_K8S_BUILDER \
  rpc-policy-k8s-task /task/rpc-policy-k8s "$@" --project-root /candidate; }
mkdir -p out && chmod 777 out
run accuracy                                   # exit 0 PASS, 1 FAIL, 2 UNKNOWN; out/accuracy/report.txt
run benchmark --vs-output /vibesys/artifacts/result.json
.vibesys/tasks/rpc-policy-k8s/tests/mutants.sh rpc-policy-k8s-task /tmp/mutants   # base PASS, mutants FAIL
```

Locally, without a cluster, against `docker compose` (the task then measures what is running
and injects no faults; the application must be fresh, with nothing booked):

```sh
MODE=policy docker compose up -d --build       # frontend on localhost:5000
docker run --rm --init --network host -v "$PWD:/candidate:ro" -e HOTEL_TARGET=http://127.0.0.1:5000 \
  rpc-policy-k8s-task /task/rpc-policy-k8s accuracy --project-root /candidate --out /tmp/acc
docker compose down -v
```

`go test ./...` in `evaluator/` runs the unit tests: the oracle on synthetic histories (each
violation it must catch, unknown outcomes, the right answers), the workload's plans and its
spacing of reservations, the dataset against the application's answers, the manifest, the
scope check.

## Mutants

`tests/mutants/*.patch` are policies that are wrong in ways an optimizing agent might try,
each a change of `policy/` only (in scope), using rpcstack hooks:

| Mutant | What it does | Why the check fails it |
| --- | --- | --- |
| `stale-availability-cache` | Memoizes CheckAvailability for 30 s at the reservation service, keyed by the request's hash the frontend sends in the wire (skips the handler and its 100 ms sleep); answers built with `#![feature(specialization)]` | searches keep showing hotels whose night filled |
| `empty-reservation` | "Sheds" one MakeReservation in four with an empty OK before the handler runs | a refused reservation while rooms are left |
| `abort-after-commit` | A "deadline guard" failing one MakeReservation child in three with DEADLINE_EXCEEDED after it booked | 504s for reservations that were made: unknown outcomes above 1% |

A mutant must actually do what it claims, or its FAIL proves nothing: check that it changes
the application's behaviour (the reservation service's CPU, the answers) before relying on its
verdict. One rpcstack detail matters here: a module sees the handler's response message in
`after_poll`, while `finalize` sees it already wrapped for serialization, so the cache mutant
saves its answers in `after_poll` (a first version that saved them in `finalize` never filled
its cache, and passed).

## Pinning and maintenance

- The evaluator pins are the Go SDK in `evaluator/go.mod` and the base image in `Dockerfile`;
  move them together, then run the baseline again (three samples, "Baseline") and set the
  noise in `objectives.toml` from it, as for any change of the loads.
- When the application changes (another commit of `crates/`, `config/`, the `Dockerfile`, a
  new rpcstack pin in `Cargo.toml`/`Cargo.lock`): `k8s/render.sh` and `/task/scope write
  --project-root . --out .vibesys/tasks/rpc-policy-k8s/scope.json` (or `go run ./cmd/scope
  write` from `evaluator/`), and check `internal/hotel` against its seeds (`go test`, then the
  base's accuracy check).
