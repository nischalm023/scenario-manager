#!/bin/bash
# Agent Studio's local stack for the proof of concept, brought up by one command on this machine (fwd D109: no
# Kubernetes, no login gate), so that the API, the browser shell and the world-proto server they read and write
# can be tried together without assembling them by hand. In order, each once the one before it answers:
# PostgreSQL in a throwaway container, world-proto-server built from the world-proto checkout and seeded by
# scripts/seed-dev.sh with Foundry's application and the walmart-dallas demo site and then, when
# data/seed/studio/manifest.json exists, by studio-seed with Agent Studio's
# templates and catalog items (spec 04 section 3.13) and after them the customer sites data/seed/customer/manifest.json
# lists, the API (studio/api) with its runners' listener, built in
# Cargo.toml's studio-dev profile (its libraries optimised, since a template evaluation's wall clock is spent in
# them), told the world-proto commit it was built from as STUDIO_WPS_REVISION and given a run-object store under
# studio/.dev-state/objects (CRUCIBLE_STORAGE_DIR), its runs started as containers of this checkout's runner image
# studio-simrunner:dev-<its key> (studio/Dockerfile's simrunner target, built here unless the engine holds it built from
# the source this checkout holds, uncommitted changes and all, which the API refuses to start without; spec 08 section
# 3.5), labelled with this stack's key and run by the same engine that runs PostgreSQL, their output folders under
# studio/.dev-state/runner-out, the streaming server (studio/streaming) the runs stream their progress and frames to
# live, registered in the API's database, reached by the containers at the host's alias and by the browser on the
# loopback, keeping its disk copies of the frames under studio/.dev-state/streaming-cache, and Vite's dev server over
# studio/ui/, which hands /api to the API as the login gate will in the cluster, once the floor viewer's wasm is built
# (studio/viewer/js/build-wasm.sh on every start, cargo rebuilding only what changed). Beside world-proto-server,
# world-proto's own browser UI (the wpsui unit) runs in place from that checkout's server/ui with the commands
# world-proto's server/dev.sh runs, reading this stack's server.
#
#   ./dev.sh [start] [--no-wps] [--no-seed]        build, start, print the configuration and the addresses
#   ./dev.sh --status                               one line per unit; exits 1 when down or a unit has failed
#   ./dev.sh --logs [unit]                          follow a log: api (default), wps, wpsui, seed, streaming, ui,
#                                                   postgres, supervisor
#   ./dev.sh --stop [--db] [--runners]              stop the stack; --runners removes this stack's runs'
#                                                   containers, which otherwise go on; --db removes them too,
#                                                   and once they are gone the container, its rows, the
#                                                   run-object store's folder and the runs' output folders
#
# --no-wps starts no world-proto-server and no world-proto UI, and the API is told STUDIO_WPS_URL as set or its own
# default, where one must answer, since the API refuses to start without it; --no-seed starts it unseeded, by either
# seed. Each command also answers without its dashes, as the other dev.sh scripts spell them.
#
# After a change to a core schema or one of Agent Studio's component definitions, or to a seeded document whose
# manifest also gains an entry that can reference it, a start on the database an older seed filled stops at the seed
# with a reset hint, and world-proto-server's database must be seeded afresh. Where that database is in this stack's
# Postgres container, ./dev.sh --stop --db deletes the stack's runs and that container, the API's rows and
# world-proto-server's in it, and the next start seeds afresh. ./dev.sh --stop --db deletes nothing kept on a server
# that STUDIO_DATABASE_URL or WORLD_PROTO_DATABASE_URL names, so a world-proto-server database kept there must be reset
# there before the next start.
#
# Read from the environment (the API's own variables are in studio/api/README.md):
#   WORLD_PROTO_DIR           the world-proto checkout, relative to the foundry checkout's root: ../world-proto
#                             by default, looked for beside the main checkout when this is a linked worktree
#   STUDIO_DATABASE_URL       a Postgres server of your own for the API, instead of the container
#   WORLD_PROTO_DATABASE_URL  world-proto-server's database; by default a world_graph database beside the API's
#   STUDIO_DEVELOPERS         the developers; by default the local identity every request is (dev), so the
#                             developer-only controls show; set it empty to see the pages as anyone else does
#   STUDIO_*                  passed to the API as set; what dev.sh sets itself is printed at start
#
# Like studio/runs/dev.sh and world-proto's server/dev.sh, it publishes itself to Sutra: a meta file under
# ~/.dev-runner/ named by a hash of this folder's path, one status file per unit, and a supervisor leading its
# own process group, so the one SIGTERM that --stop or Sutra's terminate button sends takes every unit down.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$SCRIPT_DIR"
LOG_DIR="$REPO_ROOT/studio/.dev-log"
STATE_DIR="$REPO_ROOT/studio/.dev-state"
PORTS_FILE="$STATE_DIR/ports"
RUN_FILE="$STATE_DIR/run"
FAILURE_FILE="$STATE_DIR/failure"

# Guarded, because an empty key would turn the cleanup globs below onto every other runner's files.
REGISTRY_KEY=$(printf '%s' "$SCRIPT_DIR" | shasum -a 256 2>/dev/null | cut -c1-16)
if [ ${#REGISTRY_KEY} -lt 8 ]; then
    echo "dev.sh: could not derive a key for $SCRIPT_DIR; is shasum on the PATH?" >&2
    exit 1
fi
REGISTRY_DIR="$HOME/.dev-runner"
META_FILE="$REGISTRY_DIR/$REGISTRY_KEY"
UNITS="postgres wps wpsui seed api runners streaming ui"

PG_CONTAINER="studio-pg-$REGISTRY_KEY"
PG_IMAGE="postgres:17"
PG_USER="postgres"
PG_PASSWORD="postgres"
PG_DB="studio"
WPS_DB="world_graph"

WITH_WPS=true
WITH_SEED=true
# Per checkout, as the Postgres container is, so two checkouts' stacks never build over each other's image.
RUNNER_IMAGE="studio-simrunner:dev-$REGISTRY_KEY"
ENGINE=""
DB_MODE=""
POSTGRES_PORT=""
API_PORT=""
RUNNERS_PORT=""
WPS_PORT=""
UI_PORT=""
WPSUI_PORT=""
STREAMING_INTERNAL_PORT=""
STREAMING_PORT=""
WPSUI_DIR=""
API_DATABASE_URL=""
WPS_DATABASE_URL=""
WPS_COMMIT=""
WPS_REVISION=""
API_BINARY=""
STREAMING_BINARY=""
SEED_BINARY=""
OVERRIDDEN=""

# ---------------------------------------------------------------------------
# What this folder tells Sutra
# ---------------------------------------------------------------------------

# Written aside and renamed, never with a bare `>`, so a reader never meets a truncated line; and not at all
# once the supervisor is tearing down, when a unit dying of its signal would write over the cleared registry.
update_status() {  # <unit> <state[: detail]>
    [ -e "$STATE_DIR/stopping" ] && return 0
    local file="$REGISTRY_DIR/$REGISTRY_KEY.$1.status"
    printf '%s\n' "$2" > "$file.tmp" && mv -f "$file.tmp" "$file"
}

read_status() {
    cat "$REGISTRY_DIR/$REGISTRY_KEY.$1.status" 2>/dev/null || echo "-"
}

clear_status() {
    rm -f "$REGISTRY_DIR/$REGISTRY_KEY".*.status "$REGISTRY_DIR/$REGISTRY_KEY".*.status.tmp
}

# A unit that gives up says why in its status line, which Sutra shows, and first in the failure file, which
# outlives the supervisor's cleanup so that `start` can still say what went wrong once the stack is down.
unit_failed() {  # <unit> <reason>
    printf '%s: %s\n' "$1" "$2" > "$FAILURE_FILE"
    update_status "$1" "failed: $2"
    echo "[$1] $2"
}

write_meta() {  # <supervisor pid>
    {
        echo "DIR=$SCRIPT_DIR"
        echo "PID=$1"
        echo "STARTED=$(date +%s)"
        if [ "$DB_MODE" = container ]; then echo "POSTGRES_PORT=$POSTGRES_PORT"; fi
        if [ "$WITH_WPS" = true ]; then echo "WPS_PORT=$WPS_PORT"; fi
        if [ -n "$WPSUI_DIR" ]; then echo "WPSUI_PORT=$WPSUI_PORT"; fi
        echo "API_PORT=$API_PORT"
        echo "RUNNERS_PORT=$RUNNERS_PORT"
        echo "STREAMING_INTERNAL_PORT=$STREAMING_INTERNAL_PORT"
        echo "STREAMING_PORT=$STREAMING_PORT"
        echo "UI_PORT=$UI_PORT"
    } > "$META_FILE.tmp" && mv -f "$META_FILE.tmp" "$META_FILE"
}

meta_value() {
    grep "^$1=" "$META_FILE" 2>/dev/null | head -1 | cut -d= -f2-
}

file_value() {  # <file> <key>
    grep "^$2=" "$1" 2>/dev/null | head -1 | cut -d= -f2-
}

process_cwd() {  # <pid>
    if [ -r "/proc/$1/cwd" ]; then
        readlink -f "/proc/$1/cwd" 2>/dev/null
    else
        lsof -a -d cwd -p "$1" -Fn 2>/dev/null | sed -n 's/^n//p' | head -1
    fi
}

# Whether a recorded PID is still this folder's supervisor: alive, leading its own process group, running this
# script and standing in this folder. A PID failing any of the four has been handed to another process.
is_our_supervisor() {  # <pid>
    [ -n "$1" ] && kill -0 "$1" 2>/dev/null || return 1
    [ "$(ps -o pgid= -p "$1" 2>/dev/null | tr -d ' ')" = "$1" ] || return 1
    case "$(ps -o args= -p "$1" 2>/dev/null)" in *dev.sh*) ;; *) return 1 ;; esac
    [ "$(process_cwd "$1")" = "$SCRIPT_DIR" ]
}

# What a supervisor killed outright leaves running: its process group outlives it while any member does, and the
# kernel hands out no id a living group still uses, so the members of the recorded group are this folder's own.
orphans_of() {  # <pid>
    [ -n "$1" ] && ! kill -0 "$1" 2>/dev/null && pgrep -g "$1" >/dev/null 2>&1
}

supervisor_running() {
    [ -f "$META_FILE" ] || return 1
    local pid
    pid=$(meta_value PID)
    is_our_supervisor "$pid" && return 0
    orphans_of "$pid" && return 1
    rm -f "$META_FILE"
    clear_status
    return 1
}

# ---------------------------------------------------------------------------
# Tools, ports and the world-proto checkout
# ---------------------------------------------------------------------------

container_engine() {
    if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
        echo docker
    elif command -v podman >/dev/null 2>&1 && podman info >/dev/null 2>&1; then
        echo podman
    fi
}

ui_package_manager() {
    if command -v bun >/dev/null 2>&1; then echo bun; elif command -v npm >/dev/null 2>&1; then echo npm; fi
}

preflight() {
    local missing=()
    command -v cargo >/dev/null 2>&1 || missing+=("cargo: install Rust from https://rustup.rs")
    command -v curl >/dev/null 2>&1 || missing+=("curl: every readiness probe here is a request")
    command -v python3 >/dev/null 2>&1 || missing+=("python3: a free port is found by binding it, and the seed runs it")
    command -v lsof >/dev/null 2>&1 || [ -d /proc ] || missing+=("lsof: the supervisor is told apart by the folder it runs in")
    if [ "$DB_MODE" = container ] && [ -z "$ENGINE" ]; then
        missing+=("Docker or Podman, started: Postgres runs in a container unless STUDIO_DATABASE_URL names a server")
    fi
    if [ "$WITH_WPS" = true ] && [ "$WITH_SEED" = true ]; then
        command -v jq >/dev/null 2>&1 || missing+=("jq: the seed reads every answer with it")
    fi
    [ -n "$(ui_package_manager)" ] || missing+=("bun (https://bun.sh) or npm: the dev server's dependencies")
    command -v wasm-bindgen >/dev/null 2>&1 || missing+=("wasm-bindgen: the floor viewer's wasm; cargo install -f \
wasm-bindgen-cli --version $(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' "$REPO_ROOT/studio/viewer/Cargo.toml")")
    # Vite's binary starts with `#!/usr/bin/env node`, and a version manager's shim is found yet fails to run.
    node --version >/dev/null 2>&1 || missing+=("a node that runs: Vite's binary starts with #!/usr/bin/env node")
    [ ${#missing[@]} -eq 0 ] && return 0
    echo "dev.sh: Agent Studio's stack cannot be brought up without:" >&2
    printf '  %s\n' "${missing[@]}" >&2
    return 1
}

# Whether every port given can be taken, asked by binding it rather than by looking for a listener, on the loopback
# and on every interface, with address reuse as the servers bind (world-proto's server/dev.sh gives the reasons).
ports_free() {  # <port>...
    python3 - "$@" <<'PY' 2>/dev/null
import socket, sys
for port in sys.argv[1:]:
    for host in ("127.0.0.1", "0.0.0.0"):
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            s.bind((host, int(port)))
            s.listen(1)
        except OSError:
            sys.exit(1)
        finally:
            s.close()
PY
}

# The last start's seven ports while they are free, so that open tabs find the stack again after a restart, and
# otherwise seven in a row from a random place in 20000-29999; a ports file from before world-proto's UI or the
# streaming server had ports gives them the ones after the first four.
choose_ports() {
    API_PORT=$(file_value "$PORTS_FILE" API_PORT)
    RUNNERS_PORT=$(file_value "$PORTS_FILE" RUNNERS_PORT)
    WPS_PORT=$(file_value "$PORTS_FILE" WPS_PORT)
    UI_PORT=$(file_value "$PORTS_FILE" UI_PORT)
    WPSUI_PORT=$(file_value "$PORTS_FILE" WPSUI_PORT)
    STREAMING_INTERNAL_PORT=$(file_value "$PORTS_FILE" STREAMING_INTERNAL_PORT)
    STREAMING_PORT=$(file_value "$PORTS_FILE" STREAMING_PORT)
    if [ -n "$API_PORT" ] && [ -z "$WPSUI_PORT" ]; then WPSUI_PORT=$((API_PORT + 4)); fi
    if [ -n "$API_PORT" ] && [ -z "$STREAMING_INTERNAL_PORT" ]; then STREAMING_INTERNAL_PORT=$((API_PORT + 5)); fi
    if [ -n "$API_PORT" ] && [ -z "$STREAMING_PORT" ]; then STREAMING_PORT=$((API_PORT + 6)); fi
    if [ -n "$API_PORT" ] && [ -n "$RUNNERS_PORT" ] && [ -n "$WPS_PORT" ] && [ -n "$UI_PORT" ] \
        && ports_free "$API_PORT" "$RUNNERS_PORT" "$WPS_PORT" "$UI_PORT" "$WPSUI_PORT" "$STREAMING_INTERNAL_PORT" \
            "$STREAMING_PORT"; then
        return 0
    fi
    local i=0 base
    while [ "$i" -lt 50 ]; do
        base=$(( 20000 + RANDOM % 1400 * 7 ))
        if ports_free "$base" $((base + 1)) $((base + 2)) $((base + 3)) $((base + 4)) $((base + 5)) $((base + 6)); then
            API_PORT=$base RUNNERS_PORT=$((base + 1)) WPS_PORT=$((base + 2)) UI_PORT=$((base + 3))
            WPSUI_PORT=$((base + 4)) STREAMING_INTERNAL_PORT=$((base + 5)) STREAMING_PORT=$((base + 6))
            return 0
        fi
        i=$((i + 1))
    done
    echo "dev.sh: no five free ports in a row in 20000-29999 after 50 tries" >&2
    return 1
}

# The port the container was made with, which is how a start finds its rows again; for a new container, a random
# free port in 30000-49999 that no other container, running or stopped, was made to publish either.
choose_postgres_port() {
    POSTGRES_PORT=$("$ENGINE" inspect -f '{{with index .HostConfig.PortBindings "5432/tcp"}}{{(index . 0).HostPort}}{{end}}' \
        "$PG_CONTAINER" 2>/dev/null)
    [ -n "$POSTGRES_PORT" ] && return 0
    local ids claimed="" i=0 port
    ids=$("$ENGINE" ps -aq 2>/dev/null)
    if [ -n "$ids" ]; then
        # shellcheck disable=SC2086 # one container id per word
        claimed=$("$ENGINE" inspect -f '{{range $p, $b := .HostConfig.PortBindings}}{{range $b}}{{.HostPort}} {{end}}{{end}}' \
            $ids 2>/dev/null | tr '\n' ' ')
    fi
    while [ "$i" -lt 50 ]; do
        port=$(( 30000 + RANDOM % 20000 ))
        case " $claimed " in
            *" $port "*) ;;
            *) if ports_free "$port"; then POSTGRES_PORT=$port; return 0; fi ;;
        esac
        i=$((i + 1))
    done
    echo "dev.sh: no free port for Postgres in 30000-49999 after 50 tries" >&2
    return 1
}

resolve_world_proto() {
    local wanted="${WORLD_PROTO_DIR:-../world-proto}" candidate common main
    case "$wanted" in /*) candidate="$wanted" ;; *) candidate="$REPO_ROOT/$wanted" ;; esac
    if [ ! -d "$candidate" ] && [ -z "${WORLD_PROTO_DIR:-}" ]; then
        common=$(cd "$REPO_ROOT" && git rev-parse --git-common-dir 2>/dev/null)
        main=$([ -n "$common" ] && cd "$REPO_ROOT" && cd "$common/.." 2>/dev/null && pwd -P)
        [ -n "$main" ] && candidate="$main/$wanted"
    fi
    if [ ! -f "$candidate/server/server/Cargo.toml" ]; then
        echo "dev.sh: no world-proto checkout at $candidate; set WORLD_PROTO_DIR, or start with --no-wps" >&2
        return 1
    fi
    WORLD_PROTO_DIR=$(cd "$candidate" && pwd -P)
    WPS_REVISION=$(git -C "$WORLD_PROTO_DIR" rev-parse --short HEAD 2>/dev/null)
    WPS_COMMIT=${WPS_REVISION:-an unknown commit}
    if [ -d "$WORLD_PROTO_DIR/server/ui" ]; then WPSUI_DIR="$WORLD_PROTO_DIR/server/ui"; fi
}

describe_world_proto() {
    local branch behind dirty=""
    branch=$(git -C "$WORLD_PROTO_DIR" rev-parse --abbrev-ref HEAD 2>/dev/null)
    behind=$(git -C "$WORLD_PROTO_DIR" rev-list --count HEAD..origin/master 2>/dev/null)
    [ -z "$(git -C "$WORLD_PROTO_DIR" status --porcelain --untracked-files=no 2>/dev/null)" ] \
        || dirty=", with uncommitted changes"
    printf '%s at %s%s' "${branch:-no branch}" "$WPS_COMMIT" "$dirty"
    if [ "${behind:-0}" -gt 0 ]; then printf '; %s commits behind origin/master as last fetched' "$behind"; fi
    # Read from its router: the API speaks only `/scopes/<path>/-/<operation>` (spec 04 section 3.12), which
    # world-proto has served since c384e59, answering the older spelling with 404.
    if grep -q '"/scopes/{p1}/{p2}/' "$WORLD_PROTO_DIR/server/server/src/api/mod.rs" 2>/dev/null; then
        printf '; its server spells scope routes /scopes/{p1}/{p2}/..., which the API does not speak'
    fi
}

# The local identity's subject, and whether STUDIO_DEVELOPERS names it, each read as the API reads them.
dev_subject() {
    printf '%s' "${STUDIO_DEV_IDENTITY%%,*}" | sed 's/^ *//; s/ *$//'
}

dev_is_developer() {
    printf '%s\n' "${STUDIO_DEVELOPERS:-}" | tr ',' '\n' | sed 's/^ *//; s/ *$//' | grep -qxF "$(dev_subject)"
}

mask_url() {
    printf '%s' "$1" | sed -E 's#^([A-Za-z][A-Za-z0-9+.-]*://[^:/@]*):[^@/]*@#\1:****@#'
}

with_database() {  # <postgres url> <database>: the same server, another database
    printf '%s' "$1" | sed -E "s#^([^?]*://[^/?]*)(/[^?]*)?#\\1/$2#"
}

# A variable whose value is dev.sh's to decide; a different value in the environment is named as overridden.
own() {  # <name> <value>
    local was
    eval "was=\${$1:-}"
    if [ -n "$was" ] && [ "$was" != "$2" ]; then OVERRIDDEN="$OVERRIDDEN $1"; fi
    export "$1=$2"
}

configure() {
    local email
    if [ "$DB_MODE" = external ]; then
        API_DATABASE_URL="$STUDIO_DATABASE_URL"
    else
        API_DATABASE_URL="postgres://$PG_USER:$PG_PASSWORD@127.0.0.1:$POSTGRES_PORT/$PG_DB"
    fi
    WPS_DATABASE_URL="${WORLD_PROTO_DATABASE_URL:-$(with_database "$API_DATABASE_URL" "$WPS_DB")}"
    export STUDIO_ENV="${STUDIO_ENV:-local}"
    # The run-object store's folder, where evaluations keep their plans and floors (spec 05 section 3.7) and each run
    # attempt its job spec, manifest and recording (spec 08 section 3.5).
    own CRUCIBLE_STORAGE_DIR "$STATE_DIR/objects"
    mkdir -p "$CRUCIBLE_STORAGE_DIR"
    # The local invoker's runner image, run by the engine as a container per attempt, the folder each container's
    # output is mounted from, and the API and world-proto-server as a container reaches them: the host's alias, and
    # on Linux (where the alias is the bridge, not loopback) listeners on every interface and the container running as
    # this user, whose folder it writes. The identity dessim's harness records is baked into the image at its build.
    own STUDIO_RUNNER_IMAGE "$RUNNER_IMAGE"
    own STUDIO_CONTAINER_ENGINE "$ENGINE"
    own STUDIO_CONTAINER_STACK "$REGISTRY_KEY"
    own STUDIO_RUNNER_OUT_DIR "$STATE_DIR/runner-out"
    mkdir -p "$STUDIO_RUNNER_OUT_DIR"
    HOST_ALIAS=host.docker.internal
    if [ "$ENGINE" = podman ]; then HOST_ALIAS=host.containers.internal; fi
    own STUDIO_RUNNER_URL "http://$HOST_ALIAS:$RUNNERS_PORT"
    if [ "$WITH_WPS" = true ]; then own STUDIO_RUNNER_WPS_URL "http://$HOST_ALIAS:$WPS_PORT"; fi
    LISTEN_HOST=127.0.0.1
    if [ "$(uname -s)" = Linux ]; then
        LISTEN_HOST=0.0.0.0
        own STUDIO_CONTAINER_USER "$(id -u):$(id -g)"
    fi
    # The streaming server: its runners' port where the containers reach it (the host's alias, so bound as the
    # runners' listener is), its viewers' port on the loopback, and the address it registers for the dispatcher.
    own STUDIO_STREAMING_RUNNERS_BIND "$LISTEN_HOST:$STREAMING_INTERNAL_PORT"
    own STUDIO_STREAMING_VIEWERS_BIND "127.0.0.1:$STREAMING_PORT"
    own STUDIO_STREAMING_RUNNER_URL "ws://$HOST_ALIAS:$STREAMING_INTERNAL_PORT/runners"
    # Live frames: the folder it keeps its disk copies of the frames in. It opens the run-object store at
    # CRUCIBLE_STORAGE_DIR, set above as the API has it, and inherits it with the rest of this environment.
    own STUDIO_STREAMING_CACHE_DIR "$STATE_DIR/streaming-cache"
    mkdir -p "$STUDIO_STREAMING_CACHE_DIR"
    own DESVIS_RUN_BRANCH "$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
    own DESVIS_RUN_COMMIT "$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
    if [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
        own DESVIS_RUN_DIRTY true
    else
        own DESVIS_RUN_DIRTY false
    fi
    own STUDIO_BIND "127.0.0.1:$API_PORT"
    own STUDIO_RUNNER_BIND "$LISTEN_HOST:$RUNNERS_PORT"
    if [ "$WITH_WPS" = true ]; then
        own STUDIO_WPS_URL "http://127.0.0.1:$WPS_PORT"
        own STUDIO_WPS_ROUTES path
        # The client's start-up probe holds the server to what this revision is known to have landed (spec 04
        # section 3.12); a checkout git cannot name leaves it to the probe alone.
        if [ -n "$WPS_REVISION" ]; then own STUDIO_WPS_REVISION "$WPS_REVISION"; fi
    fi
    # The local identity's email is the developer's own from git, when git has one (spec 01 section 3.1).
    if [ -z "${STUDIO_DEV_IDENTITY:-}" ]; then
        email=$(git -C "$REPO_ROOT" config user.email 2>/dev/null | tr -d ', ')
        export STUDIO_DEV_IDENTITY="dev,${email:-dev@localhost},Developer"
    fi
    # Unset, the local identity is the developer; set empty, nobody is, since the API refuses a blank value.
    if [ -z "${STUDIO_DEVELOPERS+set}" ]; then
        STUDIO_DEVELOPERS=$(dev_subject)
        export STUDIO_DEVELOPERS
    elif [ -z "$STUDIO_DEVELOPERS" ]; then
        unset STUDIO_DEVELOPERS
    fi
    # The agent's model (ruling D122): Gemini by default, its key the first line of the file the API reads on start,
    # which dev.sh names and never prints; Anthropic with STUDIO_MODEL_TRANSPORT=anthropic and ANTHROPIC_API_KEY.
    # The API takes each provider's model id by default (gemini-2.5-flash, claude-sonnet-5-5).
    export STUDIO_MODEL_TRANSPORT="${STUDIO_MODEL_TRANSPORT:-gemini}"
    if [ "$STUDIO_MODEL_TRANSPORT" = gemini ]; then
        export STUDIO_GEMINI_API_KEY_FILE="${STUDIO_GEMINI_API_KEY_FILE:-$HOME/.config/agent-studio/secrets/google-api-keys.txt}"
    fi
}

# What the agent runs on, never printing a key: the provider, its model as the API will name it, and whether the key
# it needs is in place (for Gemini, a first line that is not blank).
describe_agent() {
    local model key
    case "$STUDIO_MODEL_TRANSPORT" in
        gemini)
            model="${STUDIO_MODEL_ID:-gemini-2.5-flash}"
            if [ -s "$STUDIO_GEMINI_API_KEY_FILE" ] && head -n 1 "$STUDIO_GEMINI_API_KEY_FILE" | grep -q '[^[:space:]]'; then
                key="its key file holds a key: $STUDIO_GEMINI_API_KEY_FILE"
            else
                key="NO KEY at $STUDIO_GEMINI_API_KEY_FILE: a message answers model-unavailable"
            fi
            echo "  the agent: provider gemini, model $model (Gemini's id); $key" ;;
        anthropic)
            model="${STUDIO_MODEL_ID:-claude-sonnet-5-5}"
            if [ -n "${ANTHROPIC_API_KEY:-}" ]; then key="ANTHROPIC_API_KEY is set"; else key="ANTHROPIC_API_KEY is NOT set"; fi
            echo "  the agent: provider anthropic, model $model (Anthropic's id); $key" ;;
        *)
            echo "  the agent: provider $STUDIO_MODEL_TRANSPORT, which this build does not call (task 52)" ;;
    esac
}

print_configuration() {
    local line name who
    echo "Agent Studio dev runner for $SCRIPT_DIR ($REGISTRY_KEY)"
    if [ "$WITH_WPS" = true ]; then
        echo "  WORLD_PROTO_DIR=$WORLD_PROTO_DIR"
        echo "    $(describe_world_proto)"
        echo "  world-proto-server's DATABASE_URL=$(mask_url "$WPS_DATABASE_URL")"
        if [ "$WITH_SEED" = false ]; then
            echo "    both seeds are skipped (--no-seed)"
        elif [ -f "$REPO_ROOT/data/seed/studio/manifest.json" ]; then
            echo "    seeded by its own seed, then by studio-seed from data/seed/studio/manifest.json and" \
                "data/seed/customer/manifest.json"
        else
            echo "    seeded by its own seed; no data/seed/studio/manifest.json, so studio-seed does not run"
        fi
    else
        echo "  WORLD_PROTO_DIR is not used: no world-proto-server is started (--no-wps)"
    fi
    if [ "$DB_MODE" = container ]; then
        echo "  postgres: the container $PG_CONTAINER ($PG_IMAGE) on 127.0.0.1:$POSTGRES_PORT"
    else
        echo "  postgres: your server, from STUDIO_DATABASE_URL"
    fi
    echo "  the API's STUDIO_DATABASE_URL=$(mask_url "$API_DATABASE_URL")"
    echo "  the API's CRUCIBLE_STORAGE_DIR=$CRUCIBLE_STORAGE_DIR (the run-object store)"
    echo "  the runs: containers of $RUNNER_IMAGE run by $ENGINE, their folders under $STUDIO_RUNNER_OUT_DIR"
    echo "  the streaming server: runners at $STUDIO_STREAMING_RUNNER_URL, viewers at ws://127.0.0.1:$STREAMING_PORT/streams/<run>/<attempt> (events) and /frames/<run>/<attempt> (the picture) and /metrics/<run>/<attempt> (one metric), with a ticket from POST /api/runs/<run>/watch"
    echo "  the streaming server's disk copies of the frames: $STUDIO_STREAMING_CACHE_DIR"
    echo "  the API's STUDIO_* variables (an unset one takes its default in studio/api/README.md):"
    env | grep -E '^(STUDIO|DESVIS_RUN)_' | LC_ALL=C sort | while IFS= read -r line; do
        name=${line%%=*}
        case "$name" in
            *KEY_FILE) printf '    %s=%s\n' "$name" "${line#*=}" ;;
            *KEY*|*SECRET*|*TOKEN*|*PASSWORD*) printf '    %s=(set, not shown)\n' "$name" ;;
            *) printf '    %s=%s\n' "$name" "$(mask_url "${line#*=}")" ;;
        esac
    done
    describe_agent
    [ -z "$OVERRIDDEN" ] || echo "  dev.sh set its own value for:$OVERRIDDEN; the environment's is not used"
    who=$(dev_subject)
    if dev_is_developer; then
        echo "  every request is $who, a developer, so the developer-only controls show; STUDIO_DEVELOPERS= hides them"
    else
        echo "  every request is $who, not a developer, so the developer-only controls stay hidden"
    fi
}

# ---------------------------------------------------------------------------
# The units, each run by the supervisor
# ---------------------------------------------------------------------------

# Polls <probe> once a second: 0 once it passes, 1 when <seconds> pass or <pid>, if given, ends first.
await_ready() {  # <seconds> <pid or ""> <probe...>
    local seconds="$1" pid="$2" i=0
    shift 2
    while [ "$i" -lt "$seconds" ]; do
        "$@" >/dev/null 2>&1 && return 0
        if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then return 1; fi
        sleep 1
        i=$((i + 1))
    done
    return 1
}

await_status() {  # <unit>...
    local unit waiting
    while :; do
        waiting=""
        for unit in "$@"; do
            case "$(read_status "$unit")" in
                ready*) ;;
                failed*) return 1 ;;
                *) waiting=yes ;;
            esac
        done
        [ -z "$waiting" ] && return 0
        sleep 0.5
    done
}

not_answering() {  # <pid> <seconds> <what it never answered>
    if kill -0 "$1" 2>/dev/null; then echo "never answered $3 within $2 s"; else echo "exited before answering $3"; fi
}

pg_ready() {
    # Over TCP: the image's first start serves its init scripts on the socket alone, then restarts.
    "$ENGINE" exec "$PG_CONTAINER" pg_isready -h 127.0.0.1 -U "$PG_USER" -d "$PG_DB"
}

wps_ready() { curl -sf "http://127.0.0.1:$WPS_PORT/healthz"; }
api_ready() { curl -sf "http://127.0.0.1:$API_PORT/healthz"; }
runners_ready() { [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$RUNNERS_PORT/")" != 000 ]; }
streaming_ready() { curl -sf "http://127.0.0.1:$STREAMING_PORT/healthz"; }
ui_ready() { curl -sf "http://127.0.0.1:$UI_PORT/"; }
# `localhost`, as world-proto's own probe: its Vite binds the loopback name, which macOS may answer on ::1 alone.
wpsui_ready() { curl -sf "http://localhost:$WPSUI_PORT/"; }

create_wps_database() {
    [ "$WITH_WPS" = true ] && [ -z "${WORLD_PROTO_DATABASE_URL:-}" ] || return 0
    local exists
    if [ "$DB_MODE" = container ]; then
        exists=$("$ENGINE" exec "$PG_CONTAINER" psql -U "$PG_USER" -d "$PG_DB" -tAc \
            "SELECT 1 FROM pg_database WHERE datname = '$WPS_DB'" 2>/dev/null)
        [ "$exists" = 1 ] || "$ENGINE" exec "$PG_CONTAINER" createdb -U "$PG_USER" "$WPS_DB"
    elif command -v psql >/dev/null 2>&1; then
        exists=$(psql "$API_DATABASE_URL" -tAc "SELECT 1 FROM pg_database WHERE datname = '$WPS_DB'" 2>/dev/null)
        [ "$exists" = 1 ] || psql "$API_DATABASE_URL" -c "CREATE DATABASE $WPS_DB" >/dev/null
    else
        echo "[postgres] no psql on the PATH to make $WPS_DB on your server; make it, or set WORLD_PROTO_DATABASE_URL"
    fi
}

# Postgres in a container named after this folder, which every start reuses, rows and all; the rows are scratch,
# so --stop --db removes the container with the anonymous volume its image keeps them in.
start_postgres() {
    if [ "$DB_MODE" = external ]; then
        create_wps_database || { unit_failed postgres "could not make $WPS_DB on your server"; return 1; }
        update_status postgres "ready: your server, from STUDIO_DATABASE_URL"
        return 0
    fi
    update_status postgres "building: the container"
    local state
    state=$("$ENGINE" ps -a --filter "name=^${PG_CONTAINER}\$" --format '{{.State}}' 2>/dev/null | head -1)
    case "${state:-absent}" in
        running) echo "[postgres] reusing $PG_CONTAINER on $POSTGRES_PORT" ;;
        absent)
            echo "[postgres] starting $PG_IMAGE as $PG_CONTAINER on $POSTGRES_PORT"
            if ! "$ENGINE" run -d --name "$PG_CONTAINER" -e POSTGRES_USER="$PG_USER" \
                -e POSTGRES_PASSWORD="$PG_PASSWORD" -e POSTGRES_DB="$PG_DB" \
                -p "127.0.0.1:$POSTGRES_PORT:5432" "$PG_IMAGE" >/dev/null; then
                "$ENGINE" rm -f -v "$PG_CONTAINER" >/dev/null 2>&1
                unit_failed postgres "the container would not start on $POSTGRES_PORT"
                return 1
            fi
            ;;
        *)
            echo "[postgres] restarting $PG_CONTAINER (was $state) on $POSTGRES_PORT"
            "$ENGINE" start "$PG_CONTAINER" >/dev/null \
                || { unit_failed postgres "the container would not start on $POSTGRES_PORT"; return 1; }
            ;;
    esac
    update_status postgres "building: waiting for connections"
    await_ready 60 "" pg_ready || { unit_failed postgres "never accepted connections"; return 1; }
    create_wps_database || { unit_failed postgres "could not make $WPS_DB"; return 1; }
    update_status postgres "ready"
}

# world-proto is built with its own target directory rather than any CARGO_TARGET_DIR this shell carries: the seed
# runs its CLI from target/debug.
build_wps() {
    update_status wps "building: cargo, world-proto at $WPS_COMMIT"
    echo "[wps] building world-graph-server and world-graph-cli at $WPS_COMMIT"
    if ! (cd "$WORLD_PROTO_DIR/server" && unset CARGO_TARGET_DIR \
        && cargo build -p world-graph-server && cargo build -p world-graph-cli) >>"$LOG_DIR/wps.log" 2>&1; then
        unit_failed wps "build error at $WPS_COMMIT, see studio/.dev-log/wps.log"
        return 1
    fi
}

run_wps() {
    local pid
    update_status wps "building: connecting to postgres and binding"
    (cd "$WORLD_PROTO_DIR/server" && exec env DATABASE_URL="$WPS_DATABASE_URL" BIND="$LISTEN_HOST:$WPS_PORT" \
        "$WORLD_PROTO_DIR/target/debug/world-graph-server") >>"$LOG_DIR/wps.log" 2>&1 &
    pid=$!
    echo "$pid" > "$STATE_DIR/wps.pid"
    if ! await_ready 300 "$pid" wps_ready; then
        unit_failed wps "$(not_answering "$pid" 300 /healthz), see studio/.dev-log/wps.log"
        return 1
    fi
    update_status wps "ready"
    # Reached only when the server ends, which after `ready` means it fell over.
    wait "$pid"
    unit_failed wps "world-proto-server exited"
}

# World-proto's browser UI as its server/dev.sh's start_ui serves it, from the same checkout, unchanged and with the
# same commands; only its port and the server it reads (VITE_SERVER_URL) are this stack's.
run_wpsui() {
    local pid
    update_status wpsui "starting"
    if ! command -v bun >/dev/null 2>&1; then
        unit_failed wpsui "bun is not installed, and world-proto's UI runs on it"
        return 1
    fi
    update_status wpsui "building: bun install"
    (cd "$WPSUI_DIR" && bun install) >>"$LOG_DIR/wpsui.log" 2>&1 \
        || { unit_failed wpsui "bun install failed, see studio/.dev-log/wpsui.log"; return 1; }
    update_status wpsui "building: typecheck"
    (cd "$WPSUI_DIR" && bun run typecheck) >>"$LOG_DIR/wpsui.log" 2>&1 \
        || { unit_failed wpsui "typecheck error at $WPS_COMMIT, see studio/.dev-log/wpsui.log"; return 1; }
    update_status wpsui "building: bun run dev"
    (cd "$WPSUI_DIR" && exec env VITE_SERVER_URL="http://127.0.0.1:$WPS_PORT" bun run dev -- --port "$WPSUI_PORT") \
        >>"$LOG_DIR/wpsui.log" 2>&1 &
    pid=$!
    echo "$pid" > "$STATE_DIR/wpsui.pid"
    if ! await_ready 60 "$pid" wpsui_ready; then
        unit_failed wpsui "$(not_answering "$pid" 60 "http://localhost:$WPSUI_PORT/"), see studio/.dev-log/wpsui.log"
        return 1
    fi
    update_status wpsui "ready"
    wait "$pid"
    unit_failed wpsui "world-proto's UI exited"
}

# Foundry's application first (scripts/seed-dev.sh), which registers Agent Studio's vocabulary among its components;
# then, when the manifest exists, studio-seed writes Agent Studio's templates and catalog items through the client,
# skipping every entry whose scope already holds a changeset (spec 04 section 3.13), and after them the customer sites
# data/seed/customer/manifest.json lists, each document imported as it stands.
run_seed() {
    local manifest="$REPO_ROOT/data/seed/studio/manifest.json" each args
    update_status seed "building: seed-dev.sh"
    echo "[seed] seeding http://127.0.0.1:$WPS_PORT with Foundry's application"
    if ! "$REPO_ROOT/scripts/seed-dev.sh" --server "http://127.0.0.1:$WPS_PORT" --world-proto "$WORLD_PROTO_DIR" \
        >>"$LOG_DIR/seed.log" 2>&1; then
        unit_failed seed "seed-dev.sh failed, see studio/.dev-log/seed.log"
        return 1
    fi
    if [ ! -f "$manifest" ]; then
        update_status seed "ready: seeded (no data/seed/studio/manifest.json for studio-seed)"
        return 0
    fi
    update_status seed "building: cargo, studio-seed"
    echo "[seed] building studio-seed"
    if ! (cd "$REPO_ROOT" && cargo build -p wps-client --bin studio-seed) >>"$LOG_DIR/seed.log" 2>&1; then
        unit_failed seed "studio-seed would not build, see studio/.dev-log/seed.log"
        return 1
    fi
    args=(--commit "$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
        --vocabulary "$REPO_ROOT/data/seed/application/components")
    update_status seed "building: studio-seed"
    for each in "$manifest" "$REPO_ROOT/data/seed/customer/manifest.json"; do
        [ -f "$each" ] || continue
        echo "[seed] seeding from ${each#"$REPO_ROOT"/}"
        if ! (cd "$REPO_ROOT" && "$SEED_BINARY" --manifest "$each" "${args[@]}") >>"$LOG_DIR/seed.log" 2>&1; then
            unit_failed seed "studio-seed stopped on ${each#"$REPO_ROOT"/}, see studio/.dev-log/seed.log"
            return 1
        fi
    done
    update_status seed "ready: seeded, data/seed/studio/manifest.json's entries and the customer sites included"
}

build_api() {
    update_status api "building: cargo"
    update_status streaming "building: cargo, with the API"
    echo "[api] building studio-api and studio-streaming"
    if ! (cd "$REPO_ROOT" && cargo build --profile studio-dev -p studio-api -p studio-streaming) \
        >>"$LOG_DIR/api.log" 2>&1; then
        unit_failed api "build error, see studio/.dev-log/api.log"
        unit_failed streaming "build error, see studio/.dev-log/api.log"
        return 1
    fi
}

# The streaming server, after the API, whose migrations make its tables: it registers itself in the API's database
# and the runs' containers reach its runners' port at the host's alias.
run_streaming() {
    local pid
    update_status streaming "building: registering and binding"
    (cd "$REPO_ROOT" && exec env STUDIO_DATABASE_URL="$API_DATABASE_URL" "$STREAMING_BINARY") \
        >>"$LOG_DIR/streaming.log" 2>&1 &
    pid=$!
    echo "$pid" > "$STATE_DIR/streaming.pid"
    if ! await_ready 60 "$pid" streaming_ready; then
        unit_failed streaming "$(not_answering "$pid" 60 /healthz), see studio/.dev-log/streaming.log"
        return 1
    fi
    update_status streaming "ready"
    wait "$pid"
    unit_failed streaming "the streaming server exited"
}

# The runner image every attempt runs as a container: studio/Dockerfile's simrunner target, built from this checkout
# by the engine unless it already holds this checkout's image of the source the checkout holds now, as
# scripts/source-identity.sh names it (the commit, and a hash of any uncommitted changes), which the image keeps in its
# agent-studio/source label and the API holds it to. A release build of the workspace inside the engine: long the first
# time; after that the layer cache, and cargo's caches of its registry, its git checkouts and its build, which the
# Dockerfile keeps, so a rebuild compiles what changed and fetches the private world-proto repository again only when
# its pinned revision moves. That fetch goes through the SSH agent, forwarded as scripts/build-and-push.sh forwards it,
# so the agent must then hold a key that reads that repository.
build_runner_image() {
    local branch commit dirty=false source baked ssh=()
    branch=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)
    commit=$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)
    [ -z "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=no 2>/dev/null)" ] || dirty=true
    if ! source=$(cd "$REPO_ROOT" && sh scripts/source-identity.sh); then
        unit_failed runners "scripts/source-identity.sh could not name this checkout's source"
        return 1
    fi
    baked=$("$ENGINE" image inspect --format '{{index .Config.Labels "agent-studio/source"}}' "$RUNNER_IMAGE" 2>/dev/null)
    if [ "$baked" = "$source" ]; then
        echo "[runners] the runner image $RUNNER_IMAGE is of this checkout's source, $source"
        return 0
    fi
    update_status runners "building: the runner image $RUNNER_IMAGE (a release build inside $ENGINE; long at first)"
    echo "[runners] building the runner image $RUNNER_IMAGE of $source with $ENGINE, see studio/.dev-log/runners.log"
    if [ -n "${SSH_AUTH_SOCK:-}" ]; then ssh=(--ssh default); fi
    if ! (cd "$REPO_ROOT" && "$ENGINE" build ${ssh[@]+"${ssh[@]}"} -f studio/Dockerfile --target simrunner \
            -t "$RUNNER_IMAGE" --build-arg CARGO_FEATURES="" --build-arg GIT_BRANCH="$branch" \
            --build-arg GIT_COMMIT="$commit" --build-arg GIT_DIRTY="$dirty" --build-arg GIT_SOURCE="$source" .) \
            >>"$LOG_DIR/runners.log" 2>&1; then
        unit_failed runners "the runner image would not build (it reads world-proto over SSH): see studio/.dev-log/runners.log"
        return 1
    fi
}

run_api() {
    local pid
    update_status api "building: migrations and bind"
    update_status runners "building: waiting for the API"
    (cd "$REPO_ROOT" && exec env STUDIO_DATABASE_URL="$API_DATABASE_URL" "$API_BINARY") >>"$LOG_DIR/api.log" 2>&1 &
    pid=$!
    echo "$pid" > "$STATE_DIR/api.pid"
    if ! await_ready 120 "$pid" api_ready; then
        unit_failed api "$(not_answering "$pid" 120 /healthz), see studio/.dev-log/api.log"
        return 1
    fi
    update_status api "ready"
    if ! await_ready 30 "$pid" runners_ready; then
        unit_failed runners "the runners' listener never answered on $RUNNERS_PORT"
        return 1
    fi
    update_status runners "ready"
    wait "$pid"
    update_status runners "failed: the API exited"
    unit_failed api "the API exited"
}

prepare_ui() {
    local manager
    update_status ui "starting"
    [ -x "$REPO_ROOT/studio/ui/node_modules/.bin/vite" ] && return 0
    manager=$(ui_package_manager)
    update_status ui "building: $manager install"
    (cd "$REPO_ROOT/studio/ui" && "$manager" install) >>"$LOG_DIR/ui.log" 2>&1 \
        || { unit_failed ui "the dependencies would not install"; return 1; }
}

# The floor viewer's wasm, which studio's `crucible-viewer` alias imports from the package's source: a build of
# studio/viewer, not checked in, so a checkout that never built it draws no floor. Every start builds it, and
# cargo decides what is stale from everything the build reads (Cargo.lock and the manifests included), which a list
# of source folders kept here would not (reviews-impl/astra-t12.md finding 8): a minute from cold, seconds when
# nothing changed. build-wasm.sh moves a fresh directory into place rather than writing over the module Vite serves.
build_viewer() {
    update_status ui "building: the floor viewer's wasm"
    "$REPO_ROOT/studio/viewer/js/build-wasm.sh" release >>"$LOG_DIR/ui.log" 2>&1 \
        || { unit_failed ui "the floor viewer's wasm would not build, see studio/.dev-log/ui.log"; return 1; }
}

# Vite run as its own binary rather than through `bun run dev`, so the recorded PID is the dev server itself.
run_ui() {
    local pid
    update_status ui "building: vite"
    (cd "$REPO_ROOT/studio/ui" && exec env STUDIO_API_PORT="$API_PORT" \
        "$REPO_ROOT/studio/ui/node_modules/.bin/vite" --host 127.0.0.1 --port "$UI_PORT" --strictPort) >>"$LOG_DIR/ui.log" 2>&1 &
    pid=$!
    echo "$pid" > "$STATE_DIR/ui.pid"
    if ! await_ready 60 "$pid" ui_ready; then
        unit_failed ui "$(not_answering "$pid" 60 "http://127.0.0.1:$UI_PORT/"), see studio/.dev-log/ui.log"
        return 1
    fi
    update_status ui "ready"
    wait "$pid"
    unit_failed ui "vite exited"
}

# ---------------------------------------------------------------------------
# start, --status, --logs, --stop
# ---------------------------------------------------------------------------

# The units' own word for how far they got, printed as it changes, so `start` holds the terminal until there is
# something to open, and a unit that gives up says so here rather than in a log nobody is reading yet.
await_units() {  # <supervisor pid>
    local seen="" waited=0 unit line all
    while [ "$waited" -lt 3600 ]; do
        all=ready
        for unit in $UNITS; do
            line="[$unit] $(read_status "$unit")"
            case "$seen$line" in *"|$line|"*|*"] -") ;; *) echo "  $line"; seen="$seen|$line|" ;; esac
            case "$line" in
                *"] failed"*) return 1 ;;
                *"] ready"*|*"] stopped"*) ;;
                *) all=waiting ;;
            esac
        done
        [ "$all" = ready ] && return 0
        kill -0 "$1" 2>/dev/null || return 1
        sleep 0.5
        waited=$((waited + 1))
    done
    echo "dev.sh: the stack did not come up within thirty minutes" >&2
    return 1
}

report_failure() {
    local failure unit log
    failure=$(cat "$FAILURE_FILE" 2>/dev/null)
    unit=${failure%%:*}
    log="$LOG_DIR/${unit:-supervisor}.log"
    [ -f "$log" ] || log="$LOG_DIR/supervisor.log"
    echo "dev.sh: ${failure:-no unit said why}" >&2
    echo "--- the end of $log:" >&2
    tail -20 "$log" >&2
    case "$unit" in
        wps|seed) echo "--no-seed starts world-proto-server without either seed; --no-wps starts the stack without it" >&2 ;;
    esac
}

cmd_start() {
    local pid unit
    while [ $# -gt 0 ]; do
        case "$1" in
            --no-wps) WITH_WPS=false ;;
            --no-seed) WITH_SEED=false ;;
            *) echo "dev.sh: start takes --no-wps and --no-seed, not $1" >&2; return 1 ;;
        esac
        shift
    done
    if [ -n "${STUDIO_DATABASE_URL:-}" ]; then DB_MODE=external; else DB_MODE=container; fi
    # The runs' containers need the engine whatever the database is.
    ENGINE=$(container_engine)
    if [ -z "$ENGINE" ]; then
        echo "dev.sh: no container engine answers (docker or podman): the runs are containers of the runner image" >&2
        return 1
    fi
    preflight || return 1
    mkdir -p "$LOG_DIR" "$STATE_DIR" "$REGISTRY_DIR"

    if supervisor_running; then
        echo "Agent Studio's stack is already running for this folder"
        cmd_status
        return 0
    fi
    if orphans_of "$(meta_value PID)"; then
        echo "dev.sh: processes of an earlier start are still running; ./dev.sh --stop takes them down first" >&2
        return 1
    fi
    if [ "$WITH_WPS" = true ]; then resolve_world_proto || return 1; fi
    choose_ports || return 1
    if [ "$DB_MODE" = container ]; then choose_postgres_port || return 1; fi
    API_BINARY="$(cd "$REPO_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    API_BINARY="${API_BINARY:-${CARGO_TARGET_DIR:-$REPO_ROOT/target}}/studio-dev/studio-api"
    STREAMING_BINARY="$(dirname "$API_BINARY")/studio-streaming"
    SEED_BINARY="$(dirname "$(dirname "$API_BINARY")")/debug/studio-seed"
    configure
    print_configuration
    printf 'API_PORT=%s\nRUNNERS_PORT=%s\nWPS_PORT=%s\nUI_PORT=%s\nWPSUI_PORT=%s\nSTREAMING_INTERNAL_PORT=%s\nSTREAMING_PORT=%s\n' \
        "$API_PORT" "$RUNNERS_PORT" "$WPS_PORT" "$UI_PORT" "$WPSUI_PORT" "$STREAMING_INTERNAL_PORT" "$STREAMING_PORT" \
        > "$PORTS_FILE"
    printf 'WITH_WPS=%s\nWORLD_PROTO_DIR=%s\nWPS_COMMIT=%s\n' \
        "$WITH_WPS" "${WORLD_PROTO_DIR:-}" "$WPS_COMMIT" > "$RUN_FILE"

    # Each start's logs begin empty, so a failure's tail is this start's.
    rm -f "$FAILURE_FILE" "$STATE_DIR/stopping" "$STATE_DIR"/*.pid
    for unit in wps wpsui seed api runners streaming ui supervisor; do : > "$LOG_DIR/$unit.log"; done
    clear_status
    for unit in $UNITS; do update_status "$unit" "starting"; done
    if [ "$WITH_WPS" = false ]; then
        update_status wps "stopped: started with --no-wps"
        update_status seed "stopped: started with --no-wps"
    elif [ "$WITH_SEED" = false ]; then
        update_status seed "stopped: started with --no-seed"
    fi
    if [ "$WITH_WPS" = false ]; then
        update_status wpsui "stopped: no world-proto UI, started with --no-wps"
    elif [ -z "$WPSUI_DIR" ]; then
        update_status wpsui "stopped: no world-proto UI, $WORLD_PROTO_DIR has no server/ui"
    fi

    # `set -m` gives the subshell a process group of its own, so its PID is the group's id and one signal to the
    # group, from --stop, Sutra or a hand, reaches every unit.
    set -m
    (
        set +e
        cd "$SCRIPT_DIR" || exit 1
        # bash 3.2, as macOS ships it, has no $BASHPID: the kernel is asked which group this subshell leads.
        MY_PGID=$(sh -c 'ps -o pgid= -p $$' | tr -d ' ')
        MY_PID=$(sh -c 'echo $PPID')
        if [ -z "$MY_PGID" ] || [ "$MY_PGID" != "$MY_PID" ]; then
            unit_failed supervisor "pid $MY_PID does not lead its own process group; not starting"
            exit 1
        fi
        cleanup() {
            trap - INT TERM HUP EXIT
            touch "$STATE_DIR/stopping"
            rm -f "$META_FILE"
            clear_status
            kill -- -"$MY_PGID" 2>/dev/null
            true
        }
        trap cleanup INT TERM HUP EXIT

        start_postgres || exit 1
        if [ "$WITH_WPS" = true ]; then
            build_wps || exit 1
            run_wps &
            if [ -n "$WPSUI_DIR" ]; then run_wpsui & fi
            await_status wps || exit 1
            if [ "$WITH_SEED" = true ]; then run_seed || exit 1; fi
        fi
        # The runner image before the API: its source is named before either is built, and the API names the source
        # again as it starts, so a file saved during either build leaves the two names apart and the API refuses the
        # image, rather than running a simulator other than its own build's.
        build_runner_image || exit 1
        build_api || exit 1
        run_api &
        await_status api runners || exit 1
        run_streaming &
        await_status streaming || exit 1
        prepare_ui || exit 1
        build_viewer || exit 1
        run_ui &
        await_status ui || exit 1
        wait
    ) >>"$LOG_DIR/supervisor.log" 2>&1 &
    pid=$!
    set +m
    # Off this shell's job table: the supervisor reaps its own group, and a tracked job dying that way makes bash
    # print `Terminated: 15` over whatever the person is reading.
    disown "$pid" 2>/dev/null || true
    write_meta "$pid"

    echo "Starting (supervisor pid $pid); Ctrl+C stops the waiting here, not the stack:"
    if ! await_units "$pid"; then
        report_failure
        stop_group "$pid"
        rm -f "$META_FILE" "$STATE_DIR"/*.pid
        clear_status
        return 1
    fi
    echo
    echo "  studio      http://127.0.0.1:$UI_PORT/        Vite over studio/ui/, handing /api to the API"
    echo "  api         http://127.0.0.1:$API_PORT/healthz"
    echo "  runners     http://127.0.0.1:$RUNNERS_PORT/   the runners' listener: each run's simrunner reads its pins here"
    echo "  streaming   ws://127.0.0.1:$STREAMING_PORT/streams/<run>/<attempt>   viewers, with a ticket from POST /api/runs/<run>/watch"
    if [ "$WITH_WPS" = true ]; then
        echo "  wps         http://127.0.0.1:$WPS_PORT/protocol   world-proto at $WPS_COMMIT"
    fi
    if [ -n "$WPSUI_DIR" ]; then
        echo "  wpsui       http://localhost:$WPSUI_PORT/   world-proto's own UI at $WPS_COMMIT, reading that server"
    fi
    if [ "$DB_MODE" = container ]; then echo "  postgres    127.0.0.1:$POSTGRES_PORT ($PG_CONTAINER)"; fi
    echo "  supervisor  pid $pid; ./dev.sh --stop, or Sutra's terminate button"
    echo "  logs        ./dev.sh --logs [api|wps|wpsui|seed|streaming|ui|postgres|supervisor]"
}

unit_where() {  # <unit>: where a unit answers, as its start recorded it
    local port pid=""
    port=$(meta_value "$(echo "$1" | tr '[:lower:]' '[:upper:]')_PORT")
    if [ -f "$STATE_DIR/$1.pid" ] && kill -0 "$(cat "$STATE_DIR/$1.pid")" 2>/dev/null; then
        pid=", pid $(cat "$STATE_DIR/$1.pid")"
    fi
    case "$1" in
        postgres) [ -z "$port" ] || printf ' · %s on 127.0.0.1:%s' "$PG_CONTAINER" "$port" ;;
        wps) printf ' · http://127.0.0.1:%s%s, world-proto at %s' "$port" "$pid" "$(file_value "$RUN_FILE" WPS_COMMIT)" ;;
        wpsui) [ -z "$port" ] || printf ' · http://localhost:%s/%s, world-proto at %s' "$port" "$pid" \
            "$(file_value "$RUN_FILE" WPS_COMMIT)" ;;
        api|runners|ui) printf ' · http://127.0.0.1:%s/%s' "$port" "$pid" ;;
        streaming) printf ' · viewers ws://127.0.0.1:%s/streams/<run>/<attempt> and /frames/<run>/<attempt>, runners on %s%s, cache %s' \
            "$port" "$(meta_value STREAMING_INTERNAL_PORT)" "$pid" "$(cache_size)" ;;
    esac
}

# How much the streaming server's disk copies of the frames take, as `du` says it; "none" before the folder exists.
cache_size() {
    if [ -d "$STATE_DIR/streaming-cache" ]; then du -sh "$STATE_DIR/streaming-cache" 2>/dev/null | cut -f1; else echo none; fi
}

cmd_status() {
    local pid unit state failed=0
    pid=$(meta_value PID)
    if ! supervisor_running; then
        if orphans_of "$pid"; then
            echo "Agent Studio's supervisor (pid $pid) is gone, but processes it started still run; --stop ends them"
        else
            echo "Agent Studio's stack is not running for $SCRIPT_DIR"
        fi
        return 1
    fi
    echo "Agent Studio dev runner ($REGISTRY_KEY): supervisor pid $pid, up $(( $(date +%s) - $(meta_value STARTED) ))s"
    for unit in $UNITS; do
        state=$(read_status "$unit")
        case "$state" in failed*) failed=1 ;; esac
        if [ "$(file_value "$RUN_FILE" WITH_WPS)" = false ] && { [ "$unit" = wps ] || [ "$unit" = seed ] \
            || [ "$unit" = wpsui ]; }; then
            printf '  %-9s %s\n' "$unit" "$state"
        else
            printf '  %-9s %s%s\n' "$unit" "$state" "$(unit_where "$unit")"
        fi
    done
    printf '  %-9s %s\n' logs "$LOG_DIR" sutra "$META_FILE"
    return "$failed"
}

cmd_logs() {
    local unit="${1:-api}"
    case "$unit" in
        runners) unit=api ;;
        postgres)
            ENGINE=$(container_engine)
            [ -n "$ENGINE" ] || { echo "dev.sh: no Docker or Podman is running to read $PG_CONTAINER's log" >&2; return 1; }
            echo "$PG_CONTAINER (Ctrl+C to detach)"
            exec "$ENGINE" logs -f "$PG_CONTAINER"
            ;;
    esac
    if [ ! -f "$LOG_DIR/$unit.log" ]; then
        echo "dev.sh: no log at $LOG_DIR/$unit.log; the logs are api, wps, wpsui, seed, streaming, ui, postgres and supervisor" >&2
        return 1
    fi
    echo "$LOG_DIR/$unit.log (Ctrl+C to detach)"
    exec tail -f "$LOG_DIR/$unit.log"
}

# One SIGTERM to the supervisor's group, which its trap answers by taking the rest of the group down with it.
# Members still standing after ten seconds keep the group's id from being reused, so it is still this folder's.
stop_group() {  # <process group id>
    local i=0
    kill -TERM -- -"$1" 2>/dev/null
    while [ "$i" -lt 20 ] && pgrep -g "$1" >/dev/null 2>&1; do sleep 0.5; i=$((i + 1)); done
    if pgrep -g "$1" >/dev/null 2>&1; then kill -KILL -- -"$1" 2>/dev/null; fi
}

# This stack's runner containers, by the label its API gives them; none of another checkout's stack.
stack_runners() {
    "$ENGINE" ps -a --filter label=agent-studio=runner --filter "label=agent-studio/stack=$REGISTRY_KEY" \
        --format '{{.Names}}' 2>/dev/null
}

cmd_stop() {
    local pid with_db=false with_runners=false runners
    while [ $# -gt 0 ]; do
        case "$1" in
            --db) with_db=true ;;
            --runners) with_runners=true ;;
            *) echo "dev.sh: --stop takes --db and --runners, not $1" >&2; return 1 ;;
        esac
        shift
    done
    pid=$(meta_value PID)
    if supervisor_running || orphans_of "$pid"; then
        echo "stopping Agent Studio's stack (process group $pid)"
        stop_group "$pid"
    else
        echo "no stack is running for $SCRIPT_DIR"
    fi
    rm -f "$META_FILE" "$STATE_DIR"/*.pid
    clear_status

    ENGINE=$(container_engine)
    [ -n "$ENGINE" ] || return 0
    # This stack's runs' containers are the record of what runs: they go on without the API, which adopts them at its
    # next start, unless asked to remove them. Removing the rows and the store does that too, and only once they are
    # gone, since no API could adopt them after.
    if ! runners=$(stack_runners); then
        echo "dev.sh: the engine would not list this stack's runs' containers" >&2
        if [ "$with_db" = true ]; then echo "dev.sh: so their rows and storage are kept" >&2; return 1; fi
        runners=""
    fi
    if [ -n "$runners" ] && { [ "$with_runners" = true ] || [ "$with_db" = true ]; }; then
        echo "$runners" | xargs "$ENGINE" rm -f >/dev/null 2>&1
        if ! runners=$(stack_runners) || [ -n "$runners" ]; then
            echo "dev.sh: this stack's runs' containers are not known gone ($(echo "$runners" | tr '\n' ' '))" >&2
            if [ "$with_db" = true ]; then echo "dev.sh: so their rows and storage are kept" >&2; fi
            return 1
        fi
        echo "removed this stack's runs' containers"
    elif [ -n "$runners" ]; then
        echo "this stack's runs' containers are still there ($(echo "$runners" | wc -l | tr -d ' ')); ./dev.sh --stop --runners removes them"
    fi
    if [ "$with_db" = true ] && [ -d "$STATE_DIR/objects" ]; then
        rm -rf "$STATE_DIR/objects" && echo "removed the run-object store's folder, whose objects the rows named"
    fi
    if [ "$with_db" = true ] && [ -d "$STATE_DIR/runner-out" ]; then
        rm -rf "$STATE_DIR/runner-out" && echo "removed the runs' output folders"
    fi
    if [ "$with_db" = true ]; then
        if "$ENGINE" rm -f -v "$PG_CONTAINER" >/dev/null 2>&1; then
            echo "removed $PG_CONTAINER and its data"
        else
            echo "no $PG_CONTAINER to remove"
        fi
    elif [ -n "$("$ENGINE" ps -a --filter "name=^${PG_CONTAINER}\$" --format '{{.Names}}' 2>/dev/null)" ]; then
        echo "$PG_CONTAINER is still up, holding its rows; ./dev.sh --stop --db removes it"
    fi
}

case "${1:-start}" in
    start) shift; cmd_start "$@" ;;
    --no-wps|--no-seed) cmd_start "$@" ;;
    stop|--stop) shift; cmd_stop "$@" ;;
    status|--status) shift; cmd_status ;;
    logs|--logs) shift; cmd_logs "$@" ;;
    -h|--help|help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0" ;;
    *) echo "dev.sh: no such command: $1 (./dev.sh --help lists them)" >&2; exit 1 ;;
esac
