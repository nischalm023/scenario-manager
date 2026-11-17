#!/bin/zsh
# Harness A: the branch's workbook is master's workbook, cell for cell.
#
# The `.worldview` writer, its reader and the `workbook` command sit between a
# run and the spreadsheet everyone reads it in, and none of them may change a
# number. So both sides run the same four gate fixtures the same way — master's
# desvis writes its own xlsx, the branch's desvis writes a recording and the
# `workbook` command writes the xlsx from it — and every cell of every sheet is
# compared, except the provenance cells named in xlsx_cell_diff.py.
#
# Each side runs twice, and all six pairings of the four workbooks are
# compared: either side losing determinism then shows up as itself rather than
# as a parity defect in the branch. Exit 1 on any difference. The verdicts also
# go to RESULTS.md in the results folder.
#
# usage: parity_a.sh [<results-dir> [<master-runs-dir>]]
#   <results-dir>       where the branch's runs go. Default:
#                       target/worldview-harness/branch in this checkout — pass
#                       the notes folder's harness/<round> to keep them.
#   <master-runs-dir>   where master's runs are read from, and written to under
#                       REBUILD_MASTER=1. Default:
#                       target/worldview-harness/master in this checkout.
# REBUILD_MASTER=1 re-runs master's pair instead of reusing what is there;
# MASTER_CHECKOUT=<dir> is the pinned master worktree; NAMES=... picks fixtures.
set -u

here=${0:A:h}
repo=${here:h:h}
results=${1:-$repo/target/worldview-harness/branch}
master_runs=${2:-$repo/target/worldview-harness/master}
# The one home path: the master worktree is pinned at fd63aadab and lives
# outside any checkout of this repository.
master_checkout=${MASTER_CHECKOUT:-~/vt/greyorange/worldview-master}
names=${NAMES:-minimal_putaway minimal_easyloop_flow cencora dfw6}

for binary in $master_checkout/target/release/desvis $repo/target/release/desvis $repo/target/release/workbook; do
  [[ -x $binary ]] || { print -u2 "parity_a: no binary at $binary"; exit 2 }
done

if [[ -n ${REBUILD_MASTER:-} ]]; then
  print "running master's twins into $master_runs"
  NAMES=$names CHECKOUT=$master_checkout $here/twin_runs.sh $master_runs \
    $master_checkout/target/release/desvis || exit 3
fi

# Master's pinned worktree is detached and clean, so its runs record the branch
# as "HEAD" and the code as matching the commit. Neither cell is one the diff's
# name list covers, so the branch's runs state the same two things; commit,
# hostname and timestamp stay excluded by name, master's binary having no way
# to be told them.
print "running the branch's twins into $results"
DESVIS_RUN_BRANCH=HEAD DESVIS_RUN_DIRTY=false NAMES=$names CHECKOUT=$repo \
  $here/twin_runs.sh $results $repo/target/release/desvis $repo/target/release/workbook || exit 3

failed=0
verdicts=()
compare() {  # <fixture> <what> <a.xlsx> <b.xlsx>
  local report
  report=$(python3 $here/xlsx_cell_diff.py $3 $4 2>&1)
  local outcome=$?
  local lines=(${(f)report})
  # The census line, then the verdict line — which is the LAST line only when
  # nothing differed; on a failure the problems follow it.
  local said=$lines[-1]
  (( outcome == 0 )) || said=$lines[2]
  print "$1 $2: $lines[1] — $said"
  if (( outcome != 0 )); then
    print $report
    failed=1
    verdicts+=("| $1 | $2 | $said — first: ${${lines[3]##* }:-?} |")
  else
    verdicts+=("| $1 | $2 | identical |")
  fi
}

for name in ${=names}; do
  compare $name "master determinism" $master_runs/$name/run{1,2}/$name.xlsx
  compare $name "branch determinism" $results/$name/run{1,2}/$name.xlsx
  compare $name "parity run1" $master_runs/$name/run1/$name.xlsx $results/$name/run1/$name.xlsx
  compare $name "parity run2" $master_runs/$name/run2/$name.xlsx $results/$name/run2/$name.xlsx
  compare $name "cross run1-run2" $master_runs/$name/run1/$name.xlsx $results/$name/run2/$name.xlsx
  compare $name "cross run2-run1" $master_runs/$name/run2/$name.xlsx $results/$name/run1/$name.xlsx
done

{
  print "# Harness A — every pairing of the four workbooks per fixture"
  print ""
  print "Written by \`scripts/worldview/parity_a.sh\` on $(date -u +%Y-%m-%dT%H:%M:%SZ)."
  print "Master's runs: $master_runs. The branch's runs: $results."
  print ""
  print "| fixture | comparison | result |"
  print "|---|---|---|"
  print -l $verdicts
} > $results/RESULTS.md
print "wrote $results/RESULTS.md"
exit $failed
