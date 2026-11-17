#!/bin/bash
# Put the terminal client through every command it answers, against a server
# that is already up, and fail if any of them fails or prints the server's
# greeting.
#
# The greeting is why this gate exists. The server says what it accepts in one
# message as soon as a socket opens, before anything is asked for, and a client
# that took that first message for its own answer failed `list`, `series` and
# `submit` and printed the greeting as noise out of `attach`. Nothing else in
# the gates runs this client, so that went through every one of them green.
#
# usage: cli_gate.sh <server-ws-url> <results-dir>
#   The walk is written to <results-dir>/cli-transcript.txt, commands and
#   output together. CLI=<path> runs that binary instead of building this
#   checkout's own.
set -u

server=${1:-}
results=${2:-}
if [ -z "$server" ] || [ -z "$results" ]; then
    echo "usage: cli_gate.sh <server-ws-url> <results-dir>" >&2
    exit 2
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
checkout="$(cd "$here/../.." && pwd)"
spec="sim/dessim/tests/fixtures/minimal_easyloop_flow_spec.json"

mkdir -p "$results"
transcript="$results/cli-transcript.txt"
: > "$transcript"

CLI=${CLI:-}
if [ -z "$CLI" ]; then
    (cd "$checkout" && cargo build --release -p crucible --bin crucible-cli) || exit 1
    target=$(cd "$checkout" && cargo metadata --format-version 1 --no-deps 2>/dev/null \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    [ -n "$target" ] || target="$checkout/target"
    CLI="$target/release/crucible-cli"
fi
[ -x "$CLI" ] || { echo "cli_gate: no terminal client at $CLI" >&2; exit 1; }

failed=0
OUT=""
errors="$results/.cli-stderr"

# One command. What it printed on stdout is left in OUT for the steps below,
# because that is where a run's id and a metric's name arrive; the transcript
# takes both streams, with the lines this client wrote to stderr marked `#`.
# A command that fails, or that prints the greeting, fails the gate.
walk() {  # <what> <argument...>
    local what="$1"; shift
    printf '\n$ crucible-cli %s\n' "$*" >> "$transcript"
    local status=0
    # The command is the first argument this client reads, so the server it is
    # pointed at goes after it.
    OUT=$(cd "$checkout" && "$CLI" "$@" --server "$server" 2>"$errors") || status=$?
    [ -n "$OUT" ] && printf '%s\n' "$OUT" >> "$transcript"
    [ -s "$errors" ] && sed 's/^/# /' "$errors" >> "$transcript"
    if [ "$status" -ne 0 ]; then
        echo "cli_gate: $what failed (exit $status)" >&2
        tail -3 "$errors" >&2
        failed=$((failed + 1))
        return 1
    fi
    if { printf '%s\n' "$OUT"; cat "$errors"; } | grep -qi 'hello'; then
        echo "cli_gate: $what printed the server's greeting" >&2
        failed=$((failed + 1))
        return 1
    fi
    return 0
}

printf 'the terminal client against %s\n%s\n' "$server" "$(date)" >> "$transcript"

# Two simulated minutes, watched to its end: `--wait` prints the run's id and
# then a line every time the run's state or progress changes.
walk "submit --wait" submit "$spec" --simtime 2 --threads 1 --name cli-gate-short --wait || exit 1
run=$(printf '%s' "$OUT" | head -1)
[ -n "$run" ] || { echo "cli_gate: the submit printed no run id" >&2; exit 1; }
echo "cli_gate: the run is $run"

walk "list" list || exit 1
printf '%s' "$OUT" | grep -q "$run" \
    || { echo "cli_gate: the list does not name $run" >&2; exit 1; }

walk "status" status "$run" || exit 1
walk "series (every metric)" series "$run" || exit 1
metric=$(printf '%s' "$OUT" | head -1 | awk '{print $1}')
[ -n "$metric" ] || { echo "cli_gate: the run states no metric to ask for" >&2; exit 1; }
walk "series $metric" series "$run" "$metric" || exit 1

walk "attach from 0" attach "$run" 0 || exit 1
walk "attach from 60000" attach "$run" 60000 || exit 1

# A longer run, stopped part way: the cancel is answered, and the status after
# it says what became of the attempt.
walk "submit (to cancel)" submit "$spec" --simtime 30 --threads 1 --name cli-gate-long || exit 1
longer=$(printf '%s' "$OUT" | head -1)
[ -n "$longer" ] || { echo "cli_gate: the second submit printed no run id" >&2; exit 1; }
walk "cancel" cancel "$longer" || exit 1
walk "status (cancelled)" status "$longer" || exit 1

rm -f "$errors"
if [ "$failed" -ne 0 ]; then
    echo "cli_gate: $failed command(s) went wrong; $transcript has the walk" >&2
    exit 1
fi
echo "cli_gate: every command answered; $transcript has the walk"
