#!/bin/sh
# Gzip the setup page into dist/ for include_bytes! in
# granite-fw/src/http.rs. Run this after editing any asset; the result is
# committed, so a firmware build never depends on gzip being installed or
# on a build script running in the right order.
#
#   ./build.sh          gzip into dist/ and print the sizes
#   ./build.sh --check  fail when dist/ is stale (for CI)
set -eu

here=$(cd "$(dirname "$0")" && pwd)
dist="$here/dist"
assets="index.html app.js style.css"
mkdir -p "$dist"

check=0
[ "${1:-}" = "--check" ] && check=1

total=0
total_gz=0
stale=0
for f in $assets; do
    src="$here/$f"
    out="$dist/$f.gz"
    # -n: no timestamp or name in the header, so the output is reproducible.
    gzip -9 -n -c "$src" > "$out.tmp"
    if [ "$check" = 1 ] && { [ ! -f "$out" ] || ! cmp -s "$out" "$out.tmp"; }; then
        echo "stale: $f.gz" >&2
        stale=1
    fi
    mv "$out.tmp" "$out"
    raw=$(wc -c < "$src")
    gz=$(wc -c < "$out")
    total=$((total + raw))
    total_gz=$((total_gz + gz))
    printf '%-12s %7d -> %6d\n' "$f" "$raw" "$gz"
done
printf '%-12s %7d -> %6d\n' total "$total" "$total_gz"

# ADR 0001: the assets stay under 100 KB uncompressed.
if [ "$total" -gt 102400 ]; then
    echo "assets exceed the 100 KB budget of ADR 0001" >&2
    exit 1
fi
exit "$stale"
