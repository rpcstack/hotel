# The rpc-policy-k8s task image: every job of the task runs in it, and so do its pods in the
# cluster (the probe and phase pods of a benchmark sample or an accuracy check), which run this
# image rebuilt in-cluster from the task directory (/task).
#
# It is vibesys-evaluator's microservices image at the release 0.3.0rc2 (by tag and digest:
# loadgen, kubectl and buildctl on the PATH), with the task's Go programs, built from its
# evaluator/ module, whose go.mod requires the Go SDK at the same release, v0.3.0-rc.2, in /task:
# rpc-policy-k8s (the benchmark and the accuracy check), oracle (judges a captured history
# offline), scope (writes or checks scope.json) and explore (the benchmark at other loads).
FROM golang:1.25.9-bookworm@sha256:298734aec230b5f3e8cee450ce6d7eccc39f1797ba548ee90d57e9803030c6c3 AS evaluator
ENV CGO_ENABLED=0 GOFLAGS=-trimpath GOTOOLCHAIN=local
COPY evaluator/go.mod evaluator/go.sum /build/evaluator/
RUN cd /build/evaluator && go mod download
COPY evaluator/ /build/evaluator/
RUN cd /build/evaluator && go build -o /out/ ./cmd/rpc-policy-k8s ./cmd/oracle ./cmd/scope ./cmd/explore

FROM ghcr.io/uw-syfi/vibesys-evaluator-microservices:0.3.0rc2@sha256:64dc4a0dfe63507da08080c6e8925e9e7622d268bf200b71c817821f00f1140e
COPY . /task
COPY --from=evaluator /out/rpc-policy-k8s /out/oracle /out/scope /out/explore /task/
