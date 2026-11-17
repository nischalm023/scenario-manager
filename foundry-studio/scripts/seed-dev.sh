#!/bin/bash
#
# seed-dev.sh — fill a development world-proto server with Foundry's application and the walmart-dallas demo site.
#
# WPS ships no application: it holds whatever an application registers. This registers Foundry's: the generator
# modules built from studio/world-generators, the core component vocabulary (data/seed/application/core-components.json,
# schemas derived from data/seed/application/schema), the Map Creator and Agent Studio vocabularies, and the lenses
# the demo site was written for (the WPS-era library preserved under data/seed/application-tests/lenses/legacy-wps);
# then it loads the walmart-dallas catalog into module/go-catalog and the site, pinned to it, into walmart/dallas, and
# the server derives the rest inside each changeset.
#
# dev.sh runs it on every start, so most runs meet a database an earlier run seeded. A definition or version the
# server already holds answers 409, and the seed compares what is stored with what it would publish: a different
# schema or reference annotations stops the seed, since the server keeps the first publish of a version; a different
# rung is reported and left. A scope past sequence zero is left as it stands.
#
# Usage: scripts/seed-dev.sh --server URL --world-proto DIR [--author WHO]
#
# DIR is the world-proto checkout the server was built from: its target/debug/world-graph-cli publishes the lenses
# and applies changesets, and its generator SDK builds the modules.

set -eu

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

FOUNDRY="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APPLICATION="$FOUNDRY/data/seed/application"
SITE_INPUTS="$FOUNDRY/data/seed/application-tests/wps-sites/examples/walmart-dallas"
LENSES="$FOUNDRY/data/seed/application-tests/lenses/legacy-wps/shipped.json"
SCENARIO="$SITE_INPUTS/scenario.world.json"
CATALOG="$SITE_INPUTS/catalog.world.json"
GENERATORS="$FOUNDRY/studio/world-generators"

SERVER=""
WORLD_PROTO=""
AUTHOR="seed-dev"
TENANT="walmart"
SITE="dallas"
MODULE="go-catalog"

while [ $# -gt 0 ]; do
    case "$1" in
        --server)      SERVER="$2"; shift 2 ;;
        --world-proto) WORLD_PROTO="$2"; shift 2 ;;
        --author)      AUTHOR="$2"; shift 2 ;;
        -h|--help)     sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
[ -n "$SERVER" ] || { echo "seed-dev.sh: --server is required" >&2; exit 2; }
[ -n "$WORLD_PROTO" ] || { echo "seed-dev.sh: --world-proto is required" >&2; exit 2; }
WORLD_PROTO="$(cd "$WORLD_PROTO" && pwd -P)"
CLI="$WORLD_PROTO/target/debug/world-graph-cli"
MODULE_PATH="module/$MODULE"
SITE_PATH="$TENANT/$SITE"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
RESPONSE="$WORK/response.json"
HTTP_STATUS=""

request() {
    local method="$1" path="$2"; shift 2
    HTTP_STATUS=$(curl -sS -X "$method" "$SERVER$path" -H 'content-type: application/json' \
                      -o "$RESPONSE" -w '%{http_code}' "$@")
}

body() {
    cat "$RESPONSE"
}

now_ms() {
    python3 -c 'import time; print(int(time.time() * 1000))'
}

RESET_HINT="reset the database: ./dev.sh --stop --db, then ./dev.sh start"

# ---------------------------------------------------------------------------
# What a 409 does not say
#
# The server keeps the first definition of a name and the first publish of a `component@version`, and answers every
# later one 409, whatever it carries. Every schema here goes up as 0.1.0 while the format is unreleased, so a database
# seeded before the vocabulary changed would go on validating writes against its first schemas. After a 409 the seed
# reads the stored row back and compares it.
# ---------------------------------------------------------------------------

# After a define answered 409: report, and return 1, when the stored rung of $1 differs from $2. The server's rung
# stays, because a rung moves through the registry's governance edit and may be a ruling somebody made.
check_rung() {
    local component="$1" rung="$2" stored
    request GET "/registry/$component"
    [ "$HTTP_STATUS" = "200" ] || {
        echo -e "${RED}read $component back failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1; }
    stored=$(body | jq -r '.component.status')
    [ "$stored" != "$rung" ] || return 0
    echo -e "  ${YELLOW}$component's rung differs: $stored on this server, $rung in this seed${NC}. The seed keeps $stored."
    return 1
}

# After a publish answered 409: stop unless the server holds the same schema and reference annotations for $1 at the
# version payload $2 names. Both compare as JSON values, the annotations as a set of `path` and `expects`.
check_published() {
    local component="$1" payload="$2" version verdict what
    version=$(jq -r '.version' <<<"$payload")
    request GET "/registry/$component"
    [ "$HTTP_STATUS" = "200" ] || {
        echo -e "${RED}read $component back failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1; }
    verdict=$(body | jq -r --argjson p "$payload" '
        def annotations: (. // []) | map({path} + (if .expects == null then {} else {expects} end))
                         | sort_by(.path, .expects // "");
        (first(.versions[] | select(.version == $p.version)) // {}) as $s
        | "\($s.schema == $p.schema) \(($s.refs | annotations) == ($p.refs | annotations))"')
    case "$verdict" in
        "true true")  return 0 ;;
        "false true") what="its schema differs" ;;
        "true false") what="its reference annotations differ" ;;
        *)            what="its schema and its reference annotations differ" ;;
    esac
    echo -e "${RED}publish $component@$version failed (409):${NC} the server already holds a different $component@$version: $what." >&2
    echo "  The database predates a vocabulary change, and a published version cannot be replaced. To go on, $RESET_HINT." >&2
    exit 1
}

# Define component $1 from definition $2 ({owner, status, meaning, asserts, generator?}), then publish payload $3
# ({version, schema, refs}). Counts land in the caller's DEFINED, ALREADY, PUBLISHED, MATCHED and DRIFTED.
define_and_publish() {
    local component="$1" definition="$2" payload="$3"
    request POST /registry --data "$definition"
    case "$HTTP_STATUS" in
        201) DEFINED=$((DEFINED + 1)) ;;
        409)
            ALREADY=$((ALREADY + 1))
            # A definition is not re-POSTable; the generator it names is restated through the governance edit.
            if jq -e 'has("generator")' <<<"$definition" >/dev/null; then
                request PATCH "/registry/$component" --data "$(jq -c '{generator}' <<<"$definition")"
                [ "$HTTP_STATUS" = "200" ] || {
                    echo -e "${RED}name the generator of $component failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1; }
            fi
            check_rung "$component" "$(jq -r '.status' <<<"$definition")" || DRIFTED=$((DRIFTED + 1))
            ;;
        *) echo -e "${RED}define $component failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1 ;;
    esac
    request POST "/registry/$component/versions" --data "$payload"
    case "$HTTP_STATUS" in
        201) PUBLISHED=$((PUBLISHED + 1)) ;;
        409) check_published "$component" "$payload"; MATCHED=$((MATCHED + 1)) ;;
        *) echo -e "${RED}publish $component@0.1.0 failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1 ;;
    esac
}

counted() {
    if [ "$DRIFTED" -gt 0 ]; then
        echo "    Each differing rung may be a ruling; if the database instead predates a vocabulary change, $RESET_HINT."
    fi
    echo -e "  ${GREEN}✓${NC} $DEFINED defined, $ALREADY already there, $PUBLISHED versions published at 0.1.0, \
$MATCHED already there and the same"
}

# ---------------------------------------------------------------------------
# The generator modules, published before the vocabulary that names them: a component names only a generator with a
# published module. The sources' workspace manifest is a template with the world-proto and Foundry paths filled in,
# rendered into a build directory that stays put between runs so cargo rebuilds only what changed.
# ---------------------------------------------------------------------------

publish_generators() {
    echo -e "${BOLD}Registry${NC} — building and publishing Foundry's generator modules"
    local build="$FOUNDRY/target/seed-dev-generators"
    local sources="$build/sources"
    mkdir -p "$sources"
    rsync -a --delete --exclude target --exclude tests --exclude '__pycache__' "$GENERATORS/" "$sources/"
    sed -e "s#@WPS@#$WORLD_PROTO#g" -e "s#@FOUNDRY@#$FOUNDRY#g" "$GENERATORS/Cargo.toml.in" > "$sources/Cargo.toml"
    # The lock file comes along with the sources, and cargo updates it here for this checkout's path dependencies.
    if ! (unset CARGO_TARGET_DIR && cargo build --quiet --release --target wasm32-wasip2 \
            --manifest-path "$sources/Cargo.toml" --target-dir "$build/target"); then
        echo -e "${RED}building the generators failed${NC}: cargo's output is above (rustup target add wasm32-wasip2 if the target is missing)" >&2
        exit 1
    fi
    local crate source name version meaning module digest published=0
    for crate in tiling stamp rack cohort carrier_pool; do
        source="$GENERATORS/$crate/src/lib.rs"
        name=$(sed -nE 's/^ *const NAME: &.static str = "([^"]*)";/\1/p' "$source" | head -1)
        version=$(sed -nE 's/^ *const VERSION: &.static str = "([^"]*)";/\1/p' "$source" | head -1)
        meaning=$(sed -nE 's/^description = "[^:]*: (.*)"$/\1/p' "$GENERATORS/$crate/Cargo.toml")
        module="$build/target/wasm32-wasip2/release/seed_generator_$crate.wasm"
        digest=$(shasum -a 256 "$module" | cut -d' ' -f1)
        HTTP_STATUS=$(curl -sS -X PUT "$SERVER/registry/generators/modules/$digest" \
                          -H 'content-type: application/wasm' --data-binary "@$module" \
                          -o "$RESPONSE" -w '%{http_code}')
        [ "$HTTP_STATUS" = "200" ] || { echo -e "${RED}upload $name failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1; }
        request POST /registry/generators --data "$(jq -n --arg g "$name" --arg m "$meaning" --arg a "$AUTHOR" \
            '{generator: $g, owner: "world-team", status: "candidate", meaning: $m,
              author: $a, message: "seed the generator `\($g)`"}')"
        case "$HTTP_STATUS" in
            201|409) : ;;
            *) echo -e "${RED}define $name failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1 ;;
        esac
        request POST "/registry/generators/$name/versions" --data "$(jq -n --arg v "$version" --arg d "$digest" \
            --arg g "$name" --arg a "$AUTHOR" '{version: $v, digest: $d, author: $a, message: "publish \($g)@\($v)"}')"
        case "$HTTP_STATUS" in
            201) published=$((published + 1)) ;;
            # A module rebuilt at the same version under a new digest is staged but not published.
            409) : ;;
            *) echo -e "${RED}publish $name@$version failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1 ;;
        esac
    done
    echo -e "  ${GREEN}✓${NC} $published generator modules published (tiling, stamp, rack, cohort, carrier_pool)"
}

# ---------------------------------------------------------------------------
# The vocabularies
# ---------------------------------------------------------------------------

# The core components: meaning, rung and generator from core-components.json, each schema derived at seed time from
# the protocol schema layer (a committed derived copy would go stale unnoticed).
define_core_vocabulary() {
    echo -e "${BOLD}Registry${NC} — defining the core component vocabulary"
    local core="$APPLICATION/core-components.json" schemas="$WORK/schemas.json" component
    # shellcheck disable=SC2046
    FOUNDRY_WPS_ROOT="$WORLD_PROTO" python3 "$FOUNDRY/scripts/derive-component-schemas.py" \
        --out "$schemas" $(jq -r 'keys[]' "$core")
    DEFINED=0 ALREADY=0 PUBLISHED=0 MATCHED=0 DRIFTED=0
    for component in $(jq -r 'keys_unsorted[]' "$core"); do
        define_and_publish "$component" \
            "$(jq -c --arg c "$component" '.[$c] | {component: $c, owner: "world-team", status, meaning, asserts, generator}' "$core")" \
            "$(jq -nc --arg c "$component" --slurpfile s "$schemas" --slurpfile d "$core" \
                '{version: "0.1.0", schema: $s[0][$c], refs: $d[0][$c].refs}')"
    done
    counted
}

# Every definition file under folder $1 ({component, owner, status, meaning, asserts, schema, refs}), named $2 in
# what this prints. Every name is defined before any schema is published, since an annotation may name another of
# the folder's components.
define_vocabulary_folder() {
    local folder="$1" what="$2" files file
    echo -e "${BOLD}Registry${NC} — defining $what"
    files=$(find "$folder" -type f -name '*.json' | LC_ALL=C sort)
    [ -n "$files" ] || { echo -e "${RED}no definition files under $folder${NC}" >&2; exit 1; }
    DEFINED=0 ALREADY=0 PUBLISHED=0 MATCHED=0 DRIFTED=0
    while IFS= read -r file; do
        define_and_publish "$(jq -r '.component' "$file")" \
            "$(jq -c '{component, owner, status, meaning, asserts}' "$file")" \
            "$(jq -c '{version: "0.1.0", schema, refs}' "$file")"
    done <<EOF
$files
EOF
    counted
}

# The shipped lenses, each published as one changeset by the CLI, which slices the library down to the patterns one
# lens needs and leaves a version already published with the same text as it stands.
register_lenses() {
    local list="$LENSES" library lens meaning registered=0
    library="$(dirname "$list")/$(jq -r '.text' "$list")"
    echo -e "${BOLD}Registry${NC} — registering the shipped lenses from $(basename "$library")"
    while IFS=$'\t' read -r lens meaning; do
        "$CLI" publish-lens --server "$SERVER" --lens "$lens" --version "$(jq -r '.version' "$list")" \
               --owner "$(jq -r '.owner' "$list")" --status "$(jq -r '.status' "$list")" --meaning "$meaning" \
               --author "$AUTHOR" --message "seed the shipped lens \`$lens\`" "$library" 2>&1 | sed 's/^/    /'
        # A pipe's status is its last command's, so the CLI's is read off it.
        if [ "${PIPESTATUS[0]}" != 0 ]; then
            echo -e "${RED}registering $lens failed${NC}: the CLI's refusal is above. If it names another text at the same version, the database predates a change to $(basename "$library"); to go on, $RESET_HINT." >&2
            exit 1
        fi
        registered=$((registered + 1))
    done < <(jq -r '.lenses | to_entries[] | "\(.key)\t\(.value)"' "$list")
    echo -e "  ${GREEN}✓${NC} $registered lenses"
}

# ---------------------------------------------------------------------------
# The scopes and the site
#
# The catalog's templates go into module/go-catalog and the site's map into walmart/dallas, each a boundary scope
# under a prefix scope; the site's stamps pin their templates at the module's head, since a cross-scope reference
# without a pin is refused.
# ---------------------------------------------------------------------------

create_scope() {
    local path="$1" boundary="$2"
    request POST /scopes --data "$(jq -n --arg p "$path" --argjson b "$boundary" '{path:$p, boundary:$b}')"
    case "$HTTP_STATUS" in
        201) echo -e "  ${GREEN}✓${NC} $path created" ;;
        409) echo -e "  ${DIM}$path already exists${NC}" ;;
        *) echo -e "${RED}create $path failed ($HTTP_STATUS):${NC} $(body)" >&2; exit 1 ;;
    esac
}

scope_head() {
    request GET /scopes
    body | jq -r --arg p "$1" '[.scopes[] | select(.scope == $p) | .head] | first // 0'
}

# Propose the document as one changeset, then sign and apply it; the server derives inside the changeset.
write_scope() {
    local scope_path="$1" document="$2" message="$3" start
    request POST "/scopes/$scope_path/-/import?base=0" --data-binary "@$document"
    [ "$HTTP_STATUS" = "200" ] || { echo -e "${RED}import refused ($HTTP_STATUS):${NC} $(body)" >&2; exit 1; }
    body | jq '.changeset' > "$WORK/changeset.json"
    start=$(now_ms)
    "$CLI" apply --server "$SERVER" --author "$AUTHOR" --message "$message" \
                 "$WORK/changeset.json" > "$WORK/applied.json"
    jq -r --arg ms "$(( $(now_ms) - start ))" '"    changeset \(.seq) applied in \($ms) ms, " +
        (if .generation == null then "nothing generated"
         else "\(.generation.derived) entities derived" end)' "$WORK/applied.json"
}

load_the_module() {
    echo -e "${BOLD}Module${NC} — $MODULE_PATH from $(basename "$CATALOG")"
    "$FOUNDRY_WORLD" flatten "$CATALOG" --out "$WORK/module.json" 2>&1 | sed 's/^/    /'
    write_scope "$MODULE_PATH" "$WORK/module.json" "seed: the GO product catalog as $MODULE_PATH"
}

# The scenario is layered and the server takes one merged document. The catalog's own entity ids say which entries
# moved to the module; every `stamp.template` the site keeps is pinned at the module's head.
load_the_site() {
    local pin="$1"
    echo -e "${BOLD}Site${NC} — $SITE_PATH from walmart-dallas/$(basename "$SCENARIO")"
    "$FOUNDRY_WORLD" flatten "$SCENARIO" --out "$WORK/merged.json" 2>&1 | sed 's/^/    /'
    jq --argjson ids "$(jq -c '[.entities[].id]' "$CATALOG")" --arg pin "@$pin" '
        .entities |= (
            map(select(.id as $i | $ids | index($i) | not))
            | map(if (.components.stamp.template? | type) == "string"
                  then .components.stamp.template += $pin
                  else . end)
        )' "$WORK/merged.json" > "$WORK/site.json"
    write_scope "$SITE_PATH" "$WORK/site.json" "seed: walmart-dallas as $SITE_PATH"
}

# ---------------------------------------------------------------------------

[ -x "$CLI" ] || { echo -e "${RED}no world-graph-cli at $CLI${NC}: build it in $WORLD_PROTO" >&2; exit 1; }
request GET /healthz
[ "$HTTP_STATUS" = "200" ] || { echo -e "${RED}$SERVER does not answer /healthz ($HTTP_STATUS)${NC}" >&2; exit 1; }

echo -e "${DIM}building foundry-world${NC}"
(cd "$FOUNDRY" && unset CARGO_TARGET_DIR && cargo build --quiet -p foundry-world-tools --bin foundry-world)
FOUNDRY_WORLD="$FOUNDRY/target/debug/foundry-world"

publish_generators
define_core_vocabulary
define_vocabulary_folder "$APPLICATION/mapcreator/components" "the Map Creator vocabulary"
define_vocabulary_folder "$APPLICATION/components" "the site-template, flow and successor vocabulary"
register_lenses

echo -e "${BOLD}Scopes${NC}"
create_scope module false
create_scope "$TENANT" false
create_scope "$MODULE_PATH" true
create_scope "$SITE_PATH" true

SITE_HEAD=$(scope_head "$SITE_PATH")
if [ "${SITE_HEAD:-0}" != "0" ]; then
    echo -e "${YELLOW}$SITE_PATH already stands at changeset $SITE_HEAD${NC}, leaving the map as it is"
else
    MODULE_HEAD=$(scope_head "$MODULE_PATH")
    if [ "$MODULE_HEAD" = "0" ]; then
        load_the_module
        MODULE_HEAD=$(scope_head "$MODULE_PATH")
    fi
    load_the_site "$MODULE_HEAD"
fi
echo -e "${GREEN}Seeded.${NC}"
