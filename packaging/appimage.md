# AppImage

AppImage is the recommended cross-distribution artifact for early releases.

Build it from the repository root with:

```bash
./scripts/build-linux-bundles.sh appimage
```

The artifact is written under `target/release/bundle/appimage/`, and the helper fails the
build unless the result is executable. Run it as the same Unix user that runs Warframe:

```bash
chmod +x TennoScope_*_amd64.AppImage
./TennoScope_*_amd64.AppImage
```

The helper does three things to this artifact that Tauri does not. The generated
desktop entry is rewritten to a PATH-resolved `Exec=tennoscope`, because the
`/usr/bin/tennoscope` the native desktop template needs does not exist on another host, and
the KWin permission key is stripped out of it. `usr/lib/libwayland-client.so.0` is then
deleted from the AppDir: Tauri pins a 2024 linuxdeploy whose excludelist predates that
library, and the older bundled copy cannot drive the host's Mesa EGL vendor. World
permission bits are normalized across the AppDir before the repack (`chmod -R o+rX`):
v0.12.0 shipped `AppRun.wrapped` at 770, which a sandbox running as neither owner nor
group could not execute, and the catalog test runs exactly that way. linuxdeploy
then repacks the AppDir, and the helper extracts the result again: it checks that the
rewritten desktop entry is the one that shipped, and it fails if any file or executable in
the extracted payload lacks its world bits. Directories are checked on the AppDir instead,
before packing, because `--appimage-extract` creates every directory 700 whatever the image
stores; mounting the artifact shows 755 everywhere, and the mount is what a sandboxed user
traverses. It also refuses to continue if the GTK
plugin's generated launcher stops forcing `GDK_BACKEND=x11`, which is what the reward
overlay needs: that environment variable would override the backend the app requests for
itself. Build AppImages through the helper rather than invoking
`pnpm tauri build --bundles appimage` directly.

The helper sets `NO_STRIP=true` for AppImage assembly. Tauri's `linuxdeploy` bundles a
`strip` that cannot read the newer ELF RELR sections found on distributions whose toolchain
emits `.relr.dyn`. Skipping this optional packaging-time strip step produces a larger artifact
but preserves the already optimized Rust executable and allows the bundle to complete.

Tauri downloads its linuxdeploy plugins into its own cache under `~/.cache/tauri` during
the first build, so a build machine needs network access for that step.

On KDE, the AppImage uses the screen-sharing portal for reward capture. KDE authorizes a
caller by comparing the desktop entry's `Exec` with the running executable, and an AppImage's
path is ephemeral, so the two can never match; the app refuses the KWin rung outright when
`APPIMAGE` is set. Install the deb, rpm, Arch, or Gentoo package instead if you want silent
KDE capture.

The AppImage bundles neither of the two programs the app shells out to, because both are spawned
by bare name from `PATH` rather than from the AppDir. The Windows installer bundles Tesseract,
because Windows has no package manager to lean on. On Linux the OCR path runs whatever
`tesseract` is on `PATH`, so install one with English data if you want the relic overlay. The
collection browser works without it.

`xwininfo` is the other one, needed only when Wine's virtual-desktop mode hides the game from the
ordinary window list, and the collection browser and the marketplace do not use it either. Install
your distribution's `xwininfo` package if you run the game that way: `x11-utils` on Debian and
Ubuntu, `xorg-xwininfo` on Arch, `xwininfo` on Fedora, `x11-apps/xwininfo` on Gentoo. Gentoo is
the exception: Portage cannot recommend a package, so the overlay ebuilds require
`app-text/tesseract` and do not declare `xwininfo` at all. When one is missing the Diagnostics
`Reward observer` row names it: `Screen capture failed: xwininfo is not installed`.

Some distributions no longer install FUSE 2 compatibility by default. Prefer installing
the distribution's FUSE 2 compatibility package. For a one-off fallback, AppImage supports
extraction-and-run mode:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./TennoScope_*_amd64.AppImage
```

The AppImage does not bypass `/proc` or Yama restrictions, does not contain Warframe, and
should never be run as root or made setuid. The first catalog download still requires
network access.
