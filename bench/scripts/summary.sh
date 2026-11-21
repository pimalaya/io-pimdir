#!/bin/sh
# usage: summary.sh RUN_NAME... > summary.md
#
# One markdown section per run: the import steps of runs/NAME/log, the
# query table of runs/NAME/queries.txt and the incremental steps of
# runs/NAME/incr.log, whichever exist.
set -eu
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"

steps() {
  awk -v permsg="$2" '
    function mib(b) { return b == "" ? "-" : sprintf("%.0f", b / 1048576) }
    function pairs(s, sep,   n, i, kv, a) {
      delete f
      n = split(s, a, sep)
      for (i = 1; i <= n; i++) {
        if (split(a[i], kv, "=") == 2) f[kv[1]] = kv[2]
      }
    }
    function flush() {
      if (label == "") return
      cpu = (user == "") ? "" : user + sys
      line = sprintf("| %s | %s | %s | %s | %s | %s | %s |", label,
        wall == "" ? "-" : wall, cpu == "" ? "-" : sprintf("%.1f", cpu),
        rss == "" ? "-" : sprintf("%.0f", rss / 1024), mib(wb), mib(wc),
        pwall == "" ? "-" : sprintf("%.1f / %.1f", pwall, pcpu))
      if (permsg == 1) {
        line = line sprintf(" %s | %s |",
          (cpu == "" || files == 0) ? "-" : sprintf("%.3f", cpu * 1000 / files),
          (wb == "" || files == 0) ? "-" : sprintf("%.1f", wb / 1024 / files))
      }
      print line
      label = ""
    }
    BEGIN {
      head = "| step | wall s | CPU s | peak RSS MiB | written MiB | wchar MiB | in-process wall / CPU s |"
      rule = "| --- | --- | --- | --- | --- | --- | --- |"
      if (permsg == 1) {
        head = head " CPU ms / msg | written KiB / msg |"
        rule = rule " --- | --- |"
      }
      print head
      print rule
    }
    /^files / { files = $2 }
    /^== / {
      flush()
      label = substr($0, 4)
      wall = user = sys = rss = wb = wc = pwall = pcpu = ""
      next
    }
    /^wall=/ {
      pairs($0, ",")
      wall = f["wall"]; user = f["user"]; sys = f["sys"]; rss = f["rss_kib"]
    }
    /^io / { pairs($0, " "); wb = f["write_bytes"]; wc = f["wchar"] }
    /^phase=/ {
      pairs($0, " ")
      pwall = f["wall_s"]; pcpu = f["user_s"] + f["sys_s"]
    }
    /^(nm_|pd_)/ { size[$1] = $2 }
    END {
      flush()
      out = ""
      for (k in size) out = out sprintf("%s%s %.2f GB", out == "" ? "" : ", ", k, size[k] / 1e9)
      if (out != "") { print ""; print "On disk: " out "." }
      if (files > 0) { print ""; print "Files: " files "." }
    }
  ' "$1"
}

queries() {
  awk -F '\t' '
    function worst(a, b, c, d, e, g,   r) {
      r = 0
      if (a > 0 && d / a > r) r = d / a
      if (b > 0 && e / b > r) r = e / b
      if (c > 0 && g / c > r) r = g / c
      return r
    }
    NR == 1 {
      print "| query | hits nm / pd | notmuch count / page / threads ms | pimdir count / page / threads ms | worst pd / nm |"
      print "| --- | --- | --- | --- | --- |"
      next
    }
    {
      printf "| %s | %s / %s | %s / %s / %s | %s / %s / %s | %.1f |\n",
        $1, $2, $6, $3, $4, $5, $7, $8, $9, worst($3, $4, $5, $7, $8, $9)
    }
  ' "$1"
}

for name in "$@"; do
  R=$B/runs/$name
  echo "## $name"
  if [ -f "$R/log" ]; then
    sed -n 's/^\(io-pimdir\|notmuch\|hyperfine\|kernel\|cpu\) /- &/p' "$R/log"
    echo
    echo "### Import"
    echo
    steps "$R/log" 1
    echo
  fi
  if [ -f "$R/queries.txt" ]; then
    echo "### Queries"
    echo
    queries "$R/queries.txt"
    echo
  fi
  if [ -f "$R/incr.log" ]; then
    echo "### Incremental"
    echo
    steps "$R/incr.log" 0
    echo
  fi
done
