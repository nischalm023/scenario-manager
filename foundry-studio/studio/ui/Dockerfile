# The Crucible UI as a container: the Vite build of `studio`, served by
# nginx, with the wasm viewer built into it.
#
# ----------
# wasm build
# ----------
FROM rust:1-bookworm AS wasm

# wasm-opt, the size pass build-wasm.sh takes when it is there. It comes from
# binaryen's own release and NOT from apt: bookworm ships binaryen 108 (2022),
# which does not understand everything wasm-bindgen 0.2.x emits today and
# rewrites the module into one whose function table the glue can no longer
# grow — a page that loads and then throws `WebAssembly.Table.grow(): failed
# to grow table by 4` before the viewer opens.
ARG BINARYEN_VERSION=133
RUN set -eux; \
    case "$(dpkg --print-architecture)" in \
      amd64) arch=x86_64 ;; \
      arm64) arch=aarch64 ;; \
      *) echo "no binaryen release for $(dpkg --print-architecture)" >&2; exit 1 ;; \
    esac; \
    url="https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-${arch}-linux.tar.gz"; \
    curl -fsSL "$url" | tar -xz -C /opt; \
    ln -s "/opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt" /usr/local/bin/wasm-opt; \
    wasm-opt --version

WORKDIR /src
COPY . .

# wasm-bindgen's glue only loads a module its own version produced, so the
# command-line tool is installed at the version the crate pins — read from the
# crate rather than repeated here, which is the check build-wasm.sh makes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    rustup target add wasm32-unknown-unknown \
 && cargo install -f wasm-bindgen-cli \
      --version "$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' studio/viewer/Cargo.toml)"

# The private world-proto repository comes in over SSH, through the agent `docker build --ssh` forwards.
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN ./scripts/use-world-proto-over-ssh.sh

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=ssh \
    ./studio/viewer/js/build-wasm.sh release

# ---------
# Web Build
# ---------
FROM oven/bun:1 AS web

WORKDIR /src
COPY . .

COPY --from=wasm /src/studio/viewer/js/src/wasm ./studio/viewer/js/src/wasm

ARG VITE_CRUCIBLE_WS=""
ENV VITE_CRUCIBLE_WS=${VITE_CRUCIBLE_WS}

WORKDIR /src/studio/ui
RUN bun install --frozen-lockfile

RUN bun run build

# -------
# Runtime
# -------
FROM nginx:alpine AS runtime

COPY --from=web /src/studio/ui/dist /usr/share/nginx/html
COPY studio/ui/nginx.conf.template /etc/nginx/templates/default.conf.template

ENV PORT=8080 \
    CRUCIBLE_SERVER=http://crucible:8080 \
    DNS_RESOLVER=127.0.0.11

EXPOSE 8080
