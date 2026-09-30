## Why

<!-- The problem from the user's side, and what it costs them if nothing changes. A change no
user sees still has a why: say what it unblocks. One or two sentences. A reviewer who reads
only this should know whether the change matters. -->

## What this changes

<!-- How. If the approach is not obvious from the diff, say why this one over the one you
rejected. -->

## How it was verified

<!-- Tests are the answer for most changes. If you exercised it against a live game, say which
compositor, which launcher, and what you saw. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd app && pnpm check`

Those four are what CI gates on. It also runs the packaging and updater checks under `scripts/`.
On Linux, `./scripts/check-windows.sh` cross-compiles the Windows target. CI does the same job
with a real `windows-latest` runner instead.

## Checks

- [ ] There is a test that fails without this change, or there cannot be one and you say why.
- [ ] `CHANGELOG.md` has an entry under `## [Unreleased]`, or the change is invisible to a user.
- [ ] No account identifier, nonce, player handle, absolute local path, or raw capture is in the
      diff, including inside any image.
- [ ] Every memory path this touches is still read-only.
- [ ] New constants carry the measurement or reasoning behind them.
- [ ] If an AI agent contributed to this change, name it here. Disclosure only, and welcome; it
      is not a reason to hold the PR.
