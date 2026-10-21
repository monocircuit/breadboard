#!/bin/sh
# Read-only: GitLab's push events and merge requests since <after> into <dir>,
# the data/ measure.py reads (its --out's data/). glab of this checkout's repo.
#
#   sh fetch_gitlab.sh <dir> [<after>]   after: YYYY-MM-DD (default 2026-10-15)
set -eu
dir=${1:?usage: sh fetch_gitlab.sh <dir> [<after>]}
after=${2-2026-10-15}
mkdir -p "$dir"
case $0 in
  */*) here=${0%/*} ;;
  *) here=. ;;
esac
cd "$(git -C "$here" rev-parse --show-toplevel)"
count() { python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))))" "$1"; }
for page in 1 2 3 4 5 6 7 8 9 10; do
  glab api "projects/:id/events?action=pushed&after=$after&per_page=100&page=$page" >"$dir/events_$page.json"
  [ "$(count "$dir/events_$page.json")" -lt 100 ] && break
done
for page in 1 2 3 4 5; do
  glab api "projects/:id/merge_requests?updated_after=${after}T00:00:00Z&state=all&per_page=100&page=$page" >"$dir/mrs_$page.json"
  [ "$(count "$dir/mrs_$page.json")" -lt 100 ] && break
done
ls -la "$dir"
