#!/bin/sh
# The breadboard of this checkout for everything that starts it on its own:
# the plugin's manifest (herdr runs `sh breadboard.sh herdr …` in this
# directory, as it ran `node run.mjs …`), the tab bar's status, the hooks'
# shims, pnpm herdr's reload (#314). The binary is built first when it is
# missing or older than the checkout's HEAD (a pull, by a reload or by hand):
# cargo build --release in the package, its progress on stderr.
#
#   sh breadboard.sh <args>            build when needed, then run it
#   sh breadboard.sh --no-wait <args>  never wait for a build (the tab bar's
#                                      status, 3 s): it starts in the
#                                      background, the binary from before runs
#   sh breadboard.sh --built           only build when needed: exit 0 when the
#                                      binary is that of HEAD
#
# "Older than HEAD": target/release/breadboard.source holds what HEAD had in
# crates/, hooks/ and the workspace's Cargo files when it was built here.
# Only sh, git and cat until a build (no dirname: herdr's PATH may be short).
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

# cargo build --release of the package; on success the stamp of what it built.
build() {
  if [ ! -f "$package/Cargo.toml" ]; then
    echo "✗ $binary isn't built, and $package has no Cargo.toml to build it from" >&2
    return 1
  fi
  if ! has_cargo; then
    echo "✗ $binary is missing or older than the checkout, and there is no cargo to build it (https://rustup.rs, then pnpm build:rs in $package)" >&2
    return 1
  fi
  now=$(source_now)
  echo "▸ breadboard is built from $(git -C "$package" rev-parse --short HEAD 2>/dev/null) (cargo build --release in $package)" >&2
  if ! cargo build --release --manifest-path "$package/Cargo.toml" </dev/null 1>&2; then
    mkdir -p "${binary%/*}"
    printf '%s\n' "$now" >"$failed"
    echo "✗ cargo build --release failed in $package" >&2
    return 1
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
