# TennoScope

TennoScope is a Linux-first, GPLv3 desktop companion for Warframe. It runs without Overwolf, treats Wine/Proton as a supported environment, and keeps player collection data on the local machine.

The current MVP discovers a running Warframe process, reads its memory without modifying it, obtains an ephemeral inventory authorization value, validates a complete inventory snapshot, enriches it with WFCD catalog data and artwork, and stores the collection in SQLite. The TennoScope interface provides a paginated visual collection index, explicit sync freshness, diagnostics, automatic refresh, and an automatic click-through relic reward overlay on the supported capture path.

This project is unofficial and is not affiliated with or endorsed by Digital Extremes. Process inspection and undocumented game interfaces may carry account-policy or anti-cheat risk even when access is read-only. The application explains this on first run and does nothing until the user accepts the disclosure.

## MVP features

- Native Linux process discovery, including Wine's truncated `Warframe.x64.ex` process name.
- Read-only `/proc/<pid>/maps` and `/proc/<pid>/mem` inventory acquisition.
- Strict, bounded parsing: incomplete snapshots are rejected rather than partially replacing the collection.
- Local SQLite inventory snapshots with authoritative replacement semantics.
- A cached, integrity-checked WFCD item catalog with offline fallback to the last complete generation.
- Automatic refresh when Warframe starts and when `EE.log` reports a completed inventory sync, plus a manual refresh button.
- Canonical item artwork, collection search, category/ownership filters, 48-item pagination, mastery state, and pipeline diagnostics.
- Persisted exact snapshot metadata with a human-readable freshness indicator.
- English reward-name OCR on wlroots through `grim` and Tesseract, with consecutive-frame debounce.
- A non-focusable GTK layer-shell reward strip aligned below the in-game reward row and hidden automatically when recognition ends.
- No account, telemetry service, or cloud synchronization.

## Current limitations

- Live Warframe Market prices are **not implemented**. The MVP does not present live platinum values or make market recommendations.
- Automatic reward capture currently supports wlroots compositors such as sway and Hyprland. PipeWire portal capture for GNOME/KDE Wayland and an X11 adapter remain required for the full desktop matrix; collection synchronization and browsing are unaffected there.
- Inventory acquisition currently targets Linux `/proc` and a Warframe session running through Wine/Proton. Native Windows and macOS acquisition adapters are not included.
- The memory/API technique depends on undocumented game behavior and may need maintenance after a Warframe update.
- The application is English-only, and release repositories/packages are not published yet.

## Requirements

- Linux with a desktop environment or window manager capable of running GTK/WebKit applications.
- Warframe running through Wine or Proton and logged in before inventory acquisition can succeed.
- Permission for the same desktop user to inspect the Warframe process. See [process permissions](#process-permissions-and-yama).
- Network access for the Warframe inventory request and the initial WFCD catalog download. A validated catalog generation is cached for later offline use.
- For the current automatic reward overlay: `grim`, Tesseract with English data, and GTK layer-shell on a wlroots Wayland session.

Build requirements are Rust 1.85 or newer, Node.js 20.19+ (or a compatible version listed in [`app/package.json`](app/package.json)), pnpm 10, and the Tauri 2 Linux system libraries. Distribution-specific prerequisite commands are documented in [packaging/README.md](packaging/README.md).

## Development

Install JavaScript dependencies once:

```bash
cd app
corepack enable
pnpm install --frozen-lockfile
```

Run the desktop application in development mode:

```bash
cd app
pnpm tauri dev
```

Run the complete non-interactive verification suite:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd app
pnpm check
```

The ignored live acquisition tests require a logged-in Warframe session and explicit local opt-in; normal test runs never inspect a game process.

## First run and automatic refresh

On first launch, TennoScope shows a one-time disclosure describing its read-only process inspection, privacy behavior, and account-policy uncertainty. Acceptance is saved locally. After acceptance, acquisition is enabled by default.

Start and log into Warframe normally. TennoScope detects the Proton/Wine process and performs an initial refresh. It also locates the active prefix's `EE.log` and watches for a complete `Inventory sync done` line. That signal schedules another inventory refresh with a cooldown. The log is only a trigger; inventory contents are not scraped from it.

If automatic refresh cannot find the process or log, use the Diagnostics page and try **Refresh inventory** after Warframe has finished logging in.

## Process permissions and Yama

TennoScope must be able to read the same user's `/proc/<pid>/maps` and `/proc/<pid>/mem`. Check the active Yama policy with:

```bash
cat /proc/sys/kernel/yama/ptrace_scope
```

`0` normally permits same-user inspection. With `1` or higher, the kernel may reject access because TennoScope is not Warframe's parent process. For a temporary, system-wide test until reboot:

```bash
sudo sysctl kernel.yama.ptrace_scope=0
```

This weakens ptrace isolation for all same-user processes while enabled. Do not run TennoScope as root, do not make the AppImage setuid, and do not grant broad capabilities merely to bypass the policy. If the temporary setting resolves acquisition, decide whether a persistent sysctl change matches your machine's threat model and distribution policy.

Also verify that Warframe and TennoScope run as the same Unix user. Sandboxed launchers can impose additional `/proc` restrictions that a Yama change will not solve.

## Privacy and network access

- Ephemeral account and nonce values are kept in memory only, redacted from `Debug`/`Display`, and never written to the database or logs.
- Raw inventory responses are validated in memory and are not persisted by default.
- The durable player data is the normalized local collection snapshot, application setup state, and health metadata.
- There is no telemetry, analytics, remote account, or secret persistence.
- Network requests are limited to the pinned Warframe inventory HTTPS origin and the pinned WFCD catalog source in the current MVP.

See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for data-source and research attribution.

## Packaging

Tauri is configured to build AppImage, Debian, and RPM bundles. The repository also includes honest, release-infrastructure-neutral guidance for Arch Linux and Gentoo:

- [Linux packaging overview](packaging/README.md)
- [AppImage](packaging/appimage.md)
- [Arch Linux](packaging/arch.md)
- [Gentoo](packaging/gentoo.md)

No package repository, AUR package, Gentoo overlay, Debian repository, or Fedora repository exists yet.

## License

TennoScope source code is licensed under [GNU GPLv3 only](LICENSE). Warframe and its data remain the property of their respective rights holders. Runtime catalog data has its own upstream licensing; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
