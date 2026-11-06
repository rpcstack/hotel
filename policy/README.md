# policy/: the hotel app's RPC policy

This directory is the **only** place an optimization agent edits. It holds the
two things that make up the RPC policy of the `policy` mode:

| Slot | File | What it decides | How it is wired |
| --- | --- | --- | --- |
| 1. Run-queue scheduler | `queue/queue.rs` (+ files next to it) | In which order the runnable tasks of every service are polled | rpcstack's `sched_custom` feature; `RPCSTACK_QUEUE_SOURCE` (set to this file in `../.cargo/config.toml`) makes `rpcstack-sched` compile it as the Tokio run queue |
| 2. Module stack / hooks | `src/stack.rs` | Which policy modules run in the hooks of every gRPC client and server stub, and in which order (budget, deadline guard, estimation, admission, ...) | the hotel crate's `build.rs` sets `.default_hooks_path("hotel_policy::Hooks")` for every proto in `--features policy` builds |

`Cargo.toml` here also owns the **Masa feature set** (which Masa building blocks
exist: `sched_slo`, `abort_slo`, `est_mean_var`, `ac_pred`, ...) and any extra
dependency a module needs. New modules go in new files under `src/` and into
the `policy_stack![...]` list in `src/stack.rs`.

## Initial content

Both slots reproduce the `masa` mode (`sched_slo,abort_slo,signal_slack,est_mean_var,ac_pred`)
exactly, so a first `policy` build performs like `masa`:

* `queue/queue.rs` is a copy of rpcstack's `BinaryHeapQueue`, the priority heap
  `sched_slo` selects: smaller `Meta` first (for Masa, the request's deadline in
  microseconds, i.e. earliest deadline first), infrastructure work (`Meta` 0) in a
  FIFO lane that is drained first. A unit test checks it pops in the same order
  as the built-in queue.
* `src/stack.rs` is Masa's `MasaStack` for that feature set: budget -> deadline
  guard (`E2eDeadlineGuardModule`) -> estimation (`EstimationModule`) -> admission
  (`PredAdmissionModule`); `Hooks = masa_policy::PolicyHooks<Stack>`.

## Build and test a change quickly

```sh
cargo test -p hotel-policy                              # seconds: queue unit tests + wiring checks
cargo test -p hotel --features policy --test hooks      # every generated stub runs hotel_policy::Hooks
cargo build --release -p hotel --features policy --bins # all service binaries, policy mode
scripts/test.sh                                         # everything, all three modes
```

End-to-end, in the stack the benchmark runs (the `policy` mode is checked like
`masa`: liveness probe, HTTP API check, moderate load, overload, recovery):

```sh
docker build --build-arg MODE=policy -t rpcstack-hotel:policy .
scripts/smoke-compose.sh policy                         # Docker Compose
MODE=policy docker compose up --build                   # HTTP API on localhost:5000
scripts/smoke-k8s.sh policy                             # Kubernetes
```

Editing only `src/stack.rs` recompiles the hotel crates; editing `queue/` also
recompiles Tokio and everything above it (the queue is part of the runtime).

## Invariants a policy must keep

1. **Edit only this directory.** The per-route SLOs (the deadline every request
   enters the stack with) are application config (`config/hotel.json`,
   `frontend.sloMs`) and the frontend assigns them; a policy must not relax them,
   and cannot, because the root context is created outside this crate.
2. **Hooks stay wired and non-empty.** `hotel_policy::Hooks` is the name the hotel
   `build.rs` refers to. It must not become `NoopHooks`; `build.rs` fails the
   build if a stub names `NoopHooks`, and `tests/hooks.rs` and
   `policy/tests/wiring.rs` check the stubs and the run queue.
3. **Keep the queue marker.** `POLICY_QUEUE_ID` in `queue/queue.rs` is how the
   tests prove Tokio runs *this* queue and not rpcstack's `custom.rs`.
4. **Deadlines are enforced.** The liveness probe (`hotel_policy_probe --expect masa`)
   sends every service a request whose deadline already passed and requires an
   `/EarlyReturn` rejection before the handler runs. A policy that drops the
   deadline guard fails the smoke test. Keep `BudgetModule` first in the stack:
   other modules read the budget it opens, and the framework seals each child's
   budget after all modules ran.
5. **Run-queue contract.** `pop` returns exactly the items `push` accepted (none
   lost, none twice) and `len` is exact; `with_capacity(cap)` is a hint that must
   not change the order; `Meta` 0 is infrastructure work (runtime and transport
   internals) and must not be starved. A queue only orders: scheduling stays
   cooperative (nothing preempts a handler between polls). The queue runs on the
   hot path of every wake-up: no blocking, no I/O, no global locks.
6. **Same build everywhere.** All services run one image, so a module may add
   wire sections (they are propagated hop to hop) without compatibility
   concerns, but every module needs a unique wire section name.
7. **Dependencies stay light and locked.** Do not touch the rpcstack pin, or the
   `[patch.crates-io]` Tokio/Hyper entries in the root `Cargo.toml`; new crates
   must be in `Cargo.lock` (CI builds `--locked`).
8. **Test what you change.** Replace the "matches the built-in queue" test when
   you change the ordering on purpose, and add a test for each new module or
   ordering rule. Numbers are only comparable between `masa` and `policy` while
   the initial content is unchanged.

## What is not here

The HTTP API (`crates/hotel/src/frontend/http.rs`), the services, the load
generator, `config/` and the Kubernetes manifests are the application and
benchmark harness, not policy. rpcstack itself is pinned in the root
`Cargo.toml`; framework behavior that no hook can express is a framework
change, not a policy edit.
