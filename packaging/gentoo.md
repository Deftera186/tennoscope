# Gentoo

TennoScope is packaged in the [`deftera`](https://github.com/Deftera186/deftera-overlay) overlay,
which is listed in the [official Gentoo overlays
database](https://api.gentoo.org/overlays/repositories.xml). The ebuilds live there rather than in
this repository — one copy, maintained where `pkgcheck` runs against it.

```bash
sudo emerge --ask app-eselect/eselect-repository
sudo eselect repository enable deftera
sudo emaint sync --repo deftera
```

## Which package

**`games-util/tennoscope-bin`** unpacks the `.deb` from the GitHub release. It installs in seconds,
needs no Node or Rust toolchain, and is the recommended package.

```bash
sudo emerge --ask games-util/tennoscope-bin
```

**`games-util/tennoscope`** builds from the release tarball. It needs `sys-apps/pnpm-bin` from
[`::guru`](https://wiki.gentoo.org/wiki/Project:GURU), and a one-off `FEATURES` override because
pnpm and cargo both resolve their lockfiles over the network during the build:

```bash
sudo eselect repository enable guru && sudo emaint sync --repo guru
sudo FEATURES="-network-sandbox" emerge --ask games-util/tennoscope
```

The two block each other; emerge one or the other.

## Runtime dependencies

Both pull in the WebKitGTK stack, plus the relic overlay's external OCR tool,
`app-text/tesseract`, whose English data is installed unconditionally. ImageMagick is no longer
needed. Wine virtual-desktop child discovery still runs `xwininfo -root -tree`, so install
`x11-apps/xwininfo` when using that mode.

## KDE screen-capture authorization

KWin permits silent ScreenShot2 capture only when TennoScope is installed with its desktop entry.
If capture says it was not authorized, close the checkout or raw `target/` binary and launch the
installed `games-util/tennoscope` or `games-util/tennoscope-bin` package. Downstream ebuilds must
preserve `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` in that desktop entry.
AppImages cannot use this KWin authorization path and use the portal fallback instead.

## Building an untagged commit

There is no ebuild for this: a local checkout has no immutable `SRC_URI` to point at. Do not run
the raw `target/` binary for KDE capture because it has no installed desktop-entry identity.
Build the AppImage instead; it uses the portal fallback.

```bash
./scripts/build-linux-bundles.sh appimage
```
