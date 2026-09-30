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

Any Arch-based distribution with `pacman` and `makepkg`. The recommended route
repacks the release `.deb`, so it needs no toolchain and installs in seconds:

```bash
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch-bin/PKGBUILD && makepkg -si
```

The PKGBUILDs on `main` track the release being prepared and may not build until it is
published; the [AppImage](#anything-else-appimage) always works.

To build from source instead, it compiles the full Rust workspace, so it takes a while.
`base-devel` is all you need beforehand, since `makepkg -s` pulls the build dependencies itself:

```bash
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch/PKGBUILD && makepkg -si
```

On a Steam Deck, `makepkg -si` needs SteamOS's read-only root disabled, and a system
update undoes the install; the [AppImage](#anything-else-appimage) is the
low-maintenance route there.

TennoScope is not in the AUR, so `yay -S tennoscope` and `paru -S tennoscope` will not
find it. If you would rather your helper drive the build, point it at a directory holding
the `PKGBUILD`:

```bash
paru -B .    # or: yay -B .
```

## Debian, Ubuntu, Fedora

Debian and Ubuntu install from this project's APT repository, so updates arrive with
everything else. The repository is built from stable releases, so it can lag the newest
release:

```bash
curl -fsSL https://deftera186.github.io/tennoscope/key.asc | sudo gpg --dearmor -o /usr/share/keyrings/tennoscope.gpg
echo "deb [signed-by=/usr/share/keyrings/tennoscope.gpg] https://deftera186.github.io/tennoscope stable main" | sudo tee /etc/apt/sources.list.d/tennoscope.list > /dev/null
sudo apt update
sudo apt install tenno-scope
```

Prereleases reach neither the pool nor `releases/latest`, so the manual download below is a
stable release.

Fedora uses the [COPR](../packaging/copr/README.md):

```bash
sudo dnf copr enable deftera/tennoscope && sudo dnf install tennoscope
```

The COPR is submitted by hand, so it can lag the latest release. The `.rpm` on the latest
stable release is the way around that lag:

```bash
sudo dnf install ./TennoScope-*.x86_64.rpm    # Fedora, without the COPR
```

Prefer manual downloads? The `.deb` is on the
[latest release](https://github.com/Deftera186/tennoscope/releases/latest):

```bash
sudo apt install ./TennoScope_*_amd64.deb     # Debian, Ubuntu
```

## Windows

Download the `.exe` from the
[latest release](https://github.com/Deftera186/tennoscope/releases/latest) and run it.
It installs for your user only, so there is no UAC prompt, and it carries its own copy of
Tesseract. There is nothing else to install by hand. The installer fetches the Microsoft
Edge WebView2 runtime itself on a machine that does not already have it, so the first
install needs a network.

SmartScreen will warn you the first time because the installer is not code-signed. "More
info" then "Run anyway" gets past it.

> [!NOTE]
> **Windows support is best-effort.** This project is developed and tested on Linux. CI
> compiles and unit-tests the Windows backends on a `windows-latest` runner, and a local
> Windows VM has checked the packaged installer, but the release job's `windows` job only
> builds that installer with `pnpm tauri build` and never runs it. The evidence behind a
> Windows release is a unit test and one manual install, not a development machine, so
> expect a round trip to diagnose a Windows-only problem.

> [!IMPORTANT]
> Set **Display Mode** to **Borderless** in Warframe's options. In exclusive fullscreen
> the game owns the display outright and no application can draw over it. The
> collection browser still works, but the reward overlay will not appear. TennoScope
> says so in its diagnostics panel if it hits this.

## Anything else: AppImage

```bash
chmod +x TennoScope_*_amd64.AppImage
./TennoScope_*_amd64.AppImage
```

Self-contained, no `tennoscope` command. If you want one:
`ln -s "$PWD"/TennoScope_*_amd64.AppImage ~/.local/bin/tennoscope`.

## The overlay's toolchain

On Windows there is nothing to do: the installer bundles its own copy of Tesseract and
its English data.

On Linux the collection browser works on its own, and the relic overlay shells out to
`tesseract` for the reward card titles. `tauri.conf.json` lists the engine as a
recommendation, not a requirement, so install it if your package manager skipped it:

```bash
sudo apt install tesseract-ocr tesseract-ocr-eng     # Debian, Ubuntu
sudo dnf install tesseract tesseract-langpack-eng    # Fedora
sudo pacman -S tesseract tesseract-data-eng          # Arch
sudo emerge app-text/tesseract                       # Gentoo
```

The `.deb` recommends both, but the `.rpm` and the COPR spec leave out the English data,
so Fedora needs the `tesseract-langpack-eng` line of its own.

Wine's virtual-desktop mode nests the game where the normal window search cannot see it, so
the X11 path falls back to `xwininfo -root -tree`. Install your distribution's `xwininfo`
package if you run Warframe that way: `x11-utils` on Debian and Ubuntu, `xwininfo` on Fedora,
`xorg-xwininfo` on Arch, `x11-apps/xwininfo` on Gentoo. Everything else on that path is
in-process; capture, cropping, thresholding and window geometry never leave the app, and
ImageMagick is no longer a runtime dependency of the application at all. Outside the reward
path, the Diagnostics report opens its issue link through `xdg-open`.

## Running requirements

- Linux with Warframe running through Wine or Proton, or Windows 10/11 with the native
  client. Either way, logged in.
- Permission to inspect your own game process. On Linux, if acquisition fails, see
  [process permissions](#process-permissions). On Windows no elevation is needed, since the
  game runs as the same user.
- Network access for the inventory request, the item catalog and market prices. The
  catalog is cached for offline use.

## Building it yourself

```bash
corepack enable
cd app && pnpm install --frozen-lockfile
```

On Linux, build through the helper. It accepts `appimage`, `deb` and `rpm` as arguments,
defaults to `appimage`, derives the repository root from its own path so the directory
you invoke it from does not matter, and writes the bundles under `target/release/bundle/`
at the repository root. From the repository root:

```bash
./scripts/build-linux-bundles.sh appimage deb rpm
```

On Windows, run Tauri directly for an NSIS installer in
`target/release/bundle/nsis/`. `tauri` is a `pnpm` script in `app/package.json`, so run
it from `app/`:

```bash
pnpm tauri build
```

The toolchain is pinned rather than ranged, and each pin lives in the file that enforces it.
`rust-toolchain.toml` pins Rust, the workspace declares an MSRV in `Cargo.toml`, and CI
reads `app/.node-version`. `app/package.json` accepts Node `^20.19.0 || >=22.12.0 <27` and
names the pnpm version in its `packageManager` field, which is the version `corepack enable`
above selects. Linux builds also need the Tauri 2 libraries; per-distribution
prerequisites and the packaging recipes are in
[`packaging/`](../packaging/README.md).

A Windows build wants `scripts/vendor-windows-tesseract.ps1` run first, in PowerShell
with 7-Zip on PATH. It downloads the pinned UB-Mannheim Tesseract and extracts the
executable, its DLLs and the `eng` and `osd` trained data into
`app/src-tauri/vendor/tesseract/`, which `tauri.windows.conf.json` globs into the
installer. The script exits early if that directory already holds a `tesseract.exe`.

## Process permissions

On Windows this section does not apply: TennoScope opens the game with
`PROCESS_VM_READ` as the same user that launched it, which needs no elevation and no
configuration.

On Linux, TennoScope reads `/proc/<pid>/maps` and `/proc/<pid>/mem` of your own game
process, and resets the soft-dirty page bits through `/proc/<pid>/clear_refs` so a poll
only rescans the pages the game wrote. On most distributions this works out of the box,
with nothing to configure: the kernel lets a process inspect others running as the same
user. The exception is a distribution that ships Yama in restricted mode, Ubuntu being
the common one:

```bash
cat /proc/sys/kernel/yama/ptrace_scope
```

If the file does not exist, your kernel has no Yama restriction and you are done. `0`
means the same. `1` restricts inspection to a process's own parent, `2` restricts it to
processes holding `CAP_SYS_PTRACE`, and `3` stops one process inspecting another at all.
TennoScope is not Warframe's parent and holds no such capability, so anything above `0`
refuses the read. You can lift that until reboot:

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

- **No macOS.** This project ships Linux and Windows packages only, and a macOS
  acquisition adapter is out of scope.
- **Overlay placement on Linux** uses a click-through, override-redirect X11 window. Placement
  against Warframe's game rectangle is verified for ordinary X11/XWayland play. A native-Wayland
  game launched with `PROTON_ENABLE_WAYLAND=1` exposes no game rectangle, so placement uses its
  captured output, depends on the compositor, and assumes Borderless or Fullscreen on one screen.
  TennoScope shows an explicit display-mode notice when it cannot find the game window.
- **Overlay placement on Windows** uses a topmost, click-through, never-activated
  window. That beats a borderless game and cannot beat an exclusive-fullscreen one,
  which is why Borderless is a requirement rather than a suggestion. If a driver or
  overlay conflict leaves the strip invisible, `TENNOSCOPE_OPAQUE_OVERLAY=1` draws it
  with a solid background instead.
- **Windows polling costs more than Linux.** There is no `soft-dirty` equivalent, so
  every memory poll rescans every region rather than only the pages the game wrote.
- **Card geometry** is measured against a 1920x1080 reward screen and scaled by window
  height, which is how Warframe scales its HUD, with each position an offset from the
  window's horizontal centre. 16:10, a Steam Deck's native 1280x800, has its own capture
  fixture. Wider ratios are untested and may drift.
- **English reward names only.** The reader matches the English display names in the item
  catalog.
- Acquisition depends on undocumented game behaviour and may need maintenance after a
  Warframe update.
