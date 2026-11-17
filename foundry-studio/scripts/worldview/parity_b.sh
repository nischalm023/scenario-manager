#!/bin/zsh
# Harness B: the branch's desvis draws master's picture, array for array.
#
# LEGACY NAMES LIVE HERE ON PURPOSE. This script and master-dump-render.patch
# drive master's binary over master's own recordings, so they must say
# `--profile` and `.desprofile`; the branch's "no legacy spelling left" grep
# gate excludes these two files by name for that reason, and nothing else in
# the tree may name them.
#
# Harness A proves the numbers in the workbook. This proves the pixels: at a
# fixed list of simulated instants both binaries are asked what they would
# upload — the bot instances, the tote instances, the per-cell rows, the relay
# reservation flags, the slice's heat overlay, the traffic overlay, and the
# run-total heat at the run's end — and every byte is compared. Master's desvis
# learns the mode from master-dump-render.patch, applied to the pinned master
# worktree's DETACHED HEAD (no branch is created, moved or deleted), built into
# its own target directory so the pristine binary harness A runs is never
# overwritten, and reverted before this script exits however it exits. The
# hand-back is verified and decides the exit status.
#
# The instants are every slice start for the first ten slices, then every fifth
# boundary, then the run's last instant — the end frame's own stamp, which is
# where the run stopped rather than where its last slice began.
#
# usage: parity_b.sh [<results-dir> [<branch-runs-dir> [<master-runs-dir>]]]
#   <results-dir>       RESULTS.md and both sides' manifests. Default: the
#                       round-4 folder of the worldview notes.
#   <branch-runs-dir>   harness A's branch runs (run1's .worldview per fixture).
#   <master-runs-dir>   harness A's master runs (run1's recording per fixture).
# MASTER_CHECKOUT=<dir> is the pinned master worktree; NAMES=... picks
# fixtures; KEEP_DUMPS=1 leaves the arrays on disk instead of deleting them
# after the comparison.
set -u

here=${0:A:h}
repo=${here:h:h}
results=${1:-~/vt/greyorange/notes/worldview-cut1-2026-09-17/harness/round4}
branch_runs=${2:-$repo/target/worldview-harness/branch}
master_runs=${3:-$repo/target/worldview-harness/master}
master_checkout=${MASTER_CHECKOUT:-~/vt/greyorange/worldview-master}
names=${NAMES:-minimal_putaway minimal_easyloop_flow cencora dfw6}
pin=fd63aadab22dda6562553e809f5595d94a83ce3e
patch=$here/master-dump-render.patch
added=apps/desvis/src/app/render_dump.rs
master_target=$repo/target/master-dump
master_desvis=$master_target/release/desvis
dumps=$repo/target/worldview-harness/dumps

for binary in $repo/target/release/desvis $repo/target/release/worldview; do
  [[ -x $binary ]] || { print -u2 "parity_b: no binary at $binary (cargo build --release -p desvis; -p worldview --features cli)"; exit 2 }
done

# The pinned worktree is borrowed, so it is only borrowed from a clean detached
# state and it is handed back in one.
[[ -z $(git -C $master_checkout status --porcelain) ]] || {
  print -u2 "parity_b: $master_checkout is not clean; refusing to touch it"; exit 2 }
[[ $(git -C $master_checkout rev-parse HEAD) == $pin ]] || {
  print -u2 "parity_b: $master_checkout is not at $pin"; exit 2 }
[[ $(git -C $master_checkout rev-parse --abbrev-ref HEAD) == HEAD ]] || {
  print -u2 "parity_b: $master_checkout is on a branch; it must be detached at $pin"; exit 2 }

# Undo the patch and say whether the worktree really came back. Idempotent, so
# the interrupt trap and the normal path can both call it.
cleaned=no
hand_back=1
restore() {
  [[ $cleaned == yes ]] && return $hand_back
  cleaned=yes
  git -C $master_checkout apply -R $patch >/dev/null 2>&1
  rm -f $master_checkout/$added
  local left head ref
  left=$(git -C $master_checkout status --porcelain 2>&1)
  head=$(git -C $master_checkout rev-parse HEAD 2>&1)
  ref=$(git -C $master_checkout rev-parse --abbrev-ref HEAD 2>&1)
  if [[ -z $left && $head == $pin && $ref == HEAD ]]; then
    hand_back=0
  else
    hand_back=1
    print -u2 "parity_b: $master_checkout did NOT come back: HEAD $head on $ref, left behind:"
    print -u2 $left
  fi
  return $hand_back
}
trap 'restore; exit 4' INT TERM

verdicts=()
measure() {
  git -C $master_checkout apply $patch || { print -u2 "parity_b: the patch did not apply"; return 3 }
  print "building master's --dump-render into $master_target"
  ( cd $master_checkout && CARGO_TARGET_DIR=$master_target cargo build --release -p desvis )
  local built=$?
  (( built == 0 )) || { print -u2 "parity_b: master's build failed ($built)"; return 3 }
  [[ -x $master_desvis ]] || { print -u2 "parity_b: no binary at $master_desvis"; return 3 }

  mkdir -p $results
  local failed=0
  local name ticks report outcome line same
  for name in ${=names}; do
    local branch_rec=($branch_runs/$name/run1/*.worldview(N))
    local master_rec=($master_runs/$name/run1/*.desprofile(N))
    if (( ${#branch_rec} != 1 || ${#master_rec} != 1 )); then
      print -u2 "parity_b: $name has ${#branch_rec} branch and ${#master_rec} master recordings under run1 — run parity_a.sh first"
      verdicts+=("| $name | — | — | ERROR: ${#branch_rec} branch and ${#master_rec} master recordings under run1 |")
      failed=1
      continue
    fi
    # Every slice start for the first ten slices, then every fifth boundary,
    # then the end frame's stamp, which is the instant the run stopped at.
    ticks=$($repo/target/release/worldview inspect $branch_rec[1] | awk '
      $3 == "delta" { t[n++] = $2 }
      $3 == "end"   { ended = $2 }
      END {
        if (n < 1 || ended == "") { exit 1 }
        for (i = 0; i < 10 && i < n; i++) pick[k++] = t[i]
        for (i = 10; i < n; i += 5) pick[k++] = t[i]
        # A run that stopped exactly on a boundary states that instant twice;
        # the dump refuses a repeated instant, so the list holds each once.
        pick[k++] = ended
        for (i = 0; i < k; i++) {
          if (!(pick[i] in seen)) { seen[pick[i]] = 1; out = out (out == "" ? "" : ",") pick[i] }
        }
        print out
      }')
    if (( $? != 0 )); then
      print -u2 "parity_b: $name: the recording states no deltas or no end frame"
      verdicts+=("| $name | — | — | ERROR: no delta stamps or no end frame in $branch_rec[1] |")
      failed=1
      continue
    fi

    rm -rf $dumps/$name
    if ! $repo/target/release/desvis --recording $branch_rec[1] --dump-render $ticks $dumps/$name/branch > /dev/null; then
      print -u2 "parity_b: $name: the branch's dump failed"
      verdicts+=("| $name | — | — | ERROR: the branch's --dump-render failed |")
      failed=1
      continue
    fi
    if ! $master_desvis --profile $master_rec[1] --dump-render $ticks $dumps/$name/master > /dev/null; then
      print -u2 "parity_b: $name: master's dump failed"
      verdicts+=("| $name | — | — | ERROR: master's --dump-render failed |")
      failed=1
      continue
    fi

    report=$(python3 $here/dump_diff.py $dumps/$name/master $dumps/$name/branch 2>&1)
    outcome=$?
    local lines=(${(f)report})
    # Every line reads "t=<instant> <array>: <verdict>"; anything else is the
    # comparer refusing the pair, and rides as its own row.
    for line in $lines; do
      if [[ $line == t=*\ *:\ * ]]; then
        verdicts+=("| $name | ${${line%% *}#t=} | ${${line#* }%%:*} | ${line#*: } |")
      else
        verdicts+=("| $name | — | — | $line |")
      fi
    done
    same=$(print -l $lines | grep -c ': identical$')
    print "$name: $same of ${#lines} arrays identical over ${#${(s:,:)ticks}} instants"
    if (( outcome != 0 )); then
      failed=1
      print $report
    fi
    mkdir -p $results/$name
    cp $dumps/$name/branch/manifest.json $results/$name/branch-manifest.json
    cp $dumps/$name/master/manifest.json $results/$name/master-manifest.json
    [[ -n ${KEEP_DUMPS:-} ]] || rm -rf $dumps/$name
  done
  return $failed
}

measure
work=$?
restore
handed=$?

mkdir -p $results
{
  print "# Harness B — the render arrays, master's desvis against the branch's"
  print ""
  print "Written by \`scripts/worldview/parity_b.sh\` on $(date -u +%Y-%m-%dT%H:%M:%SZ)."
  print "Master: $master_runs (binary built from master-dump-render.patch at $pin)."
  print "The branch: $branch_runs."
  print "$(print -l $verdicts | grep -c '| identical |') of ${#verdicts} rows identical; the pinned worktree handed back $( (( handed == 0 )) && print clean || print DIRTY )."
  print ""
  print "| fixture | instant (ms) | array | verdict |"
  print "|---|---|---|---|"
  print -l $verdicts
} > $results/RESULTS.md
print "wrote $results/RESULTS.md"

(( work == 0 )) || exit $work
(( handed == 0 )) || exit 4
exit 0
