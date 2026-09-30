# Contributing

Thanks for looking. This is a single-maintainer project; the fastest way to get a change merged is
to keep it small, tested, and honest about what it does not do.

## Before you write code

Open an issue first for anything beyond a bug fix or a typo. This project has said no to a few
plausible-sounding features already (an account system, telemetry, a market bot, anything that
writes to game memory), and it would rather tell you that before you spend an evening on it.

Two rules that are not negotiable, because breaking either one puts a user's account at risk:

- **Every memory path is read-only.** No writes to the game process. Ever.
- **Session credentials never leave memory.** The account identifier and nonce are not logged, not
  persisted, not printed in `Debug`, not sent anywhere but the inventory endpoint they belong to.

## Setting up

```bash
corepack enable
cd app && pnpm install --frozen-lockfile
```

You will also need the Tauri 2 Linux system libraries and, for the reward reader's tests,
`tesseract` with English data: the OCR tests shell out to the real engine rather than a stand-in.
Per-distribution commands are in [`packaging/README.md`](packaging/README.md).

The Windows half of the port cannot be verified by a Linux `cargo test`, because `cfg(windows)`
code is not compiled by it. CI's `windows-latest` leg remains authoritative for linking and
execution. `./scripts/check-windows.sh` is the local stand-in: it cross-compiles the whole
workspace for `x86_64-pc-windows-gnu`, which compiles `cfg(windows)` code on Linux but cannot
run it, so a platform-gating mistake fails on your machine. It needs Rust's `x86_64-pc-windows-gnu`
target and a MinGW-w64 toolchain.
Heavy cross-target artifacts default to `/var/tmp/tennoscope-$UID`; set `TENNOSCOPE_BUILD_ROOT` to
use a different filesystem.

```bash
cd app && pnpm tauri dev
```

## The check that has to pass

These four are what CI runs on every push and pull request:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm check
```

CI adds the packaging and updater checks on top: `scripts/check-versions.sh`,
`scripts/check-arch-digests.sh`, the `scripts/test-*.sh` harnesses that guard those scripts and the
bundle builder, and `winget validate` on a Windows runner. A second `windows-latest` leg runs clippy
and the suite as well.

The live tests stay `#[ignore]`d. They need a running Warframe session, a live desktop session, or a
warframe.market token in `TENNOSCOPE_LIVE_MARKET_TOKEN`, which is why no normal run, yours or CI's,
ever touches a game process. Run them with `-- --ignored` when you have the session up.

## How this codebase is written

Read a few commits and a few tests before starting; the conventions are visible and consistent.

- **Tests come first.** Nearly everything here was written test-first, and a PR that changes
  behaviour without a test that fails before it is unlikely to merge.
- **Comments explain why, not what.** The interesting comments in this codebase justify a constant,
  name what was measured, or record what was tried and failed. `// increment the counter` is noise;
  `// 74% is the middle of a plateau, not a tuned peak` is the reason the next person does not
  re-tune it.
- **Constants earn their value.** A magic number needs the measurement behind it, in the comment or
  in [`docs/research/`](docs/research).
- **Fail closed.** A partial inventory is rejected rather than merged. A reward card that does not
  match the pool is dropped rather than guessed. Keep that.
- **Errors are the user's language.** `&'static str` messages surface in Diagnostics; write them
  for someone who does not know the internals.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/), lowercase subject, no
trailing period, imperative mood:

```
feat: price reward cards from warframe.market
fix: stop an empty relic pool from disabling the reward poller
```

Types in use: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `style`, `build`, `ci`, `chore`,
`debug`.

Anything that is not trivially self-evident gets a body, wrapped at 80 columns, that says **why**:
what was broken, what was tried, what it cost, and what is still open. Those messages are how a
future reader reconstructs a decision that the diff alone cannot explain.

## Never commit

- Account identifiers, nonces, session tokens, or raw inventory responses.
- Other players' handles or account identifiers, including inside a screenshot or a capture. The
  test fixtures are real reward screens with everything outside the card title band blanked out;
  match that standard.
- Absolute paths from your machine. `scripts/_paths.py` and the app both discover the Wine prefix
  from the live process.
- Anything under `Extracted/`, a local database, or your `EE.log`.

## Scope

Linux is where this is built and tested, and the AppImage, `.deb` and `.rpm` bundles are what a
release publishes. Windows is also a target:
`crates/warframe-acquisition/src/windows_proc.rs` and the `win-capture` crate read the process and
grab frames through DXGI Desktop Duplication, CI builds and tests both on `windows-latest`, and
the release publishes an NSIS installer. macOS has no adapter, no stub and no
`cfg(target_os = "macos")` anywhere in the tree, and none is planned; a PR adding one would have
to come with someone willing to maintain it.

The Linux capture path is four backends deep and none of them proves another. X11/XWayland
through `xcap` wins whenever it finds the game window, with `xwininfo` supplying the rectangle
when Wine's virtual desktop hides it; a native-Wayland game falls through wlroots screencopy,
KWin ScreenShot2 and an already-authorized portal ScreenCast session. So say which session and
compositor you ran on. An XWayland run says nothing about whether KWin will authorize the
installed package, and a checkout binary cannot use that path at all.

## Licensing

By contributing you agree your work is licensed under [GPL-3.0-only](LICENSE), the same as the rest
of the project.
