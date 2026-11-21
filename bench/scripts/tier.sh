#!/bin/sh
# usage: tier.sh NAME MAIL_ROOT [CPU] -> runs/NAME/{nm,pd}, log in runs/NAME/log
#
# The import of one tier: notmuch new, then pimbench sync and index, each
# pinned to CPU (default 12), with a sync before each. Environment:
#   TOOLS="notmuch pimdir"  the tools to run, either or both
#   EAT=1                   run them under eatmydata (fsync disabled), the
#                           split between waits and work; the stores are
#                           removed after unless KEEP=1
set -e
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
NAME=$1 ROOT=$2 CPU=${3:-$CPU}
TOOLS=${TOOLS:-notmuch pimdir}
WRAP=
[ "${EAT:-0}" = 1 ] && WRAP=$EATMYDATA
R=$B/runs/$NAME
rm -rf "$R"
mkdir -p "$R/nm" "$R/pd"
chmod 700 "$R/pd"
"$HERE/nmconf.sh" "$ROOT" "$R/nm" "$R/nm.conf"
export NOTMUCH_CONFIG="$R/nm.conf" HOME="$R"
L=$R/log
: > "$L"
[ -f "$B/versions" ] && cat "$B/versions" >> "$L"
echo "cpu $CPU eat ${EAT:-0}" >> "$L"
echo "files $(find "$ROOT" -type f | wc -l)" >> "$L"
for tool in $TOOLS; do
  case $tool in
    notmuch)
      sync
      measure "$L" "notmuch new" taskset -c "$CPU" $WRAP "$NOTMUCH" new --quiet
      echo "nm_db $(du -sb "$R/nm" | cut -f1)" >> "$L"
      ;;
    pimdir)
      sync
      measure "$L" "pimdir sync" taskset -c "$CPU" $WRAP \
        "$PIMBENCH" sync "$R/pd/store" "$ROOT" "$R/pd/state"
      sync
      measure "$L" "pimdir index" taskset -c "$CPU" $WRAP \
        "$PIMBENCH" index "$R/pd/store"
      for f in pimdir.db index.db objects; do
        echo "pd_$f $(du -sb "$R/pd/store/$f" | cut -f1)" >> "$L"
      done
      ls -la "$R/pd/store" >> "$L"
      ;;
    *)
      echo "unknown tool $tool" >&2
      exit 2
      ;;
  esac
done
if [ "${EAT:-0}" = 1 ] && [ "${KEEP:-0}" != 1 ]; then
  rm -rf "$R/nm" "$R/pd"
fi
echo done >> "$L"
