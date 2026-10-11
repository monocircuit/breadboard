#!/bin/sh
# The breadboard of this checkout for everything that starts it: the root
# package.json's scripts (pnpm overview, wt:*, stack:*, …), the plugin's
# manifest (herdr runs `sh breadboard.sh herdr …` in this directory), the tab
# bar's status, the hooks' shims, nvim's changes tab, pnpm herdr's reload
# (#314). The binary is made first when it is missing or older than the
# checkout's HEAD (a pull, by a reload or by hand), its progress on stderr.
#
#   sh breadboard.sh <args>            build when needed, then run it
#   sh breadboard.sh --no-wait <args>  never wait for a build (the tab bar's
#                                      status, 3 s): it starts in the
#                                      background, the binary from before runs
#   sh breadboard.sh --built           only build when needed: exit 0 when the
#                                      binary is that of HEAD
#
# "Older than HEAD": target/release/breadboard.source holds what HEAD had in
# crates/, hooks/ and the workspace's Cargo files when it was made here.
# Only sh, git and cat until a build (no dirname: herdr's PATH may be short).
#
# Made, not always built: the repo's store, <git common dir>/breadboard, is
# shared by every worktree of the clone. bin/<key>/breadboard keeps each
# build by the key of its sources, so a worktree with sources built before
# (wt:new's, a pull another checkout made) copies it, ~15 MB; target/ is
# cargo's for all of them, so a build only compiles what changed.
#
# A stored copy is what its key says (#339): the build gets the key
# (BREADBOARD_SOURCE_KEY) and the binary carries it; a copy is stored only
# when the sources were committed and unchanged across the build and the
# file carries the key, and used only when it carries it. A key read before
# the store's lock, or a target/ another build wrote, once put other sources'
# binaries under a key: the overview ran code from before the fix.
case $0 in
  */*) here=${0%/*} ;;
  *) here=. ;;
esac
package=$(CDPATH='' cd -- "$here/.." && pwd -P) || exit 1
binary="$package/target/release/breadboard"
stamp="$binary.source"
failed="$binary.failed"
mode=run
case "${1-}" in
  --no-wait) mode=no-wait && shift ;;
  --built) mode=built && shift ;;
esac

# What HEAD has of the sources the binary is built from (ls-tree: one line each).
source_now() {
  git -C "$package" ls-tree HEAD -- crates hooks Cargo.toml Cargo.lock 2>/dev/null
}

# The stamp of a binary made from the sources $1. Its first line says it
# was made by a launcher that checks keys (#339): a stamp from before is no
# stamp, so every checkout makes its binary once more, from a checked copy.
stamp_of() {
  printf '#339\n%s\n' "$1"
}

# Whether the binary is there and built from HEAD's sources. Without sources
# (no Cargo.toml: a package that ships only the binary) or outside a checkout
# the binary that is there is the one.
is_fresh() {
  [ -x "$binary" ] || return 1
  [ -f "$package/Cargo.toml" ] || return 0
  now=$(source_now)
  [ -n "$now" ] || return 0
  [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$(stamp_of "$now")" ]
}

# The store's key of the sources $1 (source_now's lines).
key_of() {
  printf '%s\n' "$1" | git hash-object --stdin
}

# Whether the file $1 is a build of the key $2: the key the build put in it,
# found without running it (an old binary may not know the question).
has_key() {
  [ -n "$2" ] && grep -aqF "breadboard-source-key:$2" "$1" 2>/dev/null
}

# Whether the sources have no uncommitted changes.
is_clean() {
  [ -z "$(git -C "$package" status --porcelain -- crates hooks Cargo.toml Cargo.lock 2>/dev/null)" ]
}

# cargo on PATH, else rustup's: a herdr started from a desktop has no ~/.cargo/bin.
has_cargo() {
  command -v cargo >/dev/null 2>&1 && return 0
  PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
  command -v cargo >/dev/null 2>&1
}

# The repo's store, or nothing outside a git checkout.
store() {
  common=$(git -C "$package" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || return 1
  [ -n "$common" ] && printf '%s/breadboard\n' "$common"
}

# The file $1 as this checkout's binary: copied beside it, then renamed over
# it, so a breadboard that runs keeps the file it started from.
install() {
  mkdir -p "${binary%/*}" && cp "$1" "$binary.$$" && chmod +x "$binary.$$" && mv -f "$binary.$$" "$binary"
}

# The store's build lock, waited for: one cargo build of the repo at a time,
# so the binary a build leaves in target/ is the one it built.
lock_store() {
  lock="$shared/build.lock"
  is_said=
  while ! mkdir "$lock" 2>/dev/null; do
    pid=$(cat "$lock/pid" 2>/dev/null)
    if [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null; then
      rm -rf "$lock"
      continue
    fi
    [ -n "$is_said" ] || echo "▸ another checkout builds breadboard, this one waits for it" >&2
    is_said=1
    sleep 1
  done
  echo "$$" >"$lock/pid"
  trap 'rm -rf "$lock"; exit 130' INT TERM HUP
}

unlock_store() {
  rm -rf "$lock"
  trap - INT TERM HUP
}

# The store keeps the newest 8 builds; a checkout keeps its own copy.
prune() {
  ls -t "$shared/bin" 2>/dev/null | tail -n +9 | while IFS= read -r old; do
    rm -rf "${shared:?}/bin/$old"
  done
}

# What cargo left in this checkout's own target/ before every run used the
# store's (#336: 1.9 GB per worktree): removed once, the binary, its stamps
# and the build log stay. Not while a cargo holds it (its .cargo-lock), and
# not without flock to tell; outside a clone there is no store to use instead.
reclaim() {
  own="$package/target"
  [ -d "$own/debug" ] || [ -d "$own/release/deps" ] || return 0
  store >/dev/null || return 0
  command -v flock >/dev/null 2>&1 || return 0
  for held in "$own"/*/.cargo-lock; do
    [ -f "$held" ] || continue
    # sh, not true: herdr's PATH may have no true.
    flock -n "$held" sh -c : 2>/dev/null || return 0
  done
  rm -rf "$own/debug" "$own/release/deps" "$own/release/build" "$own/release/.fingerprint" \
    "$own/release/incremental" "$own/release/examples" "$own/release/.cargo-lock" \
    "$own/CACHEDIR.TAG" "$own/.rustc_info.json"
  rm -f "$own"/release/*.d
  echo "▸ removed this checkout's own cargo target/ (#336): every cargo run uses the store's" >&2
}

# The store's copy of the sources $1 when it has one that carries their key:
# installed and stamped. One that doesn't carry it (stored before #339, or
# poisoned) is removed.
from_store() {
  [ -n "$shared" ] && [ -n "$1" ] && is_clean || return 1
  key=$(key_of "$1")
  kept="$shared/bin/$key/breadboard"
  [ -e "$kept" ] || return 1
  if [ -x "$kept" ] && has_key "$kept" "$key" && install "$kept"; then
    rm -f "$failed"
    stamp_of "$1" >"$stamp"
    return 0
  fi
  rm -rf "${kept%/*}"
  return 1
}

# The binary of HEAD's sources: the store's when it has one, else cargo
# build --release in the store's target/; on success the stamp of what it is.
build() {
  shared=$(store) || shared=
  from_store "$(source_now)" && return 0
  if [ ! -f "$package/Cargo.toml" ]; then
    echo "✗ $binary isn't built, and $package has no Cargo.toml to build it from" >&2
    return 1
  fi
  if ! has_cargo; then
    echo "✗ $binary is missing or older than the checkout, and there is no cargo to build it (https://rustup.rs, then pnpm build:rs in $package)" >&2
    return 1
  fi
  if [ -z "$shared" ]; then
    built_in "$package/target"
    return $?
  fi
  mkdir -p "$shared" && lock_store
  built_in "$shared/target"
  made=$?
  unlock_store
  return $made
}

# The build itself, in the target/ $1; the store's lock held when there is a
# store. Every build in the store's target/ (#336), one of uncommitted
# changes too: cargo compiles only what differs, never the dependencies again.
built_in() {
  # What is built is what is there now, not before the wait for the lock.
  now=$(source_now)
  from_store "$now" && return 0
  key=
  is_clean && [ -n "$now" ] && key=$(key_of "$now")
  echo "▸ breadboard is built from $(git -C "$package" rev-parse --short HEAD 2>/dev/null) (cargo build --release in $1)" >&2
  if ! BREADBOARD_SOURCE_KEY="$key" CARGO_TARGET_DIR="$1" sh "$package/herdr/cargo.sh" build --release --manifest-path "$package/Cargo.toml" </dev/null 1>&2; then
    mkdir -p "${binary%/*}"
    printf '%s\n' "$now" >"$failed"
    echo "✗ cargo build --release failed in $package" >&2
    return 1
  fi
  made="$1/release/breadboard"
  if [ -n "$key" ] && [ -n "$shared" ]; then
    # A build of committed sources is these sources' or nothing: its key in
    # the file, the sources unchanged across it. Only then is it stored.
    if ! has_key "$made" "$key"; then
      echo "✗ the binary in $1 isn't a build of $(git -C "$package" rev-parse --short HEAD 2>/dev/null)'s sources (another build wrote it?): not used, not stored" >&2
      return 1
    fi
    if [ "$(source_now)" = "$now" ] && is_clean; then
      kept="$shared/bin/$key/breadboard"
      mkdir -p "${kept%/*}" && cp "$made" "$kept.$$" && mv -f "$kept.$$" "$kept" && prune
    fi
  fi
  install "$made" || return 1
  rm -f "$failed"
  [ -n "$now" ] && stamp_of "$now" >"$stamp"
  return 0
}

# A build in the background, one at a time (the tab bar asks every 10 s):
# 0 when one runs, 1 when there is nothing to build with, 2 when HEAD's
# sources failed already (a key or a reload tries those again, in front).
build_behind() {
  [ -f "$package/Cargo.toml" ] && has_cargo || return 1
  [ -f "$failed" ] && [ "$(cat "$failed")" = "$(source_now)" ] && return 2
  lock="$binary.building"
  mkdir -p "${binary%/*}"
  if ! mkdir "$lock" 2>/dev/null; then
    pid=$(cat "$lock/pid" 2>/dev/null)
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && return 0
    rm -rf "$lock" && mkdir "$lock" 2>/dev/null || return 0
  fi
  # Its own session where there is setsid: herdr's timeout of the status doesn't end it.
  detach=
  command -v setsid >/dev/null 2>&1 && detach=setsid
  $detach sh -c 'sh "$1" --built; rm -rf "$2"' sh "$package/herdr/breadboard.sh" "$lock" \
    >"$package/target/breadboard-build.log" 2>&1 </dev/null &
  echo "$!" >"$lock/pid" 2>/dev/null
  return 0
}

case $mode in
  built)
    reclaim
    is_fresh && exit 0
    build && [ -x "$binary" ]
    exit $?
    ;;
  no-wait)
    if ! is_fresh; then
      build_behind
      behind=$?
      if [ ! -x "$binary" ]; then
        case $behind in
          0) echo "breadboard builds…" ;;
          1) echo "breadboard isn't built: pnpm build:rs" ;;
          *) echo "breadboard didn't build: pnpm build:rs" ;;
        esac
        exit 0
      fi
    fi
    ;;
  run)
    reclaim
    if ! is_fresh && ! build; then
      if [ ! -x "$binary" ]; then
        echo "✗ breadboard can't run until it is built: pnpm build:rs in $package" >&2
        exit 1
      fi
      echo "! the breadboard from before runs ($binary)" >&2
    fi
    ;;
esac
exec "$binary" "$@"
