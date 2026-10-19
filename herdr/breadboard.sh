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
#   sh breadboard.sh --tree            the same for the sources as they are on
#                                      disk, uncommitted changes too (pnpm
#                                      build:rs, the gauntlet, #451)
#
# "Older than HEAD": target/release/breadboard.source holds the sources
# (`sources` below) the binary was made from, as the working tree had them,
# and the key of HEAD's when it was made here. A binary made from the tree
# stays HEAD's until HEAD's sources move: the uncommitted changes it was
# built with are the checkout's own, and once they are committed as they
# were, it is HEAD's binary without a second build (#451).
# Only sh, git and cat until a build (no dirname: herdr's PATH may be short).
#
# Made, not always built: the repo's store, <git common dir>/breadboard, is
# shared by every worktree of the clone. bin/<key>/breadboard keeps each
# build by the key of its sources, so a worktree with sources built before
# (wt:new's, a pull another checkout made) copies it, ~15 MB. A build runs
# in the checkout's own target/, never waiting for another's (#446); the
# dependencies come from sccache's cache where it is installed.
#
# A stored copy is what its key says (#339): the build gets the key
# (BREADBOARD_SOURCE_KEY) and the binary carries it; a copy is stored only
# when the sources were committed and unchanged across the build and the
# file carries the key, and used only when it carries it. The key is that of
# the sources as built, uncommitted ones too (#451): the build of a change
# before its commit carries the key the commit has, so the commit needs no
# build of its own (a build with another key compiles the crate again). A key read before
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
  --tree) mode=tree && shift ;;
esac

# The sources the binary is built from: the crates and what they embed
# (include_str!: the hooks' shims, the skill's templates, the herdr plugin
# init copies, this launcher, the nvim Lua, the Chromium extension, #342).
sources="crates hooks claude herdr/herdr-plugin.toml herdr/keys.toml herdr/config.toml herdr/breadboard.sh nvim chromium Cargo.toml Cargo.lock"

# What HEAD has of the sources the binary is built from (ls-tree: one line each).
source_now() {
  git -C "$package" ls-tree HEAD -- $sources 2>/dev/null
}

# What the working tree has of them, in source_now's lines (#451): the
# checkout's index copied, the package's files added to it (untracked ones
# too, not the ignored), its tree listed. The same lines as source_now's
# once the sources are committed as they are.
source_tree() {
  index=$(git -C "$package" rev-parse --path-format=absolute --git-path index 2>/dev/null) || return 1
  mkdir -p "${binary%/*}" || return 1
  copy="$binary.index.$$"
  if [ -f "$index" ]; then
    cp "$index" "$copy" || return 1
  else
    GIT_INDEX_FILE=$copy git -C "$package" read-tree HEAD 2>/dev/null || return 1
  fi
  tree=$(GIT_INDEX_FILE=$copy git -C "$package" add -A -- . 2>/dev/null &&
    GIT_INDEX_FILE=$copy git -C "$package" write-tree 2>/dev/null)
  rm -f "$copy"
  [ -n "$tree" ] && git -C "$package" ls-tree "$tree" -- $sources 2>/dev/null
}

# The stamp of a binary made from the sources $1 (source_tree's lines) on
# HEAD's sources $2. Its first line says it was made by a launcher that
# checks keys (#339): a stamp from before is no stamp, so every checkout
# makes its binary once more, from a checked copy.
stamp_of() {
  printf '#339\n%s\n#on %s\n' "$1" "$(key_of "$2")"
}

# The stamp's parts: `made`, the sources the binary was made from, and `on`,
# the key of HEAD's sources then (none in a stamp before #451). 1 without
# a stamp of this launcher.
read_stamp() {
  [ -f "$stamp" ] || return 1
  made=$(cat "$stamp")
  case $made in
    "#339
"*) made=${made#"#339
"} ;;
    *) return 1 ;;
  esac
  on=
  case $made in
    *"
#on "*)
      on=${made##*"
#on "}
      made=${made%"
#on "*}
      ;;
  esac
  return 0
}

# Whether the binary is there and HEAD's: built from HEAD's sources, or from
# the tree on top of them while HEAD's sources haven't moved since. Without
# sources (no Cargo.toml: a package that ships only the binary) or outside a
# checkout the binary that is there is the one.
is_fresh() {
  [ -x "$binary" ] || return 1
  [ -f "$package/Cargo.toml" ] || return 0
  now=$(source_now)
  [ -n "$now" ] || return 0
  read_stamp || return 1
  [ "$made" = "$now" ] && return 0
  [ -n "$on" ] && [ "$on" = "$(key_of "$now")" ]
}

# Whether the binary is there and built from the sources on disk (--tree).
is_fresh_tree() {
  [ -x "$binary" ] || return 1
  [ -f "$package/Cargo.toml" ] || return 0
  tree_now=$(source_tree)
  [ -n "$tree_now" ] || return 0
  read_stamp && [ "$made" = "$tree_now" ]
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

# The binary as the store's copy of the sources $1 under the key $2, when
# they are HEAD's: what a commit has, other checkouts get. Its lines go to
# $binary.kept, so a launch after that asks the store no more.
keep() {
  [ -n "$shared" ] && [ -n "$2" ] && [ "$1" = "$(source_now)" ] || return 0
  kept="$shared/bin/$2/breadboard"
  [ -e "$kept" ] || {
    has_key "$binary" "$2" && mkdir -p "${kept%/*}" && cp "$binary" "$kept.$$" && mv -f "$kept.$$" "$kept" && prune
  } || return 0
  printf '%s\n' "$1" >"$binary.kept"
}

# A binary built from the tree before its commit (#451): HEAD's now, and
# kept in the store for the others once, as the build would have.
keep_fresh() {
  [ -f "$package/Cargo.toml" ] && [ -n "${now-}" ] && [ "${made-}" = "$now" ] || return 0
  [ "$(cat "$binary.kept" 2>/dev/null)" = "$now" ] && return 0
  shared=$(store) || return 0
  keep "$now" "$(key_of "$now")"
}

# The store keeps the newest 8 builds; a checkout keeps its own copy.
prune() {
  ls -t "$shared/bin" 2>/dev/null | tail -n +9 | while IFS= read -r old; do
    rm -rf "${shared:?}/bin/$old"
  done
}

# The store's copy of the sources $1 when it has one that carries their key:
# installed and stamped. One that doesn't carry it (stored before #339, or
# poisoned) is removed.
from_store() {
  [ -n "$shared" ] && [ -n "$1" ] || return 1
  key=$(key_of "$1")
  kept="$shared/bin/$key/breadboard"
  [ -e "$kept" ] || return 1
  if [ -x "$kept" ] && has_key "$kept" "$key" && install "$kept"; then
    rm -f "$failed"
    stamp_of "$1" "$2" >"$stamp"
    printf '%s\n' "$1" >"$binary.kept"
    return 0
  fi
  rm -rf "${kept%/*}"
  return 1
}

# The binary of the sources on disk: the store's when it has one, else cargo
# build --release in this checkout's own target/ (#446: no queue behind
# another worktree's build; sccache, where installed, gives every target/ the
# dependencies compiled once); on success the stamp of what it is.
build() {
  shared=$(store) || shared=
  # What is built is what is there now, under its own key (#451).
  head=$(source_now)
  now=$(source_tree) || now=
  [ -n "$now" ] || now=$head
  from_store "$now" "$head" && return 0
  if [ ! -f "$package/Cargo.toml" ]; then
    echo "✗ $binary isn't built, and $package has no Cargo.toml to build it from" >&2
    return 1
  fi
  if ! has_cargo; then
    echo "✗ $binary is missing or older than the checkout, and there is no cargo to build it (https://rustup.rs, then pnpm build:rs in $package)" >&2
    return 1
  fi
  key=
  [ -n "$now" ] && key=$(key_of "$now")
  short=$(git -C "$package" rev-parse --short HEAD 2>/dev/null)
  [ "$now" = "$head" ] || short="$short and its uncommitted changes"
  echo "▸ breadboard is built from $short (cargo build --release in $package/target)" >&2
  if ! BREADBOARD_SOURCE_KEY="$key" CARGO_TARGET_DIR="$package/target" sh "$package/herdr/cargo.sh" build --release --manifest-path "$package/Cargo.toml" </dev/null 1>&2; then
    mkdir -p "${binary%/*}"
    printf '%s\n' "$head" >"$failed"
    echo "✗ cargo build --release failed in $package" >&2
    return 1
  fi
  if [ -n "$key" ]; then
    # A build is these sources' or nothing: its key in the file. Stored only
    # when they are committed and unchanged across it.
    if ! has_key "$binary" "$key"; then
      echo "✗ the binary in $package/target isn't a build of $short's sources (another build wrote it?): not used, not stored" >&2
      return 1
    fi
    rm -f "$binary.kept"
    [ "$(source_tree)" = "$now" ] && keep "$now" "$key"
  fi
  rm -f "$failed"
  [ -n "$now" ] && stamp_of "$now" "$head" >"$stamp"
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
    if is_fresh; then
      keep_fresh
      exit 0
    fi
    build && [ -x "$binary" ]
    exit $?
    ;;
  tree)
    is_fresh_tree && exit 0
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
    if is_fresh; then
      keep_fresh
    elif ! build; then
      if [ ! -x "$binary" ]; then
        echo "✗ breadboard can't run until it is built: pnpm build:rs in $package" >&2
        exit 1
      fi
      echo "! the breadboard from before runs ($binary)" >&2
    fi
    ;;
esac
exec "$binary" "$@"
