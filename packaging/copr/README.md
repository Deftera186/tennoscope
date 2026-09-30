# COPR (Fedora) packaging

Project: [`deftera/tennoscope`](https://copr.fedorainfracloud.org/coprs/deftera/tennoscope/),
built in the `fedora-43-x86_64` and `fedora-44-x86_64` chroots. COPR signs the packages it
builds itself, so nothing in this repository holds a packaging key: the spec has no signing
section and no key file lives under `packaging/copr/`.

## What the project serves

This COPR builds only what has been submitted to it, and submission is manual, so it can lag
the newest release. `sudo dnf install tennoscope` installs the newest build the COPR holds; the
[packages page](https://copr.fedorainfracloud.org/coprs/deftera/tennoscope/packages/) lists
that version and its build status. To take a release the COPR has not caught up with, install
the `.rpm` off that release:

```bash
sudo dnf install ./TennoScope-<ver>-1.x86_64.rpm
```

Closing that gap takes two human steps, in this order: the spec has to name the version being
shipped, and a source build for that version has to be submitted.

## Layout

- `tennoscope.spec`: source recipe, a locked Cargo workspace plus the pnpm frontend, same
  shape as [`../arch/PKGBUILD`](../arch/PKGBUILD) with a raw binary install and no Tauri
  bundle step. `Name:` is `tennoscope`, which is what users install.
- `Version:` is the one declaration nothing checks. `scripts/check-versions.sh` compares
  exactly five of them: `Cargo.toml`, `app/src-tauri/tauri.conf.json`, `app/package.json`, and the
  `pkgver` line in each of `packaging/arch/PKGBUILD` and `packaging/arch-bin/PKGBUILD`. No
  other script and no workflow looks at this spec, so CI will not catch it drifting.
- It tracks the newest stable tag, not the workspace version: the workspace version may be
  untagged, leaving `Source0` with no tarball to fetch, and a prerelease sorts above a stable
  version in rpm, so a build from it would take over a channel the docs present as the plain
  install path. Bump it by hand per stable release and append a `%changelog` entry.

## Submit a build

A COPR chroot has no network, so every dependency rides inside the SRPM. `rpmbuild -bs` is
the step that puts them there: it fetches `Source0` (the tag archive) here, where the
network is, and packs it and the two vendored tars into the SRPM the chroot receives.

```bash
# Per release, from the repo root, with <ver> the version being submitted:
cargo vendor vendor
(cd app && tar -I 'zstd -12' -cf ~/rpmbuild/SOURCES/tennoscope-node-modules-<ver>.tar.zst node_modules)
tar -I 'zstd -12' -cf ~/rpmbuild/SOURCES/tennoscope-vendor-<ver>.tar.zst vendor
cp packaging/copr/tennoscope.spec ~/rpmbuild/SPECS/
rpmbuild -bs ~/rpmbuild/SPECS/tennoscope.spec
copr-cli build deftera/tennoscope ~/rpmbuild/SRPMS/tennoscope-<ver>-*.src.rpm
```

The spec unpacks both tars in `%prep` and writes the `.cargo/config.toml` and the
`app/.npmrc` line that stop Cargo and pnpm reaching for the network, so nothing else has
to be staged.

Users enable it with:

```bash
sudo dnf copr enable deftera/tennoscope && sudo dnf install tennoscope
```

## Notes

- `check()` mirrors the Arch recipe, including the one 16:10 OCR skip that the combined
  upstream traineddata requires (see [`../arch.md`](../arch.md)).
- tesseract is a `Recommends`, not a `Requires`, for the same reason it is an `optdepends`
  on Arch: the collection browser runs without OCR.
- xwininfo is a `Recommends` for the same shape of reason: only Wine's virtual-desktop mode
  reaches for it, because that mode nests the game where the normal window search cannot
  see it. The Fedora package is `xwininfo`; `xorg-x11-apps` was obsoleted by individual
  packages. A missing one is not a silent failure, it reads
  `Screen capture failed: xwininfo is not installed` on the Diagnostics `Reward observer` row.
- No workflow in `.github/workflows/` submits or rebuilds this spec, so a new release needs
  a human to run the sequence above. Webhook or Packit automation is worth it when the
  release cadence says so, not before.
