<div align="center">

# TennoScope

**A Warframe companion for Linux. No Overwolf, no account, no telemetry.**

Reads your collection off the running game and tells you which relic reward is worth taking,
while the timer is still going.

[![CI](https://github.com/Deftera186/tennoscope/actions/workflows/ci.yml/badge.svg)](https://github.com/Deftera186/tennoscope/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-informational.svg)](#install)

</div>

---

## Relic overlay

A relic cracks and you get four rewards and fifteen seconds. TennoScope reads the cards off the
screen and puts prices under them, in platinum and in ducats, since those two usually disagree.

![The reward overlay, aligned under the in-game reward row](docs/screenshots/reward-overlay.png)

Prices come from warframe.market and count only sellers who are actually online, because an
offline listing is a price nobody can trade at. The squad's whole relic pool is priced while the
mission is still running, so the numbers are already local when the screen appears. Untradeable
items get a dash.

The overlay is click-through and never takes focus from the game.

## Collection

Everything the game says you own: frames, weapons, companions, prime parts, relics, resources,
blueprints, vehicles. With artwork, mastery state, search and filters, stored locally.

![The collection browser](docs/screenshots/collection.png)

It refreshes itself. TennoScope notices Warframe starting, watches the game's log for a completed
inventory sync, and re-reads. The log is only a trigger, nothing is scraped out of it.

---

## Read this before you install it

> [!IMPORTANT]
> TennoScope reads the Warframe process's memory to obtain a session token, then asks Warframe's
> own inventory endpoint for your collection. **It never writes to the game, and never modifies,
> automates, or influences gameplay.** Digital Extremes has not endorsed this, and any third-party
> tool that inspects a game process may carry account-policy or anti-cheat risk.

Other community tools have done this for years and DE has not acted on it, but that is not the
same as permission. If you are not willing to accept the risk, do not run this.

TennoScope shows this disclosure on first run and does nothing until you accept it.

## Install

No release is published yet. Build it:

```bash
corepack enable
cd app && pnpm install --frozen-lockfile
pnpm tauri build          # AppImage, .deb and .rpm land in target/release/bundle/
```

Per-distribution prerequisites, plus Arch and Gentoo recipes, are in
[`packaging/`](packaging/README.md). Building needs Rust 1.85+, Node 20.19+, pnpm 10 and the
Tauri 2 Linux libraries.

Running needs:

- Linux, with Warframe running through Wine or Proton and logged in.
- Permission to inspect your own game process. If acquisition fails, see
  [process permissions](#process-permissions).
- For the overlay: `xwininfo`, ImageMagick 7 (`magick`, not the v6 `convert`) and `tesseract` with
  English data.
- Network access for the inventory request, the item catalog and market prices. The catalog is
  cached for offline use.

## Known limits

- **Linux only, for now.** Acquisition reads `/proc`. Windows and macOS support is the goal, but
  no adapter exists yet.
- **Overlay placement** reads the game rectangle from `xwininfo` and draws an override-redirect
  X11 window over it, which is window-manager independent: Warframe is an X11 client under Wine
  and Proton alike, and the app joins it there rather than asking the compositor for anything.
  Verified on sway; other compositors are untested rather than unsupported.
- **Card geometry** is calibrated on 16:9 and scales by window width. Ultrawide is untested and
  may drift.
- **English reward names** only.
- Acquisition depends on undocumented game behaviour and may need maintenance after a Warframe
  update.

## Privacy

The account identifier and nonce are session credentials: they stay in memory, are redacted from
`Debug` and `Display`, and never reach the database or a log. Raw inventory responses are
validated in memory and not persisted. What lands on disk is your normalized collection snapshot,
setup state and health metadata.

No telemetry, no analytics, no remote account, no crash reporting. Network requests go to the
pinned Warframe inventory origin, the pinned catalog source and warframe.market. Nothing else.

## Process permissions

TennoScope must read `/proc/<pid>/maps` and `/proc/<pid>/mem` of your own game process.

```bash
cat /proc/sys/kernel/yama/ptrace_scope   # 0 normally permits same-user inspection
```

At `1` or higher the kernel may refuse, because TennoScope is not Warframe's parent.
`sudo sysctl kernel.yama.ptrace_scope=0` lifts that until reboot, at the cost of ptrace isolation
for every process you own, so decide whether a permanent change fits your machine. **Do not run
TennoScope as root, do not make the AppImage setuid, and do not grant it capabilities to work
around the policy.** Warframe and TennoScope also have to run as the same Unix user; sandboxed
launchers impose `/proc` restrictions that no Yama change will fix.

## Development

```bash
cd app && pnpm tauri dev
```

The full check, which is what CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm check
```

Live tests are `#[ignore]`d, so a normal test run never touches a game process.

[CONTRIBUTING](CONTRIBUTING.md) · [SECURITY](SECURITY.md) · [docs](docs/README.md) ·
[RELEASING](RELEASING.md) · [CHANGELOG](CHANGELOG.md)

## License

[GPL-3.0-only](LICENSE). Warframe, its data and its artwork remain the property of Digital
Extremes; runtime catalog data has its own upstream licensing, see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

TennoScope is unofficial and not affiliated with or endorsed by Digital Extremes.
