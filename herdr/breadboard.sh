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
# its sources (`sources` below) when it was made here.
# Only sh, git and cat until a build, date and find while it waits for one
# (no dirname: herdr's PATH may be short).
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

# The sources the binary is built from: the crates and what they embed
# (include_str!: the hooks' shims, the skill's templates, the herdr plugin
# init copies, this launcher, the nvim Lua, the Chromium extension, #342).
sources="crates hooks claude herdr/herdr-plugin.toml herdr/keys.toml herdr/config.toml herdr/breadboard.sh nvim chromium Cargo.toml Cargo.lock"

# What HEAD has of the sources the binary is built from (ls-tree: one line each).
source_now() {
  git -C "$package" ls-tree HEAD -- $sources 2>/dev/null
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
  [ -z "$(git -C "$package" status --porcelain -- $sources 2>/dev/null)" ]
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
# so the binary a build leaves in target/ is the one it built. The lock says
# who holds it (pid, checkout, since, boot, what it does: #389), so a wait
# shows who builds and for how long, and one nobody holds is cleared: no pid
# (a kill between mkdir and its pid), a pid that is no breadboard.sh or
# cargo (reused after a reboot), or a lock of another boot. A build without
# progress (nothing new in the store's target/) for BREADBOARD_LOCK_STALL
# seconds (15 min) is offered to be cleared on a terminal, else said how.
lock_store() {
  lock="$shared/build.lock"
  stall=${BREADBOARD_LOCK_STALL:-900}
  boot=$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)
  mark="$shared/build.watch.$$"
  progressed=$(now_s)
  is_said=
  empty=0
  ticks=0
  while ! mkdir "$lock" 2>/dev/null; do
    if [ -n "$boot" ] && [ -f "$lock/boot" ] && [ "$(cat "$lock/boot" 2>/dev/null)" != "$boot" ]; then
      clear_lock "it is of another boot"
      continue
    fi
    pid=$(cat "$lock/pid" 2>/dev/null)
    if [ -z "$pid" ]; then
      # mkdir and its pid are two steps: empty for 3 s, it is nobody's.
      empty=$((empty + 1))
      if [ "$empty" -ge 3 ]; then
        clear_lock "it names no builder"
        continue
      fi
    elif ! is_builder "$pid"; then
      clear_lock "its builder ($pid) is gone"
      continue
    else
      empty=0
    fi
    [ -f "$mark" ] || : >"$mark"
    if [ -n "$(find "$shared/target" -maxdepth 3 -newer "$mark" -print 2>/dev/null | head -n 1)" ]; then
      : >"$mark"
      progressed=$(now_s)
    fi
    now=$(now_s)
    still=
    [ -n "$now" ] && [ -n "$progressed" ] && still=$((now - progressed))
    if [ -t 2 ]; then
      [ $((ticks % 2)) -eq 0 ] && printf '\r\033[K▸ another checkout builds breadboard, this one waits for it: %s' "$(holder "$still")" >&2
    elif [ $((ticks % 60)) -eq 0 ]; then
      echo "▸ another checkout builds breadboard, this one waits for it: $(holder "$still")" >&2
    fi
    is_said=1
    if [ -n "$still" ] && [ "$still" -ge "$stall" ]; then
      if offer_clear "$(minutes "$still")"; then
        is_said=
        clear_lock "as asked"
        continue
      fi
      progressed=$(now_s)
    fi
    ticks=$((ticks + 1))
    sleep 1
  done
  rm -f "$mark"
  [ -n "$is_said" ] && [ -t 2 ] && echo >&2
  echo "$$" >"$lock/pid"
  printf '%s\n' "$package" >"$lock/checkout"
  now_s >"$lock/since"
  [ -n "$boot" ] && printf '%s\n' "$boot" >"$lock/boot"
  trap 'rm -rf "$lock" "$mark"; exit 130' INT TERM HUP
}

# Seconds since the epoch, or nothing without date (herdr's PATH may be short).
now_s() {
  date +%s 2>/dev/null
}

# Whether the pid $1 runs and is a build: breadboard.sh or cargo where /proc
# tells, any process elsewhere.
is_builder() {
  kill -0 "$1" 2>/dev/null || return 1
  [ -r "/proc/$1/cmdline" ] || return 0
  grep -aqE 'breadboard\.sh|cargo' "/proc/$1/cmdline" 2>/dev/null
}

# The lock removed, and why ($1).
clear_lock() {
  rm -rf "$lock"
  [ -n "$is_said" ] && [ -t 2 ] && echo >&2
  echo "▸ the store's build lock was cleared: $1" >&2
  is_said=
  empty=0
  ticks=0
}

# Who holds the lock: `checkout <name>, 3:12 min, compiling breadboard from
# abc1234`, and how long nothing new came to target/ ($1, seconds) past a minute.
holder() {
  who=$(cat "$lock/checkout" 2>/dev/null)
  who=${who%/packages/breadboard}
  said="checkout ${who##*/}"
  [ -n "$who" ] || said="pid $(cat "$lock/pid" 2>/dev/null)"
  since=$(cat "$lock/since" 2>/dev/null)
  now=$(now_s)
  if [ -n "$since" ] && [ -n "$now" ]; then
    took=$((now - since))
    said="$said, $((took / 60)):$(printf '%02d' $((took % 60))) min"
  fi
  what=$(cat "$lock/what" 2>/dev/null)
  said="$said, ${what:-starting}"
  [ -n "$1" ] && [ "$1" -ge 60 ] && said="$said, nothing compiled for $(minutes "$1")"
  printf '%s' "$said"
}

# $1 seconds in words: `4 min`, under one `40 s`.
minutes() {
  if [ "$1" -ge 60 ]; then
    printf '%s min' $(($1 / 60))
  else
    printf '%s s' "$1"
  fi
}

# A build without progress for $1 (minutes()): on a terminal the question whether
# to clear its lock (0: yes), else the line that says how; the wait goes on.
offer_clear() {
  [ -t 2 ] && echo >&2
  if [ -t 2 ] && [ -r /dev/tty ]; then
    printf '▸ its build made no progress for %s (nothing new in %s): clear its lock and build here? [y/N] ' "$1" "$shared/target" >&2
    read -r answer </dev/tty || answer=
    case $answer in
      y* | Y* | j* | J*) return 0 ;;
    esac
    return 1
  fi
  echo "! its build made no progress for $1 (nothing new in $shared/target): if nothing builds, rm -rf $lock clears it" >&2
  return 1
}

unlock_store() {
  rm -rf "$lock" "$mark"
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
  short=$(git -C "$package" rev-parse --short HEAD 2>/dev/null)
  [ -n "$shared" ] && [ -d "$lock" ] && echo "compiling breadboard from $short" >"$lock/what"
  echo "▸ breadboard is built from $short (cargo build --release in $1)" >&2
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
