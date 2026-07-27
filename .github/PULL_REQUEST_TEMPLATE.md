## What this changes

<!-- What was broken or missing, and what this does about it. If it is not obvious from the diff,
say why this approach over the one you rejected. -->

## How it was verified

<!-- Tests are the answer for most changes. If you exercised it against a live game, say which
compositor, which launcher, and what you saw. -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cd app && pnpm check`

## Checks

- [ ] There is a test that fails without this change (or the change cannot have one — say why).
- [ ] No account identifier, nonce, player handle, absolute local path, or raw capture is in the
      diff, including inside any image.
- [ ] Every memory path this touches is still read-only.
- [ ] New constants carry the measurement or reasoning behind them.
