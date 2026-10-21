#!/bin/sh
# How often one change compiles breadboard's own crates, and how long each
# step takes (#451). Runs in a scratch worktree of its own (detached, in
# $BENCH_DIR or $TMPDIR, removed at the end; its target/ takes a few GB, more
# than a tmpfs quota may allow): a fresh one, as wt:new makes it, its binary
# copied from the store.
#
#   sh builds.sh <flow> [<rev>]   rev: what the worktree checks out (default HEAD)
#
# A flow is two iterations of one change (cold: the worktree's first, warm:
# the next), each step timed, with the units cargo compiled in it: the
# release profile's breadboard and breadboard-core (`release`), and the test
# and clippy units (`debug`). A unit counts when its fingerprint's
# invoked.timestamp is newer than the step's start.
#
#   by-hand    the agent before #451: edit, cargo build --release (cargo.sh),
#              the gauntlet against it, test:rs, commit, wt:check
#   one-build  after #451: edit, the gauntlet (it builds the tree's binary),
#              test:rs, commit, wt:check
#   check      edit, commit, wt:check: what wt:check alone compiles
#
# The gauntlet runs one file ($GAUNTLET_FILE, default batch.check.mjs): its
# build is what is measured, not the suite. Output: one line per step, and
# the table at the end (tab-separated, $BENCH_OUT when set).
set -eu

flow=${1-}
rev=${2-HEAD}
case $flow in
  by-hand | one-build | check) ;;
  *)
    echo "usage: sh builds.sh by-hand|one-build|check [<rev>]" >&2
    exit 2
    ;;
esac

case $0 in
  */*) here=${0%/*} ;;
  *) here=. ;;
esac
repo=$(git -C "$here" rev-parse --show-toplevel)
scratch=$(mktemp -d "${BENCH_DIR:-${TMPDIR:-/tmp}}/breadboard-bench.XXXXXX")
wt="$scratch/wt"
table="$scratch/table.tsv"
file=${GAUNTLET_FILE-batch.check.mjs}

cleanup() {
  git -C "$repo" worktree remove --force "$wt" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT INT TERM

echo "▸ scratch worktree of $(git -C "$repo" rev-parse --short "$rev") in $wt" >&2
git -C "$repo" worktree add --quiet --detach "$wt" "$rev"
(cd "$wt" && pnpm install --offline --frozen-lockfile --silent >/dev/null 2>&1) ||
  (cd "$wt" && pnpm install --frozen-lockfile --silent >/dev/null)
package="$wt/packages/breadboard"
# As wt:new leaves it: the binary of HEAD, from the store when it has one.
(cd "$package" && sh herdr/breadboard.sh --built) >/dev/null 2>&1

# The units compiled since the marker $1, in the profile directory $2.
units() {
  find "$package/target/$2/.fingerprint" -maxdepth 2 -name invoked.timestamp -newer "$1" 2>/dev/null |
    sed -n 's#.*/\(breadboard[a-z-]*\)-[0-9a-f]*/invoked.timestamp#\1#p' | sort | uniq -c |
    awk '{ printf "%s%s×%s", sep, $1, $2; sep = " " }'
}

iteration=
# One step: its name, then the command (run in the worktree).
step() {
  name=$1
  shift
  marker="$scratch/marker"
  touch "$marker"
  sleep 1
  start=$(date +%s.%N)
  status=0
  (cd "$wt" && "$@") >"$scratch/last.log" 2>&1 || status=$?
  end=$(date +%s.%N)
  seconds=$(awk "BEGIN { print $end - $start }")
  release=$(units "$marker" release)
  debug=$(units "$marker" debug)
  printf '%s\t%s\t%s\t%.0f\t%s\t%s\t%s\n' "$flow" "$iteration" "$name" "$seconds" "${release:--}" "${debug:--}" "$status" >>"$table"
  printf '  %-6s %-22s %5.0f s  release: %-34s debug: %s%s\n' "$iteration" "$name" "$seconds" "${release:--}" "${debug:--}" \
    "$([ "$status" = 0 ] || echo "  (exit $status, $scratch/last.log)")" >&2
  [ "$status" = 0 ] || tail -n 20 "$scratch/last.log" >&2
}

# A change to both crates, the size of a typical one doesn't matter: cargo
# compiles a crate whole.
edit() {
  printf '// bench %s\n' "$(date +%s%N)" >>packages/breadboard/crates/core/src/lib.rs
  printf '// bench %s\n' "$(date +%s%N)" >>packages/breadboard/crates/breadboard/src/main.rs
}
commit() {
  git add packages/breadboard/crates && git commit --quiet -m "chore(breadboard): a bench change"
}
gauntlet() {
  pnpm --filter @monocircuit/breadboard-gauntlet gauntlet "src/checks/$file"
}
by_hand_build() {
  sh packages/breadboard/herdr/cargo.sh build --release
}
test_rs() {
  pnpm --filter @monocircuit/breadboard test:rs
}
check() {
  pnpm wt:check
}

printf 'flow\titeration\tstep\tseconds\trelease units\tdebug units\texit\n' >"$table"
for iteration in cold warm; do
  step edit edit
  case $flow in
    by-hand)
      step "cargo build --release" by_hand_build
      step gauntlet gauntlet
      step test:rs test_rs
      ;;
    one-build)
      step gauntlet gauntlet
      step test:rs test_rs
      ;;
  esac
  step commit commit
  step wt:check check
done

[ -z "${BENCH_OUT-}" ] || cp "$table" "$BENCH_OUT"
cat "$table"
