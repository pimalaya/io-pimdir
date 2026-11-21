# Sourced by the bench scripts: the paths they share, each one overridable
# from the environment, and the measure helper.

B=${B:-/home/soywod/code/pimalaya/.bench-notmuch}
HERE=${HERE:-$(cd "$(dirname "$0")" && pwd)}
CPU=${CPU:-12}

NOTMUCH=${NOTMUCH:-$B/tools/bin/notmuch}
GNUTIME=${GNUTIME:-$B/tools-1/bin/time}
HYPERFINE=${HYPERFINE:-$B/tools-2/bin/hyperfine}
EATMYDATA=${EATMYDATA:-$B/tools-eat/bin/eatmydata}
PIMBENCH=${PIMBENCH:-$B/target/release/pimbench}
PIMDIR=${PIMDIR:-$B/target-cli/release/pimdir}
JQ=${JQ:-$B/tools-jq-bin/bin/jq}

# measure LOG LABEL CMD...: appends "== LABEL", the command's output, an
# "io ..." line (the bytes of /proc/PID/io of a shell that reaped the
# command, so its children's count) and GNU time's line to LOG.
measure() {
  _log=$1 _label=$2
  shift 2
  echo "== $_label" >> "$_log"
  "$GNUTIME" -a -o "$_log" -f 'wall=%e,user=%U,sys=%S,rss_kib=%M' \
    sh -c '
      log=$1; shift
      "$@" >> "$log" 2>&1; s=$?
      io=
      while read -r k v; do
        case $k in
          write_bytes:|cancelled_write_bytes:|wchar:|read_bytes:)
            io="$io ${k%:}=$v" ;;
        esac
      done < /proc/$$/io
      echo "io$io" >> "$log"
      exit $s
    ' sh "$_log" "$@"
}
