#!/bin/sh
# usage: fetch.sh -> $DL/enron.tar.gz checked, extracted into $RAW/maildir
#
# Downloads the CMU Enron release of 2015-05-07 into its own directory (DL,
# default $B/dl), checks its sha256, then unpacks it into RAW (default
# $B/enron-raw) unless already there. The archive is untrusted data: it is
# only unpacked, never run from.
set -eu
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
URL=https://www.cs.cmu.edu/~enron/enron_mail_20150507.tar.gz
# The sha256 of the archive. The check compares as many leading digits as
# this holds, so a shortened value still works as a prefix check.
SHA256=b3da1b3fe0369ec3140bb4fbce94702c33b7da810ec15d718b3fadf5cd748ca7
DL=${DL:-$B/dl} RAW=${RAW:-$B/enron-raw}
TGZ=$DL/enron.tar.gz
mkdir -p "$DL"
check() {
  sum=$(sha256sum "$1" | cut -d' ' -f1)
  [ "$(printf '%s' "$sum" | cut -c1-${#SHA256})" = "$SHA256" ] && echo "$sum"
}
if [ -f "$TGZ" ] && check "$TGZ" > /dev/null; then
  echo "kept $TGZ"
else
  curl -fL --retry 3 -o "$TGZ.part" "$URL"
  if ! check "$TGZ.part" > /dev/null; then
    echo "sha256 mismatch: $(sha256sum "$TGZ.part" | cut -d' ' -f1), expected $SHA256" >&2
    exit 1
  fi
  mv "$TGZ.part" "$TGZ"
fi
echo "sha256 $(check "$TGZ")"
if [ -d "$RAW/maildir" ]; then
  echo "kept $RAW/maildir"
elif [ -e "$RAW" ]; then
  echo "$RAW exists without maildir/, left as it is" >&2
  exit 1
else
  rm -rf "$RAW.part"
  mkdir -p "$RAW.part"
  tar -xzf "$TGZ" -C "$RAW.part" --no-same-owner --no-same-permissions
  mv "$RAW.part" "$RAW"
  echo "unpacked $(find "$RAW/maildir" -type f | wc -l) files into $RAW/maildir"
fi
