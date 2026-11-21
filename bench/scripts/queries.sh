#!/bin/sh
# usage: queries.sh RUN_NAME [CPU] -> runs/NAME/queries.txt
#
# Each line of queries.tsv (label, notmuch query, pimdir query) through
# both CLIs on the stores tier.sh left: hits, then hyperfine medians in ms
# (RUNS runs, default 10, after 2 warm-ups) of a count, a page of 50 newest
# first and a threaded page of 50, process start included.
set -e
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
R=$B/runs/$1 CPU=${2:-$CPU}
export NOTMUCH_CONFIG="$R/nm.conf" HOME="$R" TZ=UTC
PD="$PIMDIR -s $R/pd/store"
OUT=$R/queries.txt
: > "$OUT"
med() {
  "$HYPERFINE" -N --warmup 2 --runs "${RUNS:-10}" --export-json "$R/h.json" \
    "taskset -c $CPU $1" > /dev/null 2>&1
  "$JQ" -r '.results[0].median*1000|.*10|round/10' "$R/h.json"
}
printf 'label\tnm_count\tnm_count_ms\tnm_page_ms\tnm_threads_ms\tpd_count\tpd_count_ms\tpd_page_ms\tpd_threads_ms\n' >> "$OUT"
while IFS='	' read -r label nq pq; do
  nc=$("$NOTMUCH" count "$nq")
  pc=$($PD count --refresh-timeout 0s --json "$pq" | "$JQ" -r '.count')
  a=$(med "$NOTMUCH count '$nq'")
  b=$(med "$NOTMUCH search --output=messages --sort=newest-first --limit=50 '$nq'")
  c=$(med "$NOTMUCH search --sort=newest-first --limit=50 '$nq'")
  d=$(med "$PD count --refresh-timeout 0s '$pq'")
  e=$(med "$PD search --refresh-timeout 0s -n 50 '$pq'")
  f=$(med "$PD search --refresh-timeout 0s --threads -n 50 '$pq'")
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$label" "$nc" "$a" "$b" "$c" "$pc" "$d" "$e" "$f" >> "$OUT"
done < "$HERE/queries.tsv"
rm -f "$R/h.json"
