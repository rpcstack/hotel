#!/usr/bin/env bash
# Runs the accuracy check on the base and on policy mutants (tests/mutants/*.patch, each a change
# of policy/ only): the base must PASS (exit 0) and every mutant FAIL (exit 1).
#
#   tests/mutants.sh TASK_IMAGE OUT_DIR [NAME...]     (NAME: base or a patch's name; default all)
#
# It needs docker, the task image (docker build -t TASK_IMAGE .vibesys/tasks/rpc-policy-k8s) and
# the cluster the environment gives: KUBECONFIG, VIBESYS_K8S_REGISTRY, VIBESYS_K8S_BUILDER (the
# task container runs on the host network), and DOCKER_CONFIG (default ~/.docker) with
# credentials for ghcr.io, where the task image's base is (README.md, "Run it"). Each candidate is
# the repository's HEAD (git archive) with the patch applied, in a fresh directory under OUT_DIR.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
image=$1 out=$2; shift 2
names=("$@")
if [ ${#names[@]} -eq 0 ]; then
  names=(base)
  for p in "$here"/mutants/*.patch; do names+=("$(basename "$p" .patch)"); done
fi
mkdir -p "$out"
status=0
for name in "${names[@]}"; do
  cand="$(mktemp -d "$out/$name.XXXX")"
  mkdir -p "$cand/project" "$cand/artifacts"
  chmod 777 "$cand/artifacts"
  git -C "$root" archive HEAD | tar -x -C "$cand/project"
  [ "$name" = base ] || patch -s -p1 -d "$cand/project" < "$here/mutants/$name.patch"
  set +e
  docker run --rm --init --network host --user "$(id -u):$(id -g)" \
    --mount "type=bind,source=$cand/project,target=/candidate,readonly" \
    --mount "type=bind,source=$KUBECONFIG,target=/vibesys/kubeconfig,readonly" \
    --mount "type=bind,source=$cand/artifacts,target=/vibesys/artifacts" \
    --mount "type=bind,source=${DOCKER_CONFIG:-$HOME/.docker},target=/docker,readonly" -e DOCKER_CONFIG=/docker \
    -e KUBECONFIG=/vibesys/kubeconfig -e VIBESYS_K8S_REGISTRY -e VIBESYS_K8S_BUILDER \
    "$image" /task/rpc-policy-k8s accuracy --project-root /candidate > "$cand/accuracy.log" 2>&1
  code=$?
  set -e
  want=1; [ "$name" = base ] && want=0
  verdict=$(case $code in 0) echo PASS;; 1) echo FAIL;; *) echo "UNKNOWN ($code)";; esac)
  echo "$name: $verdict (log: $cand/accuracy.log)"
  [ "$code" = "$want" ] || status=1
done
exit $status
