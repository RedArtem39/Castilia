#!/bin/sh
# Per-operation timings (min of 3 runs): micro.sh MODE FILES...
mode=$1; shift
for f in "$@"; do
  best=999999
  for k in 1 2 3; do
    s=$(date +%s%N); ./target/release/examples/jsrun.exe --mode $mode $f >/dev/null 2>&1; e=$(date +%s%N)
    t=$(( (e-s)/1000000 )); [ $t -lt $best ] && best=$t
  done
  echo "$(basename $f .js) $mode ${best}ms"
done
