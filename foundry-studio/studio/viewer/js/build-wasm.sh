#!/bin/bash
# Build the browser viewer's wasm and put it where this package's TypeScript
# finds it.
#
# The generated files are not checked in: they are a build of
# `studio/viewer` and would otherwise be a second, staler copy of it.
# `src/wasm/` holds the module and its declarations while tsc typechecks
# against them, and `npm run build` copies the runtime halves into `dist/`.
#
# The build writes into a fresh directory and moves it into place, so what
# `src/wasm/` holds afterwards is exactly what this build produced. Writing
# over the last build would leave a file whose source has since been renamed
# or deleted sitting there, and the package would then depend on what happened
# to be built here before.
#
# wasm-bindgen's generated glue only loads a module its OWN version produced,
# so the command-line tool and the crate must be the same version; that is
# checked here rather than discovered as an unreadable module in a tab.
set -euo pipefail

PACKAGE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$PACKAGE_DIR/../../.." && pwd)"
PINNED=$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' "$REPO_ROOT/studio/viewer/Cargo.toml")
INSTALLED=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')

if [ -z "$INSTALLED" ]; then
    echo "build-wasm.sh: no wasm-bindgen on the PATH; cargo install -f wasm-bindgen-cli --version $PINNED" >&2
    exit 1
fi
if [ "$INSTALLED" != "$PINNED" ]; then
    echo "build-wasm.sh: wasm-bindgen $INSTALLED is installed and this crate is built against $PINNED" >&2
    echo "               cargo install -f wasm-bindgen-cli --version $PINNED" >&2
    exit 1
fi

PROFILE="${1:-release}"
case "$PROFILE" in
    release) FLAGS=(--release) ;;
    debug)   FLAGS=() ;;
    *) echo "build-wasm.sh: the profiles are release and debug, not $PROFILE" >&2; exit 1 ;;
esac

cd "$REPO_ROOT"
cargo build -p crucible-viewer --target wasm32-unknown-unknown "${FLAGS[@]}"
# Where cargo put it, which CARGO_TARGET_DIR or a cargo config moves away from
# the checkout's own target/; cargo itself is asked rather than guessed after.
TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')

FRESH=$(mktemp -d "$PACKAGE_DIR/.wasm-build-XXXXXX")
trap 'rm -rf "$FRESH"' EXIT
wasm-bindgen --target web --out-dir "$FRESH" "${TARGET_DIR:-target}/wasm32-unknown-unknown/$PROFILE/crucible_viewer.wasm"

# A size pass, when the tool is installed. It rewrites the module in place and
# changes nothing the glue beside it names, so a build without it is the same
# viewer and a larger download.
if command -v wasm-opt >/dev/null 2>&1; then
    wasm-opt -Oz "$FRESH/crucible_viewer_bg.wasm" -o "$FRESH/crucible_viewer_bg.wasm.opt"
    mv "$FRESH/crucible_viewer_bg.wasm.opt" "$FRESH/crucible_viewer_bg.wasm"
else
    echo "build-wasm.sh: no wasm-opt on the PATH; the module ships unoptimised (cargo install wasm-opt)" >&2
fi

rm -rf "$PACKAGE_DIR/src/wasm"
mv "$FRESH" "$PACKAGE_DIR/src/wasm"
trap - EXIT
echo "built $PROFILE into $PACKAGE_DIR/src/wasm ($(wc -c < "$PACKAGE_DIR/src/wasm/crucible_viewer_bg.wasm") bytes)"
