#!/bin/bash
# Bring this checkout's Crucible up: a PostgreSQL container, a release build of
# the server, and the server running against both.
#
# Everything is named after the checkout, so two clones of this repository run
# side by side without meeting: the container, the host port, the storage
# folder and the registry entry all derive from a hash of this directory. That
# also means the pair finds the SAME container again on the next start, which a
# freshly chosen free port would not.
#
# The server is built in the RELEASE profile because it runs the simulator
# in-process, and a debug build runs it at about a third of real time. It is
# then run as the binary that was just built, never through a second `cargo
# run`, which would queue on the target directory's lock behind the build.
#
# `start` ends with a working UI: the wasm viewer is built, Vite's dev server is
# started over `studio/ui`, and the page reaches the server through that dev
# server's proxy — so the address printed at the end is the only one anybody
# needs.
#
# The server and the UI are children of one supervisor process, which this
# script leaves running and which owns them: it is a process-group leader, so
# one signal to it takes the pair down, and it publishes this checkout under
# `~/.dev-runner/` for Sutra, the dashboard of dev runners. Sutra's terminate
# button sends that signal, which is exactly what `./dev.sh stop` sends.
#
#   ./dev.sh start        build and start everything; print the addresses
#   ./dev.sh start --no-ui   the server alone, for the terminal client
#   ./dev.sh start --bucket <name>   store recordings in that GCS bucket instead of the folder,
#                                     built with --features gcs and reached with your own gcloud credentials
#   ./dev.sh stop         stop the UI and the server; the container keeps its rows
#   ./dev.sh stop --db    also remove the container and its data
#   ./dev.sh status       one line per unit
#   ./dev.sh logs         follow the server's log (`logs ui` for the UI's)
#   ./dev.sh submit ...   submit a spec through the terminal client
#   ./dev.sh status-of <run> · ./dev.sh list · ./dev.sh cancel <run>
#   ./dev.sh attach <run> [<ms>] ... · ./dev.sh series <run> ...   the client's readers
#   ./dev.sh gcs-emulator [stop]   a Google Cloud Storage emulator, for `cargo test -p runstore --features gcs`
set -u

# Set by `start --bucket`: the server then stores into this bucket, built with
# the `gcs` feature, and `STORAGE_DIR` below is left alone. Empty means the
# folder, which is what development runs against.
STORAGE_BUCKET=""

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
LOG_DIR="$SCRIPT_DIR/.dev-log"
STATE_DIR="$SCRIPT_DIR/.dev-state"
STORAGE_DIR="$SCRIPT_DIR/.dev-storage"
TEMP_DIR="$SCRIPT_DIR/.dev-temp"

# The stable identifier for this checkout: sha256 of its absolute path, first
# 16 hex characters. Guarded, because an empty key would let the cleanups below
# reach another checkout's files.
REGISTRY_KEY=$(printf '%s' "$SCRIPT_DIR" | shasum -a 256 2>/dev/null | cut -c1-16)
if [ ${#REGISTRY_KEY} -lt 8 ]; then
    echo "dev.sh: could not derive a key for $SCRIPT_DIR — is shasum on the PATH?" >&2
    exit 1
fi

PG_CONTAINER="crucible-pg-$REGISTRY_KEY"
PG_IMAGE="postgres:17"
PG_USER="postgres"
PG_PASSWORD="postgres"
PG_DB="crucible"
# Above the ephemeral range and below 50000, derived so the port and the
# container find each other again next time.
POSTGRES_PORT=$(( 30000 + 0x${REGISTRY_KEY:0:4} % 20000 ))
SERVER_PORT=$(( 20000 + 0x${REGISTRY_KEY:4:4} % 9000 ))
UI_PORT=$(( 10000 + 0x${REGISTRY_KEY:8:4} % 9000 ))

# The storage emulator the runstore bucket tests run against. Not a unit of the
# stack: the server here stores into a folder, and the emulator exists only so
# `cargo test -p runstore --features gcs` has a bucket to reach from a
# workstation. Its own range of ports, so it meets none of the three above.
#
# The fork, not fsouza/fake-gcs-server itself: the original answers the plain
# `PUT /{bucket}/{object}` the GCS client writes with (the XML API) with
# "invalid uploadType", and has since 2023 (fsouza/fake-gcs-server#1164). The
# fork is the image object_store's own test suite runs against. It is built for
# amd64 only, hence the platform flag; Docker Desktop and Podman emulate it.
GCS_CONTAINER="crucible-gcs-$REGISTRY_KEY"
GCS_IMAGE="tustvold/fake-gcs-server"
GCS_BUCKET="runstore-test"
GCS_PORT=$(( 50000 + 0x${REGISTRY_KEY:12:4} % 9000 ))
GCS_URL="http://127.0.0.1:$GCS_PORT"
DATABASE_URL="postgres://$PG_USER:$PG_PASSWORD@127.0.0.1:$POSTGRES_PORT/$PG_DB"
SERVER_WS="ws://127.0.0.1:$SERVER_PORT/ws"
UI_URL="http://127.0.0.1:$UI_PORT/"

PID_FILE="$STATE_DIR/server.pid"
SERVER_LOG="$LOG_DIR/server.log"
UI_DIR="$REPO_ROOT/studio/ui"
UI_PID_FILE="$STATE_DIR/ui.pid"
UI_LOG="$LOG_DIR/ui.log"
SUPERVISOR_LOG="$LOG_DIR/supervisor.log"

# ---------------------------------------------------------------------------
# What this checkout tells Sutra, the dashboard of dev runners
# ---------------------------------------------------------------------------
#
# Sutra shows every dev runner that publishes itself under `~/.dev-runner/`:
# one meta file per checkout, named by this checkout's key, saying where it is,
# which process to signal to stop it and which ports it took; and one status
# file per unit, saying what that unit is doing. Nothing is polled and nothing
# is inferred — what this script writes is what the dashboard shows, so the
# writes below sit at the moments the units actually change.

REGISTRY_DIR="$HOME/.dev-runner"
META_FILE="$REGISTRY_DIR/$REGISTRY_KEY"
# Every unit this runner reports, so a fresh start can set them all going and a
# stop can account for them.
UNITS="postgres viewer server ui"

# One unit's state, as `<state>` or `<state>: <detail>`. Written to a temporary
# file and renamed, never straight over the old one: a plain `>` truncates
# first, and a dashboard reading in that moment sees an empty file.
update_status() {
    local file="$REGISTRY_DIR/$REGISTRY_KEY.$1.status"
    printf '%s\n' "$2" > "$file.tmp" && mv -f "$file.tmp" "$file"
}

read_status() {
    cat "$REGISTRY_DIR/$REGISTRY_KEY.$1.status" 2>/dev/null || echo "-"
}

# Only ever this checkout's files: the key is in every pattern, and it is
# guarded above, because an empty one would make these globs another project's.
clear_status() {
    rm -f "$REGISTRY_DIR/$REGISTRY_KEY".*.status "$REGISTRY_DIR/$REGISTRY_KEY".*.status.tmp
}

write_meta() {  # <supervisor pid> <with-ui: true|false>
    {
        echo "DIR=$SCRIPT_DIR"
        echo "PID=$1"
        echo "STARTED=$(date +%s)"
        echo "POSTGRES_PORT=$POSTGRES_PORT"
        echo "SERVER_PORT=$SERVER_PORT"
        # An `if`, not a `[ … ] &&`: the last command in this group decides
        # whether the rename below happens at all, and a false test there
        # leaves the checkout unregistered every time it starts without a UI.
        if [ "$2" = true ]; then echo "UI_PORT=$UI_PORT"; fi
    } > "$META_FILE.tmp" && mv -f "$META_FILE.tmp" "$META_FILE"
}

meta_value() {
    grep "^$1=" "$META_FILE" 2>/dev/null | head -1 | cut -d= -f2-
}

# Is this checkout's supervisor alive? A dead PID, or one the kernel has since
# handed to something else, takes the whole registry entry with it rather than
# leaving the dashboard showing a stack that is not there.
supervisor_running() {
    [ -f "$META_FILE" ] || return 1
    local pid pgid
    pid=$(meta_value PID)
    if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; then
        rm -f "$META_FILE"
        clear_status
        return 1
    fi
    # The supervisor leads its own process group, so most recycled PIDs fail this.
    pgid=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')
    if [ "$pgid" != "$pid" ]; then
        rm -f "$META_FILE"
        clear_status
        return 1
    fi
    # Any shell leads a group too, and `stop` signals the whole group, so a PID
    # the kernel handed to somebody's shell after a reboot must not pass: this
    # checkout's supervisor is this script, running out of this checkout. A
    # working directory that cannot be read decides nothing.
    local running where
    running=$(ps -o args= -p "$pid" 2>/dev/null)
    where=$(process_cwd "$pid")
    case "$running" in
        *dev.sh*) ;;
        *)
            rm -f "$META_FILE"
            clear_status
            return 1
            ;;
    esac
    case "$where" in
        ""|"$REPO_ROOT"|"$REPO_ROOT"/*) ;;
        *)
            rm -f "$META_FILE"
            clear_status
            return 1
            ;;
    esac
    return 0
}

# Docker or Podman, whichever this machine has. Both speak the same subcommands
# for everything below, so the only difference is the name of the program.
container_engine() {
    if [ -n "${CRUCIBLE_CONTAINER_ENGINE:-}" ]; then
        echo "$CRUCIBLE_CONTAINER_ENGINE"
    elif command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
        echo docker
    elif command -v podman >/dev/null 2>&1 && podman info >/dev/null 2>&1; then
        echo podman
    else
        echo ""
    fi
}

built_binary() {
    local target
    target=$(cd "$REPO_ROOT" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    [ -n "$target" ] || target="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
    echo "$target/release/$1"
}

pg_ready() {
    "$ENGINE" exec "$PG_CONTAINER" pg_isready -U "$PG_USER" -d "$PG_DB" >/dev/null 2>&1
}

server_ready() {
    curl -sf "http://127.0.0.1:$SERVER_PORT/healthz" >/dev/null 2>&1
}

wait_until() {  # <attempts> <command...>
    local attempts="$1"; shift
    local i=0
    while [ "$i" -lt "$attempts" ]; do
        if "$@"; then return 0; fi
        sleep 1
        i=$((i + 1))
    done
    return 1
}

# Where a process is running from. PIDs are reused, and every checkout runs a
# process with the same name out of a different directory, so the name alone
# cannot tell this checkout's server from the one next door.
process_cwd() {  # <pid>
    if [ -r "/proc/$1/cwd" ]; then
        readlink -f "/proc/$1/cwd" 2>/dev/null
    else
        lsof -a -d cwd -p "$1" -Fn 2>/dev/null | sed -n 's/^n//p' | head -1
    fi
}

# A PID alone is not this server: PIDs are reused, and signalling whatever took
# the number would stop a stranger's process. The executable the PID is running
# has to be the binary THIS checkout built, and the directory it is running out
# of has to be this checkout — another clone's server is the same program with
# the same name, and stopping it is the one thing this script must never do.
server_running() {
    [ -f "$PID_FILE" ] || return 1
    local pid running where wanted
    pid=$(cat "$PID_FILE" 2>/dev/null)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null || { rm -f "$PID_FILE"; return 1; }
    running=$(ps -o args= -p "$pid" 2>/dev/null)
    where=$(process_cwd "$pid")
    wanted=$(built_binary crucible)
    case "$running" in
        "$wanted"*) ;;
        *)
            echo "the recorded pid $pid is running \`$running\`, not $wanted; forgetting it" >&2
            rm -f "$PID_FILE"
            return 1
            ;;
    esac
    if [ "$where" != "$REPO_ROOT" ]; then
        echo "the recorded pid $pid is running out of ${where:-somewhere unreadable}, not $REPO_ROOT; forgetting it" >&2
        rm -f "$PID_FILE"
        return 1
    fi
    return 0
}

# PostgreSQL in a container with no volume: this database is scratch, and a dev
# runner that quietly kept a half-migrated database across a schema change
# would cost more than it saves. A container already up under this name is
# reused, so restarting the server keeps the runs it recorded.
start_postgres() {
    local state
    state=$("$ENGINE" ps -a --filter "name=^${PG_CONTAINER}\$" --format '{{.State}}' 2>/dev/null | head -1)
    [ -n "$state" ] || state="absent"
    case "$state" in
        running) echo "[postgres] reusing $PG_CONTAINER on $POSTGRES_PORT" ;;
        absent)
            echo "[postgres] starting $PG_IMAGE as $PG_CONTAINER on $POSTGRES_PORT"
            update_status postgres "building: creating the container"
            "$ENGINE" run -d --name "$PG_CONTAINER" \
                -e POSTGRES_USER="$PG_USER" \
                -e POSTGRES_PASSWORD="$PG_PASSWORD" \
                -e POSTGRES_DB="$PG_DB" \
                -p "127.0.0.1:$POSTGRES_PORT:5432" \
                "$PG_IMAGE" >/dev/null || {
                update_status postgres "failed: the container would not start"
                return 1
            }
            ;;
        *)
            echo "[postgres] restarting $PG_CONTAINER (was $state)"
            update_status postgres "building: restarting the container"
            "$ENGINE" start "$PG_CONTAINER" >/dev/null || {
                update_status postgres "failed: the container would not start"
                return 1
            }
            ;;
    esac
    update_status postgres "building: waiting for connections"
    if wait_until 60 pg_ready; then
        update_status postgres "ready"
    else
        update_status postgres "failed: never accepted connections"
        echo "[postgres] never accepted connections" >&2
        return 1
    fi
}

# The package manager `studio` is checked in with: bun, whose lockfile is
# the one in the tree. npm is the fallback for a machine without it, and its
# lockfile is not checked in — so the two do not fight over one file.
ui_package_manager() {
    if command -v bun >/dev/null 2>&1; then echo bun; elif command -v npm >/dev/null 2>&1; then echo npm; else echo ""; fi
}

ui_ready() {
    curl -sf "$UI_URL" >/dev/null 2>&1
}

# A PID alone is not this dev server, the same argument as for the server above.
# The process has to be running THIS checkout's Vite binary and to be running it
# out of THIS checkout's studio/ui: every clone runs a process called `vite`,
# and a recycled pid belonging to one of those is exactly what `stop` must leave
# alone.
ui_running() {
    [ -f "$UI_PID_FILE" ] || return 1
    local pid running where
    pid=$(cat "$UI_PID_FILE" 2>/dev/null)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null || { rm -f "$UI_PID_FILE"; return 1; }
    running=$(ps -o args= -p "$pid" 2>/dev/null)
    where=$(process_cwd "$pid")
    case "$running" in
        *"$UI_DIR/node_modules/.bin/vite"*) ;;
        *)
            echo "the recorded UI pid $pid is running \`$running\`, not this checkout's Vite; forgetting it" >&2
            rm -f "$UI_PID_FILE"
            return 1
            ;;
    esac
    if [ "$where" != "$UI_DIR" ]; then
        echo "the recorded UI pid $pid is running out of ${where:-somewhere unreadable}, not $UI_DIR; forgetting it" >&2
        rm -f "$UI_PID_FILE"
        return 1
    fi
    return 0
}

# The wasm the page loads, built from `studio/viewer`. It is not checked
# in — it is a build of that crate — so a checkout that has never built it has
# no UI at all until this runs.
#
# A release wasm build is a minute of everyone's time, and a `start` on a stack
# that is already built and running paid it every time. It is skipped when the
# module is newer than every source it is built from: the viewer crate, the
# renderer it draws through, and the container format it decodes with.
VIEWER_SOURCES=(studio/viewer worldview/worldview-render worldview/worldview worldview/worldview-floor)

build_viewer() {
    local module="$REPO_ROOT/studio/viewer/js/src/wasm/crucible_viewer_bg.wasm"
    local newer=""
    if [ -f "$module" ]; then
        newer=$(cd "$REPO_ROOT" && find "${VIEWER_SOURCES[@]}" -type f -newer "$module" -print -quit 2>/dev/null)
        if [ -z "$newer" ]; then
            echo "[viewer] the built wasm is newer than ${VIEWER_SOURCES[*]}; keeping it"
            update_status viewer "ready: the built wasm is up to date"
            return 0
        fi
    fi
    echo "[viewer] building the wasm (${newer:-no module built yet})"
    update_status viewer "building: cargo and wasm-bindgen"
    if "$REPO_ROOT/studio/viewer/js/build-wasm.sh" release; then
        update_status viewer "ready"
    else
        update_status viewer "failed: the wasm build failed"
        echo "[viewer] wasm build failed; the UI would load nothing" >&2
        return 1
    fi
}

# What the UI needs before Vite can serve it: the dependencies and the wasm the
# page loads.
build_ui() {
    local manager
    update_status ui "starting"
    manager=$(ui_package_manager)
    if [ -z "$manager" ]; then
        update_status ui "failed: no bun and no npm on the PATH"
        echo "[ui] no bun and no npm on the PATH; start it yourself in studio/ui" >&2
        return 1
    fi
    if [ ! -d "$UI_DIR/node_modules" ]; then
        echo "[ui] installing dependencies with $manager"
        update_status ui "building: $manager install"
        (cd "$UI_DIR" && "$manager" install) || {
            update_status ui "failed: the dependencies would not install"
            return 1
        }
    fi
    build_viewer || {
        update_status ui "failed: the wasm the page loads is not built"
        return 1
    }
}

# Vite over `studio/ui`, run until it stops, so that a dev server which falls
# over after answering stops being reported as ready.
run_ui() {
    # Vite is run as the binary rather than through `<manager> run dev`, so the
    # recorded PID is the dev server itself and stopping it stops it.
    echo "[ui] starting Vite on $UI_PORT"
    update_status ui "building: vite"
    local pid
    cd "$UI_DIR" || return 1
    CRUCIBLE_SERVER_PORT="$SERVER_PORT" CRUCIBLE_UI_PORT="$UI_PORT" \
        "$UI_DIR/node_modules/.bin/vite" --host 127.0.0.1 >>"$UI_LOG" 2>&1 &
    pid=$!
    echo "$pid" > "$UI_PID_FILE"
    if wait_until 60 ui_ready; then
        update_status ui "ready"
    else
        update_status ui "failed: never answered on $UI_URL"
        echo "[ui] never answered on $UI_URL; see $UI_LOG" >&2
        tail -20 "$UI_LOG" >&2
    fi
    wait "$pid"
    update_status ui "failed: vite exited"
}

# Every tool this script reaches for, named before anything is built.
#
# A missing tool used to show up as the thing it broke rather than as itself: a
# machine without `curl` waited sixty seconds on a readiness probe that could
# not run and then reported a server that "never answered". Each line below says
# what is missing and how to get it, at the start, where it is cheap to act on.
preflight() {  # <wants-ui: true|false>
    local missing=()
    command -v cargo >/dev/null 2>&1 || missing+=("cargo — install Rust from https://rustup.rs")
    command -v curl  >/dev/null 2>&1 || missing+=("curl — the readiness probes ask the server and the UI over HTTP")
    command -v git   >/dev/null 2>&1 || missing+=("git — the server stamps the checkout's branch and commit on every recording")
    command -v shasum >/dev/null 2>&1 || missing+=("shasum — every port and container name here derives from this checkout's path")
    [ -n "$(container_engine)" ] || missing+=("Docker or Podman, started — PostgreSQL runs in a container")
    if [ "$1" = true ]; then
        command -v wasm-bindgen >/dev/null 2>&1 || missing+=(
            "wasm-bindgen — cargo install -f wasm-bindgen-cli --version \
$(sed -n 's/^wasm-bindgen = \"=\(.*\)\"$/\1/p' "$REPO_ROOT/studio/viewer/Cargo.toml")")
        [ -n "$(ui_package_manager)" ] || missing+=("bun (https://bun.sh) or npm — the UI's dependencies and its dev server")
        # Vite is started as its own binary, whose first line is `#!/usr/bin/env node`;
        # a version manager's shim with no version chosen is on the PATH and
        # still fails, so the check is that node runs, not that it is found.
        node --version >/dev/null 2>&1 || missing+=(
            "a node that runs — Vite's binary starts with #!/usr/bin/env node; with asdf: asdf set -u nodejs \$(asdf list nodejs | tail -1)")
    fi
    [ ${#missing[@]} -eq 0 ] && return 0
    echo "dev.sh: this checkout cannot be brought up without:" >&2
    printf '  %s\n' "${missing[@]}" >&2
    return 1
}

# The server, compiled first rather than through `cargo run`, which is what
# lets this say `building:` and then `failed: build error` — a `cargo run` that
# failed to compile looks exactly like one that failed to bind.
build_server() {
    update_status server "starting"
    echo "[server] building (release)"
    update_status server "building: cargo"
    # The bucket backend is linked only when a bucket was asked for, so the
    # everyday build stays free of the cloud SDK, as the crate intends.
    (cd "$REPO_ROOT" && cargo build --release -p crucible ${STORAGE_BUCKET:+--features gcs}) || {
        update_status server "failed: build error"
        echo "[server] build failed" >&2
        return 1
    }
}

# The server, run as the binary that was just built, until it stops.
run_server() {
    # The provenance the server stamps on every recording, read from the
    # checkout it was just built from. A server deployed away from its source
    # has no git to ask and refuses to start without these, rather than
    # recording a run whose provenance is blank. The schema is brought up to
    # date by the server itself, at startup, from the migrations inside it.
    local run_branch run_commit run_dirty pid
    run_branch=$(cd "$REPO_ROOT" && git rev-parse --abbrev-ref HEAD 2>/dev/null)
    run_commit=$(cd "$REPO_ROOT" && git rev-parse HEAD 2>/dev/null)
    if [ -n "$(cd "$REPO_ROOT" && git status --porcelain --untracked-files=no 2>/dev/null)" ]; then
        run_dirty=true
    else
        run_dirty=false
    fi

    # Exactly one storage variable reaches the server: it refuses both.
    local storage
    if [ -n "$STORAGE_BUCKET" ]; then
        storage="CRUCIBLE_STORAGE_BUCKET=$STORAGE_BUCKET"
        echo "[server] starting; registry $DATABASE_URL; storage gs://$STORAGE_BUCKET (your gcloud credentials)"
    else
        storage="CRUCIBLE_STORAGE_DIR=$STORAGE_DIR"
        echo "[server] starting; registry $DATABASE_URL; storage $STORAGE_DIR"
    fi
    update_status server "building: migrations and bind"
    cd "$REPO_ROOT" || return 1
    env BIND="127.0.0.1:$SERVER_PORT" \
    DATABASE_URL="$DATABASE_URL" \
    "$storage" \
    CRUCIBLE_TEMP_DIR="$TEMP_DIR" \
    DESVIS_RUN_BRANCH="$run_branch" \
    DESVIS_RUN_COMMIT="$run_commit" \
    DESVIS_RUN_DIRTY="$run_dirty" \
    RUST_LOG="${RUST_LOG:-info}" \
        "$(built_binary crucible)" >>"$SERVER_LOG" 2>&1 &
    pid=$!
    echo "$pid" > "$PID_FILE"
    if wait_until 60 server_ready; then
        update_status server "ready"
    else
        update_status server "failed: never answered healthz"
        echo "[server] never answered healthz; see $SERVER_LOG" >&2
        tail -20 "$SERVER_LOG" >&2
    fi
    # Only reached when the server ends, which after `ready` means it fell
    # over: the status has to stop saying otherwise.
    wait "$pid"
    update_status server "failed: the server exited"
}

# Wait for the units to come up, saying what each is doing as it does it. This
# is what makes `start` feel like it did before the supervisor existed: the
# command holds the terminal until there is something to open, and a unit that
# gives up says so here rather than in a log nobody is reading yet.
await_units() {  # <with-ui: true|false>
    local waited=0 said="" now=""
    while [ "$waited" -lt 1200 ]; do
        supervisor_running || { echo "[supervisor] exited; see $SUPERVISOR_LOG" >&2; return 1; }
        for unit in $UNITS; do
            case "$(read_status "$unit")" in
                failed*) echo "[$unit] $(read_status "$unit")" >&2; return 1 ;;
            esac
        done
        now="$(read_status postgres) · $(read_status viewer) · $(read_status server) · $(read_status ui)"
        [ "$now" = "$said" ] || { echo "  $now"; said="$now"; }
        # The units' own word for it, not a second probe of the same ports:
        # what this command waited for is then what the dashboard shows.
        if [ "$(read_status server)" = ready ]; then
            [ "$1" = false ] && return 0
            [ "$(read_status ui)" = ready ] && return 0
        fi
        sleep 0.5
        waited=$((waited + 1))
    done
    echo "dev.sh: the stack did not come up within ten minutes; see $SUPERVISOR_LOG" >&2
    return 1
}

cmd_start() {
    local with_ui=true
    while [ $# -gt 0 ]; do
        case "$1" in
            --no-ui) with_ui=false ;;
            --bucket)
                [ -n "${2:-}" ] || { echo "dev.sh: --bucket needs a bucket name" >&2; exit 1; }
                STORAGE_BUCKET="$2"
                shift
                ;;
            *) echo "dev.sh: start does not take $1" >&2; exit 1 ;;
        esac
        shift
    done
    preflight "$with_ui" || exit 1
    ENGINE=$(container_engine)
    echo "[engine] $ENGINE"
    mkdir -p "$LOG_DIR" "$STATE_DIR" "$STORAGE_DIR" "$TEMP_DIR" "$REGISTRY_DIR"

    # A stack already up is left alone: one checkout has one supervisor, and a
    # second `start` says where the first one answers rather than building a
    # second stack onto the same ports.
    if supervisor_running; then
        echo "[supervisor] already running for this checkout (pid $(meta_value PID))"
        if [ -n "$STORAGE_BUCKET" ]; then
            echo "dev.sh: --bucket $STORAGE_BUCKET is ignored: the running stack keeps its storage; \`stop\` it first" >&2
        fi
        cmd_status
        exit 0
    fi

    # A previous run that was killed outright leaves status files describing
    # processes that are gone; they go before this run writes its own.
    clear_status
    for unit in $UNITS; do update_status "$unit" "starting"; done
    # The wasm the page loads is built for the page, so without a UI neither
    # unit has anything to do and neither should sit at `starting` for ever.
    if [ "$with_ui" = false ]; then
        update_status ui "stopped: started with --no-ui"
        update_status viewer "stopped: started with --no-ui"
    fi

    # `set -m` puts the subshell in a process group of its own, which is what
    # makes its PID a process-group id: one signal to it reaches the server and
    # Vite together, whether it comes from `stop`, from Sutra or from a hand.
    set -m
    (
        set +e
        # Stand in the checkout, wherever `start` was typed: it is half of how
        # `supervisor_running` tells this process from a stranger with its PID.
        cd "$REPO_ROOT" || exit 1
        # macOS ships bash 3.2, which has no $BASHPID, so the subshell asks the
        # kernel for the group it leads.
        MY_PGID=$(sh -c 'ps -o pgid= -p $$' | tr -d ' ')
        MY_PID=$(sh -c 'echo $PPID')
        # `set -m` is what should have made this subshell lead a group of its
        # own. Where it did not, the group is the caller's — the terminal's, or
        # a script's — and signalling it would take strangers down, so the
        # supervisor refuses to start rather than publish a PID `stop` and Sutra
        # would signal as a group.
        if [ -z "$MY_PGID" ] || [ "$MY_PGID" != "$MY_PID" ]; then
            echo "dev.sh: the supervisor (pid $MY_PID) does not lead its own process group (group ${MY_PGID:-unknown}); not starting" >&2
            update_status server "failed: the supervisor could not take a process group of its own"
            exit 1
        fi
        cleanup() {
            trap - INT TERM HUP EXIT
            rm -f "$META_FILE"
            clear_status
            [ -n "$MY_PGID" ] && [ "$MY_PGID" = "$MY_PID" ] && kill -- -"$MY_PGID" 2>/dev/null
            true
        }
        trap cleanup INT TERM HUP EXIT

        # Built one after another, never at once: both builds are cargo, and
        # two cargo builds over one target directory take turns on its lock —
        # in parallel they finish later than in sequence, and the wait shows up
        # as a start that takes half as long again.
        start_postgres || exit 1
        build_server || exit 1
        run_server &
        if [ "$with_ui" = true ]; then
            build_ui || exit 1
            run_ui &
        fi
        wait
    ) >>"$SUPERVISOR_LOG" 2>&1 &
    local pid=$!
    set +m
    # Off this shell's job table: the supervisor reaps its own group, and a
    # tracked job dying that way makes bash print `Terminated: 15` over
    # whatever the person is reading.
    disown "$pid" 2>/dev/null || true
    write_meta "$pid" "$with_ui"

    if ! await_units "$with_ui"; then
        kill -TERM "$pid" 2>/dev/null
        exit 1
    fi

    echo
    [ "$with_ui" = true ] && echo "  UI          $UI_URL"
    echo "  websocket   $SERVER_WS"
    echo "  health      http://127.0.0.1:$SERVER_PORT/healthz"
    echo "  supervisor  pid $pid — ./dev.sh stop, or Sutra's terminate button"
    echo "  submit      studio/runs/dev.sh submit sim/dessim/tests/fixtures/cencora_v3_spec.json --simtime 10 --threads 1 --wait   (from the repository root)"
    echo "  logs        ./dev.sh logs   ·   ./dev.sh logs ui   ·   ./dev.sh logs supervisor"
}

# One process this checkout recorded, stopped if it is still what it says.
stop_recorded() {  # <name> <pid file> <identity check>
    "$3" || return 0
    local pid i=0
    pid=$(cat "$2")
    echo "stopping the $1 (pid $pid)"
    kill -TERM "$pid" 2>/dev/null
    while [ "$i" -lt 20 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.5; i=$((i + 1)); done
    # Ten seconds is long enough for the PID to have become somebody else's:
    # the identity is asked again before the signal nothing can refuse.
    if kill -0 "$pid" 2>/dev/null && "$3" 2>/dev/null; then kill -9 "$pid" 2>/dev/null; fi
}

# Stopping is one signal to the supervisor, which is what Sutra's terminate
# button sends too: its own trap takes down the server and Vite with it, and
# removes this checkout's registry entry. The signal goes to the process group
# first so a child that has stopped listening still goes.
cmd_stop() {
    ENGINE=$(container_engine)
    if supervisor_running; then
        local pid i=0
        pid=$(meta_value PID)
        echo "stopping this checkout's stack (supervisor pid $pid)"
        kill -TERM -- -"$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null
        while [ "$i" -lt 20 ] && kill -0 "$pid" 2>/dev/null; do sleep 0.5; i=$((i + 1)); done
        # Asked again for the same reason as in `stop_recorded`: the PID has
        # had ten seconds to change hands.
        if kill -0 "$pid" 2>/dev/null && supervisor_running; then
            kill -9 -- -"$pid" 2>/dev/null || kill -9 "$pid" 2>/dev/null
        fi
    else
        echo "no supervisor is running for this checkout"
        # A stack this checkout started before it had a supervisor: the two
        # recorded pids are still this checkout's own server and Vite, checked
        # against the binary each is running and the directory it runs from, so
        # `stop` still stops what `start` started.
        stop_recorded server "$PID_FILE" server_running
        stop_recorded ui "$UI_PID_FILE" ui_running
    fi
    rm -f "$META_FILE" "$PID_FILE" "$UI_PID_FILE"
    clear_status

    # The container is not a child of the server, so it survives on purpose:
    # restarting against a database that still holds its runs is the point of a
    # container named after the checkout.
    if [ "${1:-}" = "--db" ]; then
        if [ -n "$ENGINE" ] && "$ENGINE" rm -f "$PG_CONTAINER" >/dev/null 2>&1; then
            echo "removed $PG_CONTAINER and its data"
        else
            echo "no $PG_CONTAINER to remove"
        fi
    elif [ -n "$ENGINE" ] && [ -n "$("$ENGINE" ps -a --filter "name=^${PG_CONTAINER}\$" --format '{{.Names}}' 2>/dev/null)" ]; then
        echo "$PG_CONTAINER is still up, holding its rows. ./dev.sh stop --db removes it."
    fi
}

cmd_status() {
    ENGINE=$(container_engine)
    local pg="absent"
    if [ -n "$ENGINE" ]; then
        pg=$("$ENGINE" ps -a --filter "name=^${PG_CONTAINER}\$" --format '{{.State}}' 2>/dev/null | head -1)
        [ -n "$pg" ] || pg="absent"
    fi
    echo "crucible ($REGISTRY_KEY)"
    printf "  %-10s %s\n" "engine" "${ENGINE:-none}"
    # What the dashboard is being told, unit by unit, beside what this machine
    # says about the same things: a disagreement between the two columns is
    # this script failing to keep its own registry entry honest.
    if supervisor_running; then
        printf "  %-10s running (pid %s), up %ss\n" "supervisor" "$(meta_value PID)" \
            "$(( $(date +%s) - $(meta_value STARTED) ))"
    else
        printf "  %-10s stopped\n" "supervisor"
    fi
    printf "  %-10s %s · container %s on %s\n" "postgres" "$(read_status postgres)" "$pg" "$POSTGRES_PORT"
    printf "  %-10s %s\n" "viewer" "$(read_status viewer)"
    if server_running; then
        printf "  %-10s %s · pid %s on %s\n" "server" "$(read_status server)" "$(cat "$PID_FILE")" "$SERVER_WS"
    else
        printf "  %-10s %s · no process of this checkout's\n" "server" "$(read_status server)"
    fi
    if ui_running; then
        printf "  %-10s %s · pid %s on %s\n" "ui" "$(read_status ui)" "$(cat "$UI_PID_FILE")" "$UI_URL"
    else
        printf "  %-10s %s · no process of this checkout's\n" "ui" "$(read_status ui)"
    fi
    printf "  %-10s %s\n" "storage" "$STORAGE_DIR"
    printf "  %-10s %s\n" "sutra" "$META_FILE"
}

cmd_logs() {
    local log="$SERVER_LOG"
    case "${1:-}" in
        ui) log="$UI_LOG" ;;
        supervisor) log="$SUPERVISOR_LOG" ;;
    esac
    [ -f "$log" ] || { echo "no log yet at $log" >&2; exit 1; }
    echo "$log (Ctrl+C to detach)"
    tail -f "$log"
}

# The terminal client, built beside the server and pointed at it.
cmd_submit() {
    (cd "$REPO_ROOT" && cargo build --release -p crucible --bin crucible-cli) || exit 1
    CRUCIBLE_SERVER="$SERVER_WS" "$(built_binary crucible-cli)" submit "$@"
}

cmd_client() {
    (cd "$REPO_ROOT" && cargo build --release -p crucible --bin crucible-cli) || exit 1
    CRUCIBLE_SERVER="$SERVER_WS" "$(built_binary crucible-cli)" "$@"
}

gcs_ready() {
    curl -sf "$GCS_URL/storage/v1/b" >/dev/null 2>&1
}

# The emulator, up with the test bucket in it, and the two variables the suite
# reads printed as a line to paste. A container already up is reused; `stop`
# removes it, objects and all, because nothing in it is worth keeping.
cmd_gcs_emulator() {  # [stop]
    ENGINE=$(container_engine)
    [ -n "$ENGINE" ] || { echo "dev.sh: Docker or Podman, started, is needed to run the emulator" >&2; exit 1; }
    if [ "${1:-}" = "stop" ]; then
        if "$ENGINE" rm -f "$GCS_CONTAINER" >/dev/null 2>&1; then echo "removed $GCS_CONTAINER"; else echo "no $GCS_CONTAINER to remove"; fi
        return 0
    fi
    local state image
    state=$("$ENGINE" ps -a --filter "name=^${GCS_CONTAINER}\$" --format '{{.State}}' 2>/dev/null | head -1)
    # A container under this name running some other image is replaced, not
    # reused: the image changed once already, and a stale emulator kept alive
    # by its name fails the suite in a way that looks like the backend's fault.
    image=$("$ENGINE" inspect --format '{{.Config.Image}}' "$GCS_CONTAINER" 2>/dev/null)
    if [ -n "$state" ] && [ "$image" != "$GCS_IMAGE" ]; then
        echo "[gcs] $GCS_CONTAINER runs $image, not $GCS_IMAGE; replacing it"
        "$ENGINE" rm -f "$GCS_CONTAINER" >/dev/null 2>&1
        state=""
    fi
    case "${state:-absent}" in
        running) echo "[gcs] reusing $GCS_CONTAINER on $GCS_PORT" ;;
        absent)
            echo "[gcs] starting $GCS_IMAGE as $GCS_CONTAINER on $GCS_PORT"
            # `-public-host` is what the emulator writes into the links it
            # answers with; it has to be the address this machine reaches it at.
            "$ENGINE" run -d --name "$GCS_CONTAINER" --platform linux/amd64 -p "127.0.0.1:$GCS_PORT:4443" \
                "$GCS_IMAGE" -scheme http -backend memory -public-host "127.0.0.1:$GCS_PORT" >/dev/null || exit 1
            ;;
        *)
            echo "[gcs] restarting $GCS_CONTAINER (was $state)"
            "$ENGINE" start "$GCS_CONTAINER" >/dev/null || exit 1
            ;;
    esac
    wait_until 30 gcs_ready || { echo "[gcs] never answered on $GCS_PORT" >&2; exit 1; }
    # The bucket has to exist before anything is written to it. Asking for one
    # that already exists is refused, which is the answer wanted.
    curl -sf -X POST -H 'Content-Type: application/json' --data "{\"name\":\"$GCS_BUCKET\"}" \
        "$GCS_URL/storage/v1/b" >/dev/null 2>&1 || true
    echo "export RUNSTORE_GCS_EMULATOR=$GCS_URL RUNSTORE_GCS_BUCKET=$GCS_BUCKET"
    echo "cargo test -p runstore --features gcs"
}

case "${1:-start}" in
    start)  shift || true; cmd_start "$@" ;;
    stop)   shift || true; cmd_stop "$@" ;;
    status) shift || true; cmd_status "$@" ;;
    logs)   shift || true; cmd_logs "$@" ;;
    submit) shift; cmd_submit "$@" ;;
    # `status` is this script's own; the client's is `status-of <run>`, which is
    # the verb the client answers to once the script's own word is out of the way.
    status-of) shift; cmd_client status "$@" ;;
    list|cancel|attach|series) cmd_client "$@" ;;
    gcs-emulator) shift; cmd_gcs_emulator "$@" ;;
    -h|--help|help)
        sed -n '2,38p' "$0" | sed 's/^# \{0,1\}//'
        ;;
    *)
        echo "dev.sh: no such command: $1" >&2
        exit 1
        ;;
esac
