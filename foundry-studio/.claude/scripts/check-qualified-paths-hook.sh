#!/bin/sh
# PostToolUse hook (Edit|Write): when the edited file is Rust, run the
# qualified-paths checker on it and, on violations, exit 2 so the hit list on
# stderr is fed straight back to the model in the same turn. Companion to the
# .githooks/pre-commit gate — this one catches the path before it ever reaches
# a commit. Receives the hook JSON on stdin.

f=$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input", {}).get("file_path") or "")' 2>/dev/null) || exit 0
case "$f" in
    *.rs) ;;
    *) exit 0 ;;
esac

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null || echo .)}" || exit 0
[ -f scripts/find-qualified-paths.py ] || exit 0
[ -f "$f" ] || exit 0

python3 scripts/find-qualified-paths.py --check "$f" || exit 2
exit 0
