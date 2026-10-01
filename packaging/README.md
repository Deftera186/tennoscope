# Linux packaging

This directory is for people building or repackaging TennoScope. If you only want to install it,
the [README's install section](../README.md#install) is shorter.

A release attaches the AppImage, Debian (`.deb`) and RPM (`.rpm`) bundles Tauri generates, and a
Windows NSIS installer. Beyond those:

- **Gentoo**: packaged in the [`deftera`](https://github.com/Deftera186/deftera-overlay) overlay,
  which is in the official overlays database. See [gentoo.md](gentoo.md).
- **Arch**: [`arch/PKGBUILD`](arch/PKGBUILD) builds from the release tarball, and
  [`arch-bin/PKGBUILD`](arch-bin/PKGBUILD) repacks the release `.deb` for a fast
  install with no toolchain. No AUR package is published, so the [AppImage](appimage.md) is
  the route that always works. [arch.md](arch.md) documents the window and the digest rules
  that go with it.
- **Fedora**: a [`COPR`](https://copr.fedorainfracloud.org/coprs/deftera/tennoscope/) project;
  see [copr/README.md](copr/README.md).
- **Debian/Ubuntu**: this project's APT repository, published from each release's `.deb`;
  see [apt/README.md](apt/README.md).
- **Windows**: [`winget`](winget.md) manifests for submission to `winget-pkgs`.

Release artifacts are minisign-signed for the in-app updater. The key is `plugins.updater.pubkey`
in [`../app/src-tauri/tauri.conf.json`](../app/src-tauri/tauri.conf.json), and the release workflow
gates each published feed against it with
[`../scripts/verify-update-feed.sh`](../scripts/verify-update-feed.sh). The APT repository carries
its own GPG key. Flathub is [evaluated and deferred](flathub.md).

All source builds need network access to resolve the Cargo and pnpm lockfiles, unless a
distributor vendors them and an offline workflow, which is what the COPR spec does.

## Common preparation

Three toolchains, three different pins. `rust-toolchain.toml` pins the Rust channel and
rustup installs it on the first `cargo` call; `Cargo.toml` sets `rust-version = "1.85"`, which is
the minimum the code supports rather than what a release is built with. Node has to satisfy
`app/package.json`'s `^20.19.0 || >=22.12.0 <27` (CI reads `app/.node-version`), and Corepack
installs the pnpm version its `packageManager` field pins. Then your distribution's
Tauri 2 WebKitGTK prerequisites:

```bash
corepack enable
cd app
pnpm install --frozen-lockfile
```

Build one or more bundles:

```bash
./scripts/build-linux-bundles.sh appimage
./scripts/build-linux-bundles.sh deb
./scripts/build-linux-bundles.sh rpm
./scripts/build-linux-bundles.sh appimage deb rpm
```

The helper accepts `appimage`, `deb` or `rpm` and defaults to `appimage`. It resolves the
repository root from its own path, so the working directory does not matter. It never invokes
`sudo`, installs packages, or changes system configuration. By default it gates on
`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `pnpm check`;
`--skip-gates` drops those three, which is what the release workflow passes because it refuses to
start until CI has passed on the same commit. Prefer it over calling
`pnpm tauri build --bundles appimage` directly: the AppImage needs three post-processing steps Tauri
does not perform on its own. The generated desktop entry is rewritten to a PATH-resolved
`Exec=tennoscope` with the KWin permission key stripped, `usr/lib/libwayland-client.so.0` is
deleted from the AppDir, and world permission bits are normalized across it before
linuxdeploy repackages it. See [appimage.md](appimage.md).

Generated files appear beneath `target/release/bundle/` in target-specific directories.

## Debian/Ubuntu prerequisites

The libraries below are the ones CI's `Rust` job installs (`.github/workflows/ci.yml`), the
release job adds `libarchive-tools` for `bsdtar` and `rpm` for the rpm target, and a local run
also wants `build-essential` and, because the gates run the OCR tests, `tesseract-ocr` with
`tesseract-ocr-eng`:

```bash
sudo apt update
sudo apt install build-essential \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  libxdo-dev libssl-dev libpipewire-0.3-dev libclang-dev libgbm-dev libegl-dev \
  libwayland-dev libxcb1-dev libarchive-tools rpm \
  tesseract-ocr tesseract-ocr-eng
```

Install Node.js, Corepack/pnpm, and Rust separately using your preferred distribution-supported
method. Building a `.deb` does not publish or configure an APT repository.

## Fedora prerequisites

This is the spec's own `BuildRequires` list ([`copr/tennoscope.spec`](copr/tennoscope.spec)),
which is the one Fedora build this repository proves. Add `rpm` and `libarchive` for the helper's
rpm assertions (`rpm -qp --requires` and `bsdtar`), and `dpkg` if you build the deb too:

```bash
sudo dnf install cargo clang gcc gcc-c++ make pnpm wget \
  libappindicator-gtk3-devel libglvnd-devel mesa-libEGL-devel mesa-libgbm-devel \
  openssl-devel pipewire-devel webkit2gtk4.1-devel \
  tesseract tesseract-langpack-eng rpm libarchive
```

The spec takes Rust and pnpm from packages because a COPR chroot has no network to fetch them
from; a desktop build can use rustup and Corepack instead. Building an `.rpm` does not create a
DNF/Copr repository or sign the package.

## Distribution notes

- AppImage builds are the broadest single-file output but still depend on a sufficiently compatible
  Linux userspace; see [appimage.md](appimage.md).
- Tauri generates the runtime list. The release `.deb` depends on `libwebkit2gtk-4.1-0` and
  `libgtk-3-0` from that generator, plus the `libpipewire-0.3-0` this project configures. Declare
  what Tauri needs rather than bundling system libraries blindly.
- The app reaches three external programs. Two it spawns itself to read the game screen:
  `tesseract`, for the reward cards and for the Ducat Kiosk grid tiles, basket rows and quantity
  prefixes, which reuse the same engine and spawn path, and `xwininfo -root -tree` on Linux, which
  is how Warframe is found when Wine's virtual desktop hides it from the ordinary window list.
  Both are spawned directly, with a fixed argument list and no shell in between. Neither is a hard
  dependency, because the collection browser and the marketplace never call either, and only Wine's
  virtual-desktop mode reaches `xwininfo` at all. deb, rpm, Arch and COPR recommend rather than
  require them:
  `bundle.linux.deb.recommends` and `bundle.linux.rpm.recommends` in
  [`../app/src-tauri/tauri.conf.json`](../app/src-tauri/tauri.conf.json) for deb and rpm,
  `optdepends` in the two Arch recipes, and `Recommends` in the COPR spec. Gentoo is the
  exception: Portage cannot recommend a package, so the overlay ebuilds require
  `app-text/tesseract` and name `x11-apps/xwininfo` in a `pkg_postinst` elog instead. The third
  program is `xdg-open`, reached by the Diagnostics report when the user opens the issue link
  or reveals the saved report folder. It is spawned by the Rust side of
  `@tauri-apps/plugin-opener`, so no `Command::new` in this repository names it; both Arch
  recipes and the COPR spec carry `xdg-utils` as a hard dependency. A user whose acquisition
  fails on a missing `xwininfo` sees `Screen capture failed: xwininfo is not installed` on the
  Diagnostics `Reward observer` row, so point at that symptom rather than at a package name.
  ImageMagick is no longer used by the app, only by the research scripts under `../scripts/`.
  Per-format package names are in the per-distribution files: [arch.md](arch.md),
  [gentoo.md](gentoo.md), [copr/README.md](copr/README.md).
- Do not package the application with setuid bits or broad ptrace capabilities. Document Yama
  requirements instead.
- On KDE, silent ScreenShot2 capture is authorized from the installed desktop entry. That is why
  [`tennoscope.desktop`](tennoscope.desktop) and the deb/rpm template
  [`../app/src-tauri/tennoscope.desktop.hbs`](../app/src-tauri/tennoscope.desktop.hbs) both declare
  `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`. If capture says it was not
  authorized, close the checkout or the raw `target/` binary and run the installed package.
  Downstream packages must preserve that key. The AppImage drops it on purpose and uses the portal
  fallback.
- Preserve `LICENSE`, `THIRD_PARTY_NOTICES.md` and the WFCD attribution inside them. Both Arch
  recipes, the COPR spec and Tauri's own bundle configuration ship both files.
