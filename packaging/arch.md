# Arch Linux package

[`arch/PKGBUILD`](arch/PKGBUILD) builds and installs a native package from the release tarball. No
AUR package or binary repository is published.

```bash
sudo pacman -S --needed base-devel cargo nodejs pnpm webkit2gtk-4.1
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch/PKGBUILD
makepkg -si
```

`source` points at the tag's GitHub archive, so `makepkg` fetches it. `sha256sums` is `SKIP` — this
recipe is not distributed through a package repository, and the tarball arrives over HTTPS from the
same place the `PKGBUILD` did. If you are repackaging this for anyone but yourself, replace it with
a real digest:

```bash
updpkgsums
```

The recipe builds the locked Rust workspace and frontend, runs both test suites, and installs
`tennoscope`, its desktop entry, icon, GPLv3 license and third-party notices. Dependency resolution
needs network access, so `makepkg` will not work in an offline chroot without vendored sources.

The relic overlay's toolchain is in `optdepends`, not `depends` — the collection browser runs
without it. `check()` does need it, so `tesseract` is in `checkdepends`; skip
that step with `makepkg --nocheck` if you would rather not pull it in to build.

Before any AUR submission: use a literal `sha256sums` digest, and add a `.SRCINFO`.
