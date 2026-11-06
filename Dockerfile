# syntax=docker/dockerfile:1.7
#
# One builder compiles every hotel binary once; one runtime image carries them
# all and docker/entrypoint.sh picks the binary from the first argument.
#
#   docker build --build-arg MODE=masa -t rpcstack-hotel:masa .
#
# MODE selects the policy preset (Cargo feature of the hotel crate):
#   baseline  generated stubs use NoopHooks (no policy)
#   masa      sched_slo,abort_slo,signal_slack,est_mean_var,ac_pred
#   policy    the RPC policy of the policy/ directory (run queue + module
#             stack); starts out equal to masa
# FEATURES, if set, overrides MODE with an explicit Cargo feature list.

ARG RUST_IMAGE=rust:1-slim-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

# Dependencies are compiled in their own layer with cargo-chef so that a source
# change only recompiles the hotel crates, and so that CI can persist that layer
# (docker buildx --cache-from/--cache-to type=gha).
ARG CHEF_IMAGE=lukemathwalker/cargo-chef:0.1.72-rust-1-slim-bookworm
FROM ${CHEF_IMAGE} AS chef-bin

FROM ${RUST_IMAGE} AS chef
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git pkg-config \
 && rm -rf /var/lib/apt/lists/*
COPY --from=chef-bin /usr/local/cargo/bin/cargo-chef /usr/local/cargo/bin/cargo-chef
WORKDIR /src
# rust-toolchain.toml pins the nightly rpcstack needs; install it in its own layer.
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install

# recipe.json lists only what determines the dependency graph; it is identical
# across source edits, so the cook layer below stays cached.
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY policy policy
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# Declared late so the toolchain layers above are shared by every mode.
ARG MODE=baseline
ARG FEATURES=
ARG BUILD_JOBS=
RUN set -eu; \
    features="${FEATURES:-}"; \
    if [ -z "$features" ]; then \
      case "$MODE" in \
        baseline) features=baseline ;; \
        masa) features=masa ;; \
        policy) features=policy ;; \
        *) echo "unknown MODE=$MODE (expected baseline, masa or policy)" >&2; exit 1 ;; \
      esac; \
    fi; \
    echo "$features" > /features
COPY --from=planner /src/recipe.json recipe.json
# The policy mode compiles the run queue into rpcstack-sched, which is a
# dependency, so the queue source must exist at cook time (.cargo/config.toml
# points RPCSTACK_QUEUE_SOURCE at it; other modes ignore it).
COPY .cargo .cargo
COPY policy/queue policy/queue
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    export ${BUILD_JOBS:+CARGO_BUILD_JOBS=$BUILD_JOBS}; \
    cargo chef cook --locked --release -p hotel --features "$(cat /features)" \
      --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY policy policy
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    set -eu; \
    features="$(cat /features)"; \
    echo "building hotel with features: $features"; \
    export ${BUILD_JOBS:+CARGO_BUILD_JOBS=$BUILD_JOBS}; \
    cargo build --locked --release -p hotel --bins --features "$features"; \
    mkdir -p /out; \
    for f in target/release/hotel_* target/release/loadgen; do \
      [ -x "$f" ] && [ -f "$f" ] && cp "$f" /out/; \
    done; \
    echo "$MODE ${features}" > /out/HOTEL_MODE; \
    ls -l /out

FROM ${RUNTIME_IMAGE} AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/ /usr/local/bin/
RUN mv /usr/local/bin/HOTEL_MODE /etc/hotel-mode
COPY docker/entrypoint.sh /usr/local/bin/hotel-entrypoint
COPY config/ /etc/hotel/
ENV LOG_LEVEL=info \
    HOTEL_CONFIG=/etc/hotel/hotel.json \
    HOTEL_GEN_CONFIG=/etc/hotel/gen_config.json
ENTRYPOINT ["/usr/local/bin/hotel-entrypoint"]
CMD ["frontend"]
