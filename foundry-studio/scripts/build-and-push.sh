#!/usr/bin/env bash
# Build this checkout's deployable images, and push them unless told not to.
#
# Four images, which is what the des chart in gm-multi-tenant-apps deploys:
# studio-api, studio-streaming and studio (the shell) as Deployments, and
# studio-simrunner, which runs as one Kubernetes Job per attempt and so is never
# deployed as a service. The Crucible server used to be built here too; nothing
# deploys it since Agent Studio replaced it, and no crate in the workspace depends
# on its own, so it is no longer built.
#
# PUSH=false builds and stops, which is what a pull request wants: proof the
# images still build, without a registry tag moving before the change is
# merged. Anything else pushes.
set -euo pipefail

REGISTRY="us-docker.pkg.dev/greymatter-development/apps"
PUSH="${PUSH:-true}"

VERSION_TAG="$(git describe --tags --always --dirty)"
REF_TAG="${GITHUB_REF_NAME:-$(git rev-parse --abbrev-ref HEAD)}"
REF_TAG="${REF_TAG//\//-}"

# What the server stamps on every recording. It has no git inside the image,
# refuses to start without these, and reads DIRTY as exactly true or false.
GIT_BRANCH="${GITHUB_REF_NAME:-$(git rev-parse --abbrev-ref HEAD)}"
GIT_COMMIT="${GITHUB_SHA:-$(git rev-parse HEAD)}"
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  GIT_DIRTY=true
else
  GIT_DIRTY=false
fi

build_and_push() {
  local service_name="$1"
  local dockerfile_path="$2"
  shift 2
  local image="${REGISTRY}/${service_name}"

  echo "========================================================="
  echo "Building ${service_name}..."
  echo "========================================================="

  # The cargo builds inside fetch the private world-proto repository through the SSH agent, when there is one.
  local ssh=()
  if [ -n "${SSH_AUTH_SOCK:-}" ]; then ssh=(--ssh default); fi

  docker build \
    --platform linux/amd64 \
    ${ssh[@]+"${ssh[@]}"} \
    -f "${dockerfile_path}" \
    -t "${image}:${VERSION_TAG}" \
    -t "${image}:${REF_TAG}" \
    "$@" \
    .

  if command -v grype >/dev/null 2>&1; then
    echo "Scanning ${service_name} for vulnerabilities..."
    grype "${image}:${VERSION_TAG}" --only-fixed --fail-on high || true
  fi

  if [ "$PUSH" != true ]; then
    echo "Built ${image}:${VERSION_TAG} (PUSH=${PUSH}; not pushing)"
    return 0
  fi

  echo "Pushing ${service_name}..."
  docker push "${image}:${VERSION_TAG}"
  docker push "${image}:${REF_TAG}"

  echo "Successfully pushed ${image}:${VERSION_TAG} and ${image}:${REF_TAG}"
}

# Agent Studio's three binaries, one Dockerfile, one cargo build shared
# between its targets: each later build finds the first's layers in the cache.
build_and_push "studio-api" "studio/Dockerfile" \
  --target api \
  --build-arg "GIT_BRANCH=${GIT_BRANCH}" \
  --build-arg "GIT_COMMIT=${GIT_COMMIT}" \
  --build-arg "GIT_DIRTY=${GIT_DIRTY}"

build_and_push "studio-simrunner" "studio/Dockerfile" \
  --target simrunner \
  --build-arg "GIT_BRANCH=${GIT_BRANCH}" \
  --build-arg "GIT_COMMIT=${GIT_COMMIT}" \
  --build-arg "GIT_DIRTY=${GIT_DIRTY}"

build_and_push "studio-streaming" "studio/Dockerfile" \
  --target streaming \
  --build-arg "GIT_BRANCH=${GIT_BRANCH}" \
  --build-arg "GIT_COMMIT=${GIT_COMMIT}" \
  --build-arg "GIT_DIRTY=${GIT_DIRTY}"

# No VITE_CRUCIBLE_WS: the page asks its own origin for /ws, and the image's
# nginx proxies that to the server named by CRUCIBLE_SERVER at runtime.
build_and_push "studio" "studio/ui/Dockerfile"

if [ "$PUSH" = true ]; then
  echo "All images built and pushed successfully!"
else
  echo "All images built successfully (nothing pushed)."
fi
