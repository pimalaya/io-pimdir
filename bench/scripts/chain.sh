#!/bin/sh
# usage: chain.sh [CPU] -> runs/{x1-eat,x1,x2}, then summary.md
#
# The whole protocol, one step at a time on one core: fresh tiers, the
# fsync split at 517k, the 517k import, its queries and incremental runs,
# then the 1M import and its queries (5 runs each).
set -e
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
CPU=${1:-$CPU}
C=$B/corpus
"$HERE/corpus.sh" x1 x2
EAT=1 TOOLS=pimdir "$HERE/tier.sh" x1-eat "$C/x1" "$CPU"
"$HERE/tier.sh" x1 "$C/x1" "$CPU"
"$HERE/queries.sh" x1 "$CPU"
"$HERE/incr.sh" x1 "$C/x1" "$CPU"
"$HERE/corpus.sh" x1
"$HERE/tier.sh" x2 "$C/x2" "$CPU"
RUNS=5 "$HERE/queries.sh" x2 "$CPU"
"$HERE/summary.sh" x1-eat x1 x2 > "$B/runs/summary.md"
echo "summary in $B/runs/summary.md"
