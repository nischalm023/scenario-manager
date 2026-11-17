#!/bin/sh
# The source a checkout holds, as Agent Studio's local runner image records it in its agent-studio/source label and as
# dev.sh and the API compare it, so that a local run uses a runner built from the code the API runs: the commit, and
# when the working tree differs from it, a hash of the difference, every tracked file's change and every untracked file
# git does not ignore, after a `+`. An uncommitted edit names another source, and reverting it names the commit's again.
# Run from inside the checkout; prints the one line.
set -eu
commit=$(git rev-parse HEAD)
changes=$(
    {
        git diff HEAD --binary --no-ext-diff --no-textconv --no-renames --no-color
        git ls-files --others --exclude-standard
        git ls-files --others --exclude-standard | git hash-object --stdin-paths
    } | git hash-object --stdin
)
if [ -z "$(git status --porcelain --untracked-files=normal)" ]; then
    echo "$commit"
else
    echo "$commit+$(echo "$changes" | cut -c1-16)"
fi
