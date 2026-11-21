#!/bin/sh
# usage: setup.sh [tree|snapshot] -> the tools rooted under $B, pimbench in
# $TARGET (default $B/target), the pimdir CLI in $TARGET_CLI (default
# $B/target-cli), their versions in $B/versions
#
# tree      builds io-pimdir's working tree as it stands (the default)
# snapshot  builds a git archive of io-pimdir HEAD unpacked under
#           $B/snapshot/io-pimdir, the working tree's bench/ copied in when
#           HEAD does not hold it
#
# The tools are GC roots: tools (notmuch), tools-1 (GNU time), tools-2
# (hyperfine), tools-eat (libeatmydata). Builds stay on ext4 under $B,
# never on the shared /tmp tmpfs.
set -eu
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
IO=${IO:-$(cd "$HERE/../.." && pwd)}
MODE=${1:-tree}
TARGET=${TARGET:-$B/target} TARGET_CLI=${TARGET_CLI:-$B/target-cli}
mkdir -p "$B"
nix build nixpkgs#notmuch nixpkgs#time nixpkgs#hyperfine -o "$B/tools"
nix build nixpkgs#libeatmydata -o "$B/tools-eat"
nix build nixpkgs#jq -o "$B/tools-jq"
REV=$(git -C "$IO" rev-parse HEAD)
case $MODE in
  tree)
    SRC=$IO
    [ -z "$(git -C "$IO" status --porcelain)" ] || REV="$REV-dirty"
    ;;
  snapshot)
    SRC=$B/snapshot/io-pimdir
    rm -rf "$SRC"
    mkdir -p "$SRC"
    git -C "$IO" archive HEAD | tar -x -C "$SRC"
    [ -d "$SRC/bench" ] || cp -R "$IO/bench" "$SRC/bench"
    ;;
  *)
    echo "usage: setup.sh [tree|snapshot]" >&2
    exit 2
    ;;
esac
cd "$IO"
CARGO_TARGET_DIR=$TARGET nix develop --command \
  cargo build --release --manifest-path "$SRC/bench/Cargo.toml"
CARGO_TARGET_DIR=$TARGET_CLI nix develop --command \
  cargo build --release --features cli --bin pimdir \
  --manifest-path "$SRC/Cargo.toml"
{
  echo "io-pimdir $REV ($MODE)"
  echo "notmuch $("$NOTMUCH" --version | cut -d' ' -f2)"
  echo "hyperfine $("$HYPERFINE" --version | cut -d' ' -f2)"
  echo "kernel $(uname -r)"
} > "$B/versions"
cat "$B/versions"
