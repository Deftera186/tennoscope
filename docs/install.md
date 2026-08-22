# Install guide

How to install, run and build TennoScope. The [README](../README.md) has the short
version. Every option below gives you a `tennoscope` command and a desktop entry, except
the AppImage, which is a single file you run directly.

## Gentoo

TennoScope is packaged in the [`deftera`](https://github.com/Deftera186/deftera-overlay)
overlay, which is listed in the official Gentoo overlays database:

```bash
sudo emerge --ask app-eselect/eselect-repository
sudo eselect repository enable deftera
sudo emaint sync --repo deftera
sudo emerge --ask games-util/tennoscope-bin
```

`tennoscope-bin` unpacks the released binary and installs in seconds.
`games-util/tennoscope` builds from source instead; it needs `sys-apps/pnpm-bin` from
`::guru` and a one-off `FEATURES="-network-sandbox"` because pnpm and cargo resolve
their lockfiles during the build.

## Arch, Manjaro, EndeavourOS, CachyOS

Any Arch-based distribution with `pacman` and `makepkg`:

```bash
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch/PKGBUILD && makepkg -si
```

`makepkg -s` pulls the build dependencies itself, so `base-devel` is all you need
beforehand.

On a Steam Deck, `makepkg -si` needs SteamOS's read-only root disabled, and a system
update undoes the install; the [AppImage](#anything-else--appimage) is the
low-maintenance route there.

There is no AUR package yet, so `yay -S tennoscope` and `paru -S tennoscope` will not
find it — AUR helpers install *from* the AUR, and the command above is the supported
route. If you would rather your helper drive the build, point it at a directory holding
the `PKGBUILD`:

```bash
paru -B .    # or: yay -B .
```

Building from source takes a while: it compiles the full Rust workspace. The
[AppImage](#anything-else--appimage) or the `.deb` via
[`debtap`](https://wiki.archlinux.org/title/Debtap) is faster if you just want to run it.

## Debian, Ubuntu, Fedora

Download the `.deb` or `.rpm` from the
[latest release](https://github.com/Deftera186/tennoscope/releases/latest):

```bash
sudo apt install ./TennoScope_*_amd64.deb     # Debian, Ubuntu
sudo dnf install ./TennoScope-*.x86_64.rpm    # Fedora
```

## Windows

Download the `.exe` from the
[latest release](https://github.com/Deftera186/tennoscope/releases/latest) and run it.
It installs for your user only, so there is no UAC prompt, and it carries everything it
needs — there is nothing else to install.

SmartScreen will warn you the first time, because the installer is not code-signed: a
certificate costs money this project does not take. "More info" then "Run anyway" gets
past it.

> [!NOTE]
> **Windows support is best-effort.** This project is developed and tested on Linux; the
> Windows build is compiled and unit-tested in CI, but no Windows machine runs it before
> a release. It is expected to work and bug reports are welcome — just know that a
> Windows-only problem may take a round trip to diagnose, because reproducing it needs a
> machine the author does not have.

> [!IMPORTANT]
> Set **Display Mode** to **Borderless** in Warframe's options. In exclusive fullscreen
> the game owns the display outright and no application can draw over it — the
> collection browser still works, but the reward overlay will not appear. TennoScope
> says so in its diagnostics panel if it hits this.

## Anything else — AppImage

```bash
chmod +x TennoScope_*_amd64.AppImage
./TennoScope_*_amd64.AppImage
```

Self-contained, no `tennoscope` command. If you want one:
`ln -s "$PWD"/TennoScope_*_amd64.AppImage ~/.local/bin/tennoscope`.

## The overlay's toolchain

On Windows there is nothing to do: the installer ships its own copy of Tesseract.

On Linux the collection browser works on its own, and the relic overlay needs
`tesseract` with English data. The `.deb` and `.rpm` list it as recommended rather than
required, so install it if your package manager skipped it:

```bash
sudo apt install tesseract-ocr tesseract-ocr-eng     # Debian, Ubuntu
sudo dnf install tesseract tesseract-langpack-eng    # Fedora
sudo pacman -S tesseract tesseract-data-eng          # Arch
sudo emerge app-text/tesseract                       # Gentoo
```

## Running requirements

- Linux with Warframe running through Wine or Proton, or Windows 10/11 with the native
  client. Either way, logged in.
- Permission to inspect your own game process. On Linux, if acquisition fails, see
  [process permissions](#process-permissions). On Windows no elevation is needed — the
  game runs as the same user.
- On Windows, Warframe set to **Borderless** display mode, or the overlay cannot be
  drawn.
- Network access for the inventory request, the item catalog and market prices. The
  catalog is cached for offline use.

## Building it yourself

```bash
corepack enable
cd app && pnpm install --frozen-lockfile
```

On Linux, build through the helper — it produces AppImage, `.deb` and `.rpm` in
`target/release/bundle/`:

```bash
cd .. && ./scripts/build-linux-bundles.sh appimage deb rpm
```

On Windows, run Tauri directly for an NSIS installer in `target/release/bundle/nsis/`:

```bash
pnpm tauri build
```

The AppImage needs post-processing that Tauri does not do on its own, so prefer the
helper over `pnpm tauri build` on Linux. Building needs Rust 1.85+, Node 20.19+, pnpm 10
and the Tauri 2 Linux libraries; per-distribution prerequisites and the packaging
recipes are in [`packaging/`](../packaging/README.md). A Windows build additionally
wants `scripts/vendor-windows-tesseract.ps1` run first, which fetches the Tesseract the
installer bundles.

## Process permissions

On Windows this section does not apply: TennoScope opens the game with
`PROCESS_VM_READ` as the same user that launched it, which needs no elevation and no
configuration.

On Linux, TennoScope reads `/proc/<pid>/maps` and `/proc/<pid>/mem` of your own game
process. On most distributions this works out of the box, with nothing to configure: the
kernel lets a process inspect others running as the same user. The exception is a
distribution that ships Yama in restricted mode, Ubuntu being the common one:

```bash
cat /proc/sys/kernel/yama/ptrace_scope
```

If the file does not exist, your kernel has no Yama restriction and you are done. `0`
means the same. `1` or higher restricts process inspection to a process's parent, and
TennoScope is not Warframe's parent, so the read is refused. You can lift that until
reboot:

```bash
sudo sysctl kernel.yama.ptrace_scope=0
```

That weakens ptrace isolation for every process you own, so decide for yourself whether
to make it permanent in `/etc/sysctl.conf`. Do not work around the policy by running
TennoScope as root, making the AppImage setuid, or granting it capabilities.

Two requirements apply regardless of Yama: Warframe and TennoScope must run as the same
Unix user, and sandboxed launchers impose `/proc` restrictions that no Yama change will
fix.

## Known limits

- **No macOS.** Warframe has no macOS client, so there is nothing to read.
- **Overlay placement on Linux** draws an override-redirect X11 window over the game
  rectangle, which is window-manager independent: Warframe is an X11 client under Wine
  and Proton alike, and the app joins it there rather than asking the compositor for
  anything. Verified on sway; other compositors are untested rather than unsupported.
- **Overlay placement on Windows** uses a topmost, click-through, never-activated
  window. That beats a borderless game and cannot beat an exclusive-fullscreen one,
  which is why Borderless is a requirement rather than a suggestion. If a driver or
  overlay conflict leaves the strip invisible, `TENNOSCOPE_OPAQUE_OVERLAY=1` draws it
  with a solid background instead.
- **Windows polling costs more than Linux.** There is no `soft-dirty` equivalent, so
  every memory poll rescans every region rather than only the pages the game wrote.
- **Card geometry** is calibrated on 16:9 and scales by window width. Ultrawide is
  untested and may drift.
- **English reward names** only.
- Acquisition depends on undocumented game behaviour and may need maintenance after a
  Warframe update.
