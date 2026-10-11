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

# One run at a time in the shared target/ (#339), held for the whole run:
# the check below and the build it guards stay one step.
if [ -z "${BREADBOARD_CARGO_HELD-}" ] && command -v flock >/dev/null 2>&1; then
  mkdir -p "$CARGO_TARGET_DIR"
  BREADBOARD_CARGO_HELD=1 exec flock "$CARGO_TARGET_DIR/.breadboard.lock" sh "$package/herdr/cargo.sh" "$@"
fi

# cargo's hash of a workspace crate leaves out where the workspace is, and it
# judges freshness by mtimes: after another worktree's build, this one's
# older files count as built, and the binary, clippy's verdict and the tests
# are the other worktree's (#339). So a checkout that takes over the target/
# from another cleans the workspace's own crates first, the dependencies stay.
# Another: the record names another checkout, or a cargo that doesn't keep
# the record (a branch's older cargo.sh, a plain cargo) built after this
# one's last run: a fingerprint newer than the record.
from="$CARGO_TARGET_DIR/.breadboard-from"
is_mine() {
  [ "$(cat "$from" 2>/dev/null)" = "$package" ] || return 1
  for prints in "$CARGO_TARGET_DIR"/*/.fingerprint; do
    [ -d "$prints" ] || continue
    [ -z "$(find "$prints" -maxdepth 1 -newer "$from" -print 2>/dev/null | head -n 1)" ] || return 1
  done
  return 0
}
if [ "${1-}" != fmt ] && ! is_mine; then
  members=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$package"/crates/*/Cargo.toml 2>/dev/null)
  if [ -n "$members" ]; then
    specs=
    for member in $members; do specs="$specs -p $member"; done
    # shellcheck disable=SC2086 # one word per -p and name
    cargo clean --quiet $specs >/dev/null 2>&1
    # shellcheck disable=SC2086
    cargo clean --quiet --release $specs >/dev/null 2>&1
  fi
  mkdir -p "$CARGO_TARGET_DIR" && printf '%s\n' "$package" >"$from"
fi

niced=
command -v nice >/dev/null 2>&1 && niced="nice -n 10"
[ "${1-}" = fmt ] && exec $niced cargo "$@"
$niced cargo "$@"
ran=$?
# Written after the run: what it built is older than the record, what
# another cargo builds next is newer.
printf '%s\n' "$package" >"$from"
exit $ran
