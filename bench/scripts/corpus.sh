#!/bin/sh
# usage: corpus.sh [TIER...] -> $B/corpus/{c1,c2} and the tiers asked for
#
# Turns the unpacked Enron tree into Maildirs with maildirify.py, once:
# c1 (copy 1, hardlinks of the original files) and c2 (copy 2, Message-IDs
# rewritten). Then rebuilds each tier named (default: x1 x2) as a fresh tree
# of hardlinks, so a run that renames or adds files (incr.sh) leaves the
# copies as they were:
#   x1  copy 1, 517,401 messages
#   x2  copies 1 and 2, 1,034,802 messages
set -eu
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
RAW=${RAW:-$B/enron-raw} C=$B/corpus
[ $# -gt 0 ] || set -- x1 x2
mkdir -p "$C"
for k in 1 2; do
  [ -d "$C/c$k" ] && continue
  [ -d "$RAW/maildir" ] || { echo "no $RAW/maildir: run fetch.sh" >&2; exit 1; }
  rm -rf "$C/c$k.part"
  nix-shell -p python3 --run \
    "python3 -I '$HERE/maildirify.py' '$RAW/maildir' '$C/c$k.part' $k"
  mv "$C/c$k.part" "$C/c$k"
done
for tier in "$@"; do
  rm -rf "$C/$tier"
  mkdir -p "$C/$tier"
  case $tier in
    x1) cp -al "$C/c1/." "$C/$tier/" ;;
    x2) cp -al "$C/c1/." "$C/c2/." "$C/$tier/" ;;
    *) echo "unknown tier $tier" >&2; exit 2 ;;
  esac
  echo "$tier: $(find "$C/$tier" -type f | wc -l) files"
done
