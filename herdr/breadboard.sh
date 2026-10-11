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

# Whether the binary is there and built from HEAD's sources. Without sources
# (no Cargo.toml: a package that ships only the binary) or outside a checkout
# the binary that is there is the one.
is_fresh() {
  [ -x "$binary" ] || return 1
  [ -f "$package/Cargo.toml" ] || return 0
  now=$(source_now)
  [ -n "$now" ] || return 0
  [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$now" ]
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

# The binary of HEAD's sources: the store's when it has one, else cargo
# build --release in the store's target/; on success the stamp of what it is.
build() {
  now=$(source_now)
  shared=$(store) || shared=
  kept=
  # Uncommitted changes to the sources are built but not kept: the store
  # keeps only what HEAD has, the key names it.
  if [ -n "$shared" ] && [ -n "$now" ] && [ -z "$(git -C "$package" status --porcelain -- crates hooks Cargo.toml Cargo.lock 2>/dev/null)" ]; then
    kept="$shared/bin/$(printf '%s\n' "$now" | git hash-object --stdin)/breadboard"
    if [ -x "$kept" ] && install "$kept"; then
      rm -f "$failed"
      printf '%s\n' "$now" >"$stamp"
      return 0
    fi
  fi
  if [ ! -f "$package/Cargo.toml" ]; then
    echo "✗ $binary isn't built, and $package has no Cargo.toml to build it from" >&2
    return 1
  fi
  if ! has_cargo; then
    echo "✗ $binary is missing or older than the checkout, and there is no cargo to build it (https://rustup.rs, then pnpm build:rs in $package)" >&2
    return 1
  fi
  # Every build in the store's target/ (#336), one of uncommitted changes
  # too: cargo compiles only what differs, never the dependencies again. The
  # store's lock: the binary copied out is the one this build made.
  target="$package/target"
  if [ -n "$shared" ]; then
    target="$shared/target"
    mkdir -p "$shared" && lock_store
    # Another checkout may have built these sources while this one waited.
    if [ -n "$kept" ] && [ -x "$kept" ] && install "$kept"; then
      unlock_store
      rm -f "$failed"
      printf '%s\n' "$now" >"$stamp"
      return 0
    fi
  fi
  echo "▸ breadboard is built from $(git -C "$package" rev-parse --short HEAD 2>/dev/null) (cargo build --release in $target)" >&2
  if ! CARGO_TARGET_DIR="$target" sh "$package/herdr/cargo.sh" build --release --manifest-path "$package/Cargo.toml" </dev/null 1>&2; then
    [ -n "$shared" ] && unlock_store
    mkdir -p "${binary%/*}"
    printf '%s\n' "$now" >"$failed"
    echo "✗ cargo build --release failed in $package" >&2
    return 1
  fi
  if [ -n "$shared" ]; then
    # Uncommitted sources never enter the store under HEAD's key.
    [ -n "$kept" ] && mkdir -p "${kept%/*}" && cp "$target/release/breadboard" "$kept.$$" && mv -f "$kept.$$" "$kept" && prune
    install "$target/release/breadboard"
    copied=$?
    unlock_store
    [ "$copied" = 0 ] || return 1
  fi
  rm -f "$failed"
  [ -n "$now" ] && printf '%s\n' "$now" >"$stamp"
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
