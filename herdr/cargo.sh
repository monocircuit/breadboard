#!/bin/sh
# Every cargo run of breadboard (#336): the launcher's build, pnpm lint:rs,
# test:rs and build:rs. One target/ for the whole clone, the store's
# (<git common dir>/breadboard/target, the one breadboard.sh builds in), so
# no worktree compiles the dependencies again in a target/ of its own
# (1.9 GB each, load 54 with three at once). cargo locks it: a second run
# waits for the first instead of compiling beside it. At low priority and on
# half the cores, so a check never freezes the machine the agents work on.
#
#   sh cargo.sh <cargo args>   CARGO_TARGET_DIR and CARGO_BUILD_JOBS set
#                              beforehand win
#
# Only sh and git until cargo (no dirname: herdr's PATH may be short).
case $0 in
  */*) here=${0%/*} ;;
  *) here=. ;;
esac
package=$(CDPATH='' cd -- "$here/.." && pwd -P) || exit 1

if [ -z "${CARGO_TARGET_DIR-}" ]; then
  common=$(git -C "$package" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)
  if [ -n "$common" ]; then
    CARGO_TARGET_DIR="$common/breadboard/target"
  else
    CARGO_TARGET_DIR="$package/target"
  fi
  export CARGO_TARGET_DIR
fi

if [ -z "${CARGO_BUILD_JOBS-}" ]; then
  cores=$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || echo 2)
  jobs=$((cores / 2))
  [ "$jobs" -ge 1 ] || jobs=1
  CARGO_BUILD_JOBS=$jobs
  export CARGO_BUILD_JOBS
fi

# cargo on PATH, else rustup's: a herdr started from a desktop has no ~/.cargo/bin.
command -v cargo >/dev/null 2>&1 || PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

cd "$package" || exit 1
if command -v nice >/dev/null 2>&1; then
  exec nice -n 10 cargo "$@"
fi
exec cargo "$@"
