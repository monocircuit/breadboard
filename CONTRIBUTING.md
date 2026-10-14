# Contributing to breadboard

Thank you for helping. One thing to know first: **this repository is a
mirror**. breadboard is developed in a monorepo, in `packages/breadboard`,
and its CI pushes every change merged there to this repository's `main`,
with the same history. Nothing is ever merged here directly, or the next push
of the mirror would be refused.

## Issues

Open them here: bugs, questions, ideas. Say what you ran (`breadboard
--version` or the commit), what happened and what you expected.

## Pull requests

1. Fork, branch from `main`, make the change. Before you push:

   ```sh
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```

2. Open the pull request against `main` with what it changes and why. A
   maintainer reviews it here, as usual.
3. Once it is accepted, the maintainer applies your commits in the monorepo
   (below), keeping you as the author, and closes the pull request with a
   link to the commit. Within minutes of the merge there, the mirror brings
   the same change back to this `main` (a new commit id, since it now carries
   the monorepo's history; your name and message stay).

By submitting a contribution you agree that it is licensed as described in
the [README](README.md#licence) (MIT OR Apache-2.0).

## For maintainers: applying a pull request

In a worktree of the monorepo, on a branch for the change:

```sh
curl -sL https://github.com/monocircuit/breadboard/pull/<n>.patch \
  | git am -3 --directory=packages/breadboard
```

`--directory` puts the paths of the patch under `packages/breadboard`; `git
am` keeps each commit's author, date and message. Add `Refs` to the issue of
the monorepo if there is one, then the usual merge request: the change goes
through the monorepo's checks and review like any other. After its merge,
close the pull request with a link to the mirrored commit.

The mirror never takes anything from this repository: a commit pushed here
directly would make the mirror's next push fail (it never forces).
