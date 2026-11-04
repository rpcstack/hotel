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
# FEATURES, if set, overrides MODE with an explicit Cargo feature list.

ARG RUST_IMAGE=rust:1-slim-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
ARG MODE=baseline
ARG FEATURES=
ARG BUILD_JOBS=
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates git pkg-config \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# rust-toolchain.toml pins the nightly rpcstack needs; install it in its own layer.
COPY rust-toolchain.toml ./
RUN rustup show active-toolchain || rustup toolchain install
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target,id=hotel-target-${MODE} \
    set -eu; \
    features="${FEATURES:-}"; \
    if [ -z "$features" ]; then \
      case "$MODE" in \
        baseline) features=baseline ;; \
        masa) features=masa ;; \
        *) echo "unknown MODE=$MODE (expected baseline or masa)" >&2; exit 1 ;; \
      esac; \
    fi; \
    echo "building hotel with features: $features"; \
    cargo build --locked --release -p hotel --bins --features "$features" ${BUILD_JOBS:+-j $BUILD_JOBS}; \
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
