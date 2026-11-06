# policy/: the hotel app's RPC policy

This directory is the **only** place an optimization agent edits. It holds the
two things that make up the RPC policy of the `policy` mode:

| Slot | File | What it decides | How it is wired |
| --- | --- | --- | --- |
| 1. Run-queue scheduler | `queue/queue.rs` (+ files next to it) | In which order the runnable tasks of every service are polled | rpcstack's `sched_custom` feature; `RPCSTACK_QUEUE_SOURCE` (set to this file in `../.cargo/config.toml`) makes `rpcstack-sched` compile it as the Tokio run queue |
| 2. Module stack / hooks | `src/stack.rs` | Which policy modules run in the hooks of every gRPC client and server stub, and in which order (whatever the policy defines: admission, deadline guards, estimators, ...) | the hotel crate's `build.rs` sets `.default_hooks_path("hotel_policy::Hooks")` for every proto in `--features policy` builds |

`Cargo.toml` here also owns the framework features the policy needs and any extra
dependency a module needs. New modules go in new files under `src/` and into
the `policy_stack![...]` list in `src/stack.rs`.

## Initial content: a no-op policy

Both slots start empty, so the `policy` mode behaves like `baseline` and its
numbers should match baseline's:

* `queue/queue.rs` is plain first-in-first-out, ignoring `Meta`: the same as the
  framework's default queue (`rpcstack_sched::builtin::fifo`). A unit test checks
  it pops in the same order as that queue.
* `src/stack.rs` has an empty module stack (`policy_stack![]`, a pass-through);
  `Hooks = rpcstack_tonic::PolicyHooks<Stack>` is the policy crate's own hooks
  type, never `NoopHooks`, even while it does nothing.
* `Cargo.toml` depends on the framework core only (`rpcstack`,
  `rpcstack-tonic`, `rpcstack-sched`, Tokio and Tonic with the `rpcstack`
  features that carry the scheduling priority).

## Reference material: the `rpcstack/` submodule

What can go in each slot is shown by the framework source and its published
policies, in the `rpcstack/` git submodule at the repository root (read-only
reference, the same commit the build uses; fetch it with
`git submodule update --init rpcstack`):

* `rpcstack/libs/rpcstack-sched/` (`README.md`, `src/custom.rs`): the run-queue
  interface, the built-in FIFO and priority-heap queues and the editable slot
  this crate's `queue/queue.rs` is the application-owned version of.
* `rpcstack/examples/`: the published policies (masa, breakwater, dagor,
  rajomon, tailclipper, c3, hedged, ...), each with queues, modules and tests;
  `examples/masa/masa-policy/src/masa_stack.rs` is a full module stack,
  `examples/tailclipper/queue.rs` an external queue source.
* `rpcstack/docs/PAPERS.md` and `rpcstack/docs/VIBESYS.md`: which mechanism
  expresses which paper's policy, and how to use rpcstack as a VibeSys policy
  tool.

The framework core has no notion of deadlines, priorities or budgets; policies
bring their own modules for those. A module from an example crate needs that
crate as a git dependency in this crate's `Cargo.toml` at the same `rev` as the
framework (features like `sched_slo` live in the example crates). The request
deadline itself is application configuration: every request enters with a
`grpc-timeout` (the route's SLO, `config/hotel.json` `frontend.sloMs`) that
modules can read from the request headers.

## Build and test a change quickly

```sh
cargo test -p hotel-policy                              # seconds: queue unit tests + wiring checks
cargo test -p hotel --features policy --test hooks      # every generated stub runs hotel_policy::Hooks
cargo build --release -p hotel --features policy --bins # all service binaries, policy mode
scripts/test.sh                                         # everything, all three modes
```

End-to-end, in the stack the benchmark runs (the `policy` mode is judged like
`baseline`: stack probe, HTTP API check, moderate load, overload, recovery;
a policy that sheds load needs its own thresholds in the smoke scripts):

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
4. **Module order.** Modules run in list order in pre-hooks and in reverse in
   post-hooks; a module that reads what another sets (for example Masa's budget
   module, which must come first) depends on that order. The smoke's stack probe
   fails if ordinary traffic is rejected.
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
   `[patch.crates-io]` Tokio/Hyper entries in the root `Cargo.toml`, and do not
   switch the framework to a path dependency on `rpcstack/` (clean checkouts
   and image builds do not have the submodule); new crates must be in
   `Cargo.lock` (CI builds `--locked`).
8. **Test what you change.** Replace the "matches the built-in queue" test when
   you change the ordering on purpose, and add a test for each new module or
   ordering rule. Numbers are only comparable between `baseline` and `policy` while
   the initial content is unchanged.

## What is not here

The HTTP API (`crates/hotel/src/frontend/http.rs`), the services, the load
generator, `config/` and the Kubernetes manifests are the application and
benchmark harness, not policy. rpcstack itself is pinned in the root
`Cargo.toml`; framework behavior that no hook can express is a framework
change, not a policy edit.
