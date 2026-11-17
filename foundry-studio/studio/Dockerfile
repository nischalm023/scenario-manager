# Agent Studio's three binaries as containers, from one build: the API
# (`--target api`), the simulation runner (`--target simrunner`) and the
# streaming server (`--target streaming`). One Dockerfile because all three are
# members of the root workspace with the same path dependencies (sim/,
# worldview/, runstore/), so one cargo invocation compiles the libraries once
# for all of them; a later `docker build` of another target finds that stage in
# the layer cache. The context is the REPOSITORY ROOT, which those path
# dependencies need.

FROM rust:1-bookworm AS build

WORKDIR /src
COPY . .

# `studio-api/gcs` and `studio-streaming/gcs` link the bucket backend of the
# run-object store, which a deployment stores recordings, manifests and job
# specs in and, since live frames, each attempt's locked segments (the
# streaming server uploads them). The development build (`dev.sh`, plain
# `cargo build`) stays without it. Set --build-arg CARGO_FEATURES="" for
# images that only know the folder.
ARG CARGO_FEATURES="studio-api/gcs,studio-streaming/gcs"

# The private world-proto repository comes in over SSH, through the agent `docker build --ssh` forwards, and only when
# the revision Cargo.lock pins is not yet in cargo's cache of git checkouts, which outlives the build like its registry.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN ./scripts/use-world-proto-over-ssh.sh

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    --mount=type=ssh \
    cargo build --release -p studio-api -p simrunner -p studio-streaming \
    ${CARGO_FEATURES:+--features "$CARGO_FEATURES"} \
    && cp target/release/studio-api target/release/simrunner target/release/studio-streaming /usr/local/bin/

# What the images were built from. The API keys its floors by the commit
# (`STUDIO_BUILD_COMMIT`), having no checkout to describe; the runner hands
# the three `DESVIS_RUN_*` to dessim's harness, which reads DIRTY as exactly
# `true` or `false` and refuses anything else.
ARG GIT_BRANCH=""
ARG GIT_COMMIT=""
ARG GIT_DIRTY="false"

# ---
# API
# ---
FROM debian:bookworm-slim AS api

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 studio

COPY --from=build /usr/local/bin/studio-api /usr/local/bin/studio-api

ARG GIT_BRANCH
ARG GIT_COMMIT
ARG GIT_DIRTY
ENV STUDIO_BUILD_BRANCH=${GIT_BRANCH} \
    STUDIO_BUILD_COMMIT=${GIT_COMMIT} \
    DESVIS_RUN_BRANCH=${GIT_BRANCH} \
    DESVIS_RUN_COMMIT=${GIT_COMMIT} \
    DESVIS_RUN_DIRTY=${GIT_DIRTY}

# 0.0.0.0 on both listeners: nothing outside the container reaches a loopback
# bind. The gate-facing one is for the login gate alone; the deployment opens
# the runners' one to runner pods only. Everything else (STUDIO_ENV,
# STUDIO_DATABASE_URL, STUDIO_WPS_URL, STUDIO_RUNNER_URL, the model, and
# exactly one of CRUCIBLE_STORAGE_BUCKET / CRUCIBLE_STORAGE_DIR) is the
# deployment's to set; studio/api/README.md lists them.
ENV STUDIO_BIND=0.0.0.0:3100 \
    STUDIO_RUNNER_BIND=0.0.0.0:3101

USER studio
EXPOSE 3100 3101

ENTRYPOINT ["/usr/local/bin/studio-api"]

# ----------------
# Streaming server
# ----------------
FROM debian:bookworm-slim AS streaming

# ca-certificates for the bucket's TLS: the server uploads frames to the
# run-object store itself since live frames.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10002 streaming

COPY --from=build /usr/local/bin/studio-streaming /usr/local/bin/studio-streaming

# 0.0.0.0 on both: the runners' port is for runner pods, the viewers' for
# whoever holds a watch ticket. So the deployment must set how each is reached,
# else the server refuses to start: STUDIO_STREAMING_RUNNER_URL (the runners'
# websocket as a pod reaches it, this pod's name on a headless Service) and
# STUDIO_STREAMING_VIEWER_URL (the viewers' websocket base as a browser reaches
# it), with STUDIO_DATABASE_URL (the API's database, migrated by the API) and
# exactly one of CRUCIBLE_STORAGE_BUCKET / CRUCIBLE_STORAGE_DIR with the
# bucket's credentials, since the server uploads the live frames itself;
# studio/streaming/README.md has the rest. The cache folder below is where the
# disk copies of the frames go; mount a volume there sized for
# STUDIO_STREAMING_CACHE_MAX_BYTES.
ENV STUDIO_STREAMING_RUNNERS_BIND=0.0.0.0:3201 \
    STUDIO_STREAMING_VIEWERS_BIND=0.0.0.0:3202 \
    STUDIO_STREAMING_CACHE_DIR=/var/lib/streaming/cache

RUN mkdir -p /var/lib/streaming/cache && chown -R streaming:streaming /var/lib/streaming

USER streaming
EXPOSE 3201 3202

ENTRYPOINT ["/usr/local/bin/studio-streaming"]

# ------
# Runner
# ------
FROM debian:bookworm-slim AS simrunner

# ca-certificates for outbound TLS; hostname, which dessim's harness asks for
# the run's identity when DESVIS_RUN_HOSTNAME is unset.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates hostname \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 simrunner

COPY --from=build /usr/local/bin/simrunner /usr/local/bin/simrunner

ARG GIT_BRANCH
ARG GIT_COMMIT
ARG GIT_DIRTY
ENV DESVIS_RUN_BRANCH=${GIT_BRANCH} \
    DESVIS_RUN_COMMIT=${GIT_COMMIT} \
    DESVIS_RUN_DIRTY=${GIT_DIRTY}

# `simrunner --job <job.json> --out <folder>`, its token in STUDIO_RUNNER_TOKEN
# (never on the command line): the Job or container that runs this image mounts
# the spec and sets the token. The folder below is where it writes.
RUN mkdir -p /var/lib/simrunner/out && chown -R simrunner:simrunner /var/lib/simrunner

USER simrunner
WORKDIR /var/lib/simrunner

ENTRYPOINT ["/usr/local/bin/simrunner"]
CMD ["--job", "/var/lib/simrunner/job.json", "--out", "/var/lib/simrunner/out"]

# The source the image was built from, uncommitted changes and all, as scripts/source-identity.sh names it: dev.sh
# rebuilds a local image, and the API refuses one, whose source is not the checkout's. Last, so it caches nothing away.
ARG GIT_SOURCE=""
LABEL agent-studio/source=${GIT_SOURCE}
