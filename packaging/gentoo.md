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

Both pull in the WebKitGTK stack, plus the relic overlay's one external tool,
`app-text/tesseract`, whose English data is installed unconditionally. Window location and the
crop pipeline used to need `x11-apps/xwininfo` and `media-gfx/imagemagick`; both are in-process
now and neither is a dependency any more.

## Building an untagged commit

There is no ebuild for this: a local checkout has no immutable `SRC_URI` to point at. Build the
bundle directly and run it out of `target/`.

```bash
./scripts/build-linux-bundles.sh appimage
```
