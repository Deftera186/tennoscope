<div align="center">

# TennoScope

**A free and open-source, Rust-based Warframe companion for Linux, Windows and the Steam Deck.
No Overwolf, no account, no telemetry.**

[![CI](https://github.com/Deftera186/tennoscope/actions/workflows/ci.yml/badge.svg)](https://github.com/Deftera186/tennoscope/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Deftera186/tennoscope)](https://github.com/Deftera186/tennoscope/releases/latest)
[![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-blue.svg)](LICENSE)
[![Platform: Linux | Windows | Steam Deck](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20Steam%20Deck-informational.svg)](#install)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584)](https://www.rust-lang.org/)

</div>

<div align="center">

## Reward overlay

</div>

See platinum and ducats under each reward the moment it appears. TennoScope sees the
reward arrive in `EE.log`, reads the cards with Tesseract and draws the values under the
row. It also marks what you own, what you are missing and what you still need for mastery.
It prices the whole squad's relic pool using sellers who are in game right now, without
taking focus from the game.

![The reward overlay on a Warframe Void Fissure reward screen, with a platinum figure, a ducat figure and an owned or not owned line under each card](docs/screenshots/reward-overlay.png)

<div align="center">

## Ducat Kiosk overlay

</div>

Before you trade a Prime part for ducats, check what it is worth in platinum. TennoScope
puts a platinum figure on every tile of the grid, carries each pick into the sell list, and
totals the sell list as it fills.

![The Ducat Kiosk overlay, with a platinum figure on every Prime part tile, the picked items and their values in a sell list on the right, and a running total at the bottom](docs/screenshots/ducat-kiosk.png)

<div align="center">

## Collection

</div>

Open one screen for gear, Prime parts, relics, resources, blueprints, mods and arcanes.
Search by name, or filter to owned, mastered or missing gear. Full mode synchronizes when
the game starts; Companion and Overlay keep the last collection visible as a timestamped
saved snapshot.

![The collection browser, with tracked, mastered and missing counts, a collection worth in platinum and ducats at stake across the top, and platinum and ducat values on every item card](docs/screenshots/collection.png)

See platinum and ducat values side by side, per item and across your collection. Market
prices come from a daily warframe.market trade summary, served by relics.run, with mods and
arcanes priced at their actual rank.

Prime parts use Baro Ki'Teer's posted ducat values. Sort by either currency or hide the
ducat figures when you only care about platinum.

<div align="center">

## warframe.market integration

</div>

Connect warframe.market only if you want to. Once linked, TennoScope flags listings for
items you no longer own or that oversell what you hold. You can also list, delist and change
your online status without leaving the app.

![The warframe.market orders page, with the listed platinum total, a count needing attention, the online status buttons, and each sell order with a remove button](docs/screenshots/market-orders.png)

<div align="center">

## You decide what it may touch

</div>

Worried about third-party tools and your Warframe account? Fair. TennoScope asks
up front: run as a Companion that never looks at the game, add Overlay screen
reading for the reward advisor, or go Full for automatic inventory sync. Nothing
starts until you confirm, and no mode ever writes to the game, automates input,
or sends telemetry. Change your mind later and the lower mode retires everything it
no longer permits before the switch completes.

![The one-time setup screen, with Companion, Overlay and Full as the three access levels, Full selected, and the list of what Full adds](docs/screenshots/warframe-access.png)

> [!IMPORTANT]
> **Choose the access you want before TennoScope observes Warframe.** First run
> preselects Full and starts no running-game access until you confirm one. Full reads an
> account ID and a nonce from the Warframe process and sends them to
> `mobile.warframe.com`'s inventory endpoint and nowhere else. Overlay does not read process
> memory. TennoScope never writes to the game or automates an action. It is an unofficial
> project and is not endorsed by Digital Extremes.

<div align="center">

## Install

</div>

| System | How |
| --- | --- |
| Windows 10 or 11 | [Installer](https://github.com/Deftera186/tennoscope/releases/latest) |
| Debian, Ubuntu | `sudo apt install tenno-scope` from this project's [APT repository](docs/install.md#debian-ubuntu-fedora) |
| Fedora | `sudo dnf copr enable deftera/tennoscope && sudo dnf install tennoscope` |
| Arch-based, incl. Steam Deck | `curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch-bin/PKGBUILD && makepkg -si` |
| Gentoo | `games-util/tennoscope-bin` from the [`deftera`](https://github.com/Deftera186/deftera-overlay) overlay |
| Any other Linux | [AppImage](https://github.com/Deftera186/tennoscope/releases/latest) |

- **Windows:** use Borderless display mode in Warframe. Exclusive fullscreen prevents
  the overlay from appearing. SmartScreen will warn about the unsigned installer; choose
  "More info", then "Run anyway".
- **Linux:** Warframe runs through Wine or Proton. The overlays need `tesseract` with
  English language data, and Wine virtual-desktop mode also needs `xwininfo`. The
  collection works without either.
- **Screens:** both readers match the English item names in the catalog, so a
  non-English Warframe client is not read.

Need help with a particular distribution, building from source or process permissions?
See the [full install guide](docs/install.md).

<div align="center">

## License

</div>

[GPL-3.0-only](LICENSE). TennoScope is an unofficial project and is not endorsed by
Digital Extremes. Warframe and its artwork belong to Digital Extremes; see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

<p align="center">

[Install guide](docs/install.md) · [Docs](docs/README.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)

</p>
