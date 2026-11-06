# Template: an application repository on rpcstack

This repository is the reference layout for porting a benchmark application
(socialnet, synthbench, tracebench, ...) onto rpcstack. Copy these pieces.

## 1. Workspace and dependency pin

* `Cargo.toml` is a workspace (`crates/<app>`, `crates/app-utils`, ...).
* Pin rpcstack once in `[workspace.dependencies]` as a git dependency at an
  exact `rev` (`masa`, `tonic`, `tonic-build` with `prost`, `tokio`); members use
  `{ workspace = true }`. No path dependencies outside the repository.
* Repeat the same rev in `[patch.crates-io]` for `tokio` and `hyper` (Cargo does
  not let patches inherit workspace deps, and patches do not propagate from
  rpcstack's own workspace). `scripts/check.sh` fails if the revs differ, if
  `Cargo.lock` resolves two revs, or if the graph has a second tokio/hyper.
* Copy rpcstack's `rust-toolchain.toml`. Commit `Cargo.lock`; build `--locked`.
* If resolution picks a newer `tokio-util`, pin `tokio-util = "=0.7.11"`
  (the vendored Hyper requires it).

## 2. The hooks-path rule

rpcstack's `tonic-build` generates stubs whose hook parameter defaults to
`tonic::rpcstack::noop::NoopHooks`. **Every** `tonic_build::configure()` call
must set `.default_hooks_path("masa::DefaultHooks")` (or your stack's hooks),
otherwise that service silently runs without the policy. Generate all protos
through one helper in `build.rs` (see `crates/hotel/build.rs`) and keep a test
like `crates/hotel/tests/hooks.rs`: it checks every client type and every
generated source's defaults, and that the mode's hooks are not `NoopHooks` when
a policy feature is on. `build.rs` also asserts that each generated stub names
the expected hooks twice and never `NoopHooks`, so a stub cannot silently fall
back to no-op hooks.

An optimization-agent variant keeps the policy in one crate: hotel's `policy/`
defines `Hooks = PolicyHooks<Stack>` and a run queue (`sched_custom` +
`RPCSTACK_QUEUE_SOURCE`, set in `.cargo/config.toml`); `build.rs` points
`default_hooks_path` at `hotel_policy::Hooks` in `--features policy` builds.
Build modes one at a time with explicit packages, not `--workspace`: the policy
crate enables Masa features that would otherwise unify into a baseline build.

## 3. Modes as Cargo features

`baseline = []`, `masa = [<preset>]` (and optionally `policy = ["dep:<app>-policy"]`) on the application crate, plus the
individual Masa features passed through. The mode flows Cargo feature →
Docker build arg `MODE` (or `FEATURES`) → Compose `MODE` env / k8s overlay.

## 4. Docker

One multi-stage `Dockerfile`: a builder compiling all binaries once with
BuildKit cache mounts (registry, git, target per mode), and one runtime image
holding every binary. `docker/entrypoint.sh` maps the first argument (service
name, `client-bench`, `probe`, `mode`) to a binary and its config path. Configs
are baked in at `/etc/<app>` and overridden by a mount of `config/`.

## 5. Compose and Kubernetes

* `docker-compose.yaml`: one service per process, all using the same image
  (`build:` on one service, `pull_policy: never` on the rest), backends with
  healthchecks and `depends_on: service_healthy`, the load generator as a
  one-shot service. Service names are the hostnames in `config/<app>.json`.
* `k8s/base`: a Deployment + Service per process with the same names, a
  ConfigMap generated from `config/` (fixed name via
  `disableNameSuffixHash`, so smoke Jobs can mount it), image `localhost:5001/<app>:baseline`;
  `k8s/overlays/masa` changes only the tag. Scripts: `kind-up.sh` (kind +
  `registry:2`), `k8s-deploy.sh [mode]`, `k8s-teardown.sh [--cluster]`,
  `kind-down.sh`. Use unique cluster/registry/project names. Pods may restart once
  while their databases start; the k8s smoke only counts restarts after
  readiness.

### Variant: generated topologies

Applications whose process graph is generated (synthbench topologies,
tracebench graphs) may skip `k8s/base` and overlays. A generator
(`scripts/gen-deploy.py`, `scripts/gen-all.sh`) writes `k8s.yaml` and
`k8s-loadgen.yaml` (plus a Compose file where needed) into `deploy/<topology>/`, with the image tag
rewritten per mode at apply time (`scripts/lib/render-k8s.sh`). Everything else
is unchanged: same `kind-up.sh`, `k8s-deploy.sh`, teardown scripts and smoke
tests, and the README must state the layout and the exact one-command deploy
for Compose and Kubernetes.

## 6. Smoke tests

`scripts/smoke-compose.sh [mode]` and `scripts/smoke-k8s.sh [mode]` must:
check readiness, run moderate load with zero transport errors and a stated
success threshold, and prove the policy is live with a check that **fails in
baseline**. Hotel's probe sends each service a request whose Masa deadline has
already expired and requires `/EarlyReturn?src=<that service>` in masa mode
and none in baseline. In baseline mode the smoke also runs the probe with
`--expect masa` as a **negative control that must fail** (it exits non-zero
with `PROBE FAIL`), so a probe that cannot distinguish the modes breaks the
smoke. Add an overload burst (masa must reject some requests)
and a recovery run, and fail on panics or restarts. `scripts/lib/check_traces.py`
classifies load-generator traces and is reusable as is.

## 7. CI

`.github/workflows/ci.yml`: fmt and `scripts/check.sh`, `scripts/test.sh` in
both modes, Docker build per mode, Compose smoke per mode. Kubernetes smoke is
a manual (`workflow_dispatch`) / nightly workflow.
