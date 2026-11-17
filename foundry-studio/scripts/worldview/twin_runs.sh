#!/bin/zsh
# One side of harness A: run one desvis twice on each gate fixture and write a
# workbook for every run.
#
# Twice, because a cell that differs between two workbooks means nothing until
# the same binary run twice is known to produce the same one. The pair proves
# the side is deterministic; parity_a.sh then compares the sides.
#
# usage: twin_runs.sh <results-dir> <desvis> [<workbook>]
#   <desvis>    the binary to run; its own checkout supplies the fixtures
#               (override with CHECKOUT=...).
#   <workbook>  given, the run's xlsx comes from this command reading the
#               `.worldview` the run wrote; absent, from desvis's own --xlsx.
# NAMES=... picks fixtures; the default is all four.
set -u

results=${1:-}
desvis=${2:-}
workbook=${3:-}
if [[ -z $results || -z $desvis ]]; then
  print -u2 "usage: twin_runs.sh <results-dir> <desvis> [<workbook>]"
  exit 2
fi
here=${0:A:h}
checkout=${CHECKOUT:-${here:h:h}}
cd $checkout || exit 99

typeset -A FX
FX=(minimal_putaway sim/dessim/tests/fixtures/minimal_putaway_spec.json
    minimal_easyloop_flow sim/dessim/tests/fixtures/minimal_easyloop_flow_spec.json
    cencora sim/dessim/tests/fixtures/cencora_v3_spec.json
    dfw6 sim/dessim/tests/fixtures/walmart_dfw6_smoke_spec.json)

mkdir -p $results
log=$results/twins.log
: >> $log
failed=0
# zsh splits an unquoted parameter only with ${=...}; spelled out, a list of
# names would arrive as one argument.
for name in ${=NAMES:-minimal_putaway minimal_easyloop_flow cencora dfw6}; do
  for run in 1 2; do
    out=$results/$name/run$run
    rm -rf $out
    mkdir -p $out
    start=$(date +%s)
    if [[ -n $workbook ]]; then
      $desvis ${FX[$name]} --headless --threads 1 --simtime 10m --allow-incomplete --name "$name-twin" \
         --output-dir $out > $out/stdout.log 2> $out/stderr.log
    else
      $desvis ${FX[$name]} --headless --threads 1 --simtime 10m --allow-incomplete --name "$name-twin" \
         --output-dir $out --xlsx $out/$name.xlsx > $out/stdout.log 2> $out/stderr.log
    fi
    ran=$?
    wrote=0
    if [[ -n $workbook ]]; then
      recording=($out/*.worldview(N))
      if (( ${#recording} == 1 )); then
        $workbook $recording[1] --out $out/$name.xlsx >> $out/stdout.log 2>> $out/stderr.log
        wrote=$?
      else
        print -u2 "twin_runs: $name run$run wrote ${#recording} recordings, expected 1"
        wrote=1
      fi
    fi
    print "$name run$run desvis=$ran workbook=$wrote wall=$(( $(date +%s) - start ))s" >> $log
    (( ran == 0 && wrote == 0 )) || failed=1
  done
done
exit $failed
