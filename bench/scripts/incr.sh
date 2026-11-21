#!/bin/sh
# usage: incr.sh RUN_NAME MAIL_ROOT [CPU] -> appends to runs/NAME/incr.log
#
# Incremental work on the stores tier.sh left: a sync that finds nothing,
# 1,000 new messages in a new folder (hardlinked from copy 2, so distinct
# Message-IDs), 1,000 flag changes (files renamed), then tagging 11,411
# messages. MAIL_ROOT is changed in place: rebuild it with corpus.sh after.
set -e
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
R=$B/runs/$1 ROOT=$2 CPU=${3:-$CPU}
C2=${C2:-$B/corpus/c2}
export NOTMUCH_CONFIG="$R/nm.conf" HOME="$R" TZ=UTC
PD="taskset -c $CPU $PIMDIR -s $R/pd/store"
L=$R/incr.log
both() {
  sync
  measure "$L" "$1 notmuch new" taskset -c "$CPU" "$NOTMUCH" new --quiet
  sync
  measure "$L" "$1 pimdir sync" taskset -c "$CPU" \
    "$PIMBENCH" sync "$R/pd/store" "$ROOT" "$R/pd/state"
  measure "$L" "$1 pimdir index" taskset -c "$CPU" \
    "$PIMBENCH" index "$R/pd/store"
}
both noop
mkdir -p "$ROOT/zz-new/inbox/cur" "$ROOT/zz-new/inbox/new" "$ROOT/zz-new/inbox/tmp"
find "$C2/dasovich-j~2/all_documents/cur" -type f | head -1000 | while read -r f; do
  ln "$f" "$ROOT/zz-new/inbox/cur/"
done
both add1000
D=$ROOT/dasovich-j/all_documents/cur
ls "$D" | grep -v F | head -1000 | while read -r f; do
  mv "$D/$f" "$D/${f%%:2,*}:2,F${f##*:2,}"
done
both flag1000
measure "$L" "tag notmuch +bench from:jeff.dasovich" \
  taskset -c "$CPU" "$NOTMUCH" tag +bench -- from:jeff.dasovich@enron.com
measure "$L" "tag pimdir --add bench from:jeff.dasovich" \
  sh -c 'exec "$@" > /dev/null' sh \
  $PD tag --refresh-timeout 0s --add bench from:jeff.dasovich@enron.com
echo "pd tagged: $($PD count --refresh-timeout 5m tag:bench | head -1)" >> "$L"
echo "nm tagged: $("$NOTMUCH" count tag:bench)" >> "$L"
echo done >> "$L"
