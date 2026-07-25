# Arch Linux package

[`arch/PKGBUILD`](arch/PKGBUILD) builds and installs a native package from a source archive. No AUR package or binary repository is claimed.

Install the build tools:

```bash
sudo pacman -S --needed base-devel cargo nodejs pnpm webkit2gtk-4.1
```

From a clean repository checkout, create the correctly rooted local source archive:

```bash
git archive --format=tar.gz --prefix=tennoscope-0.1.0/ \
  --output=packaging/arch/tennoscope-0.1.0.tar.gz HEAD
cd packaging/arch
export WARFRAME_HELPER_SHA256="$(sha256sum tennoscope-0.1.0.tar.gz | cut -d ' ' -f 1)"
makepkg -si
```

The default `source` is that adjacent archive. A release maintainer can instead set `WARFRAME_HELPER_SOURCE` to an immutable release URL or absolute archive path and set `WARFRAME_HELPER_SHA256` to its real digest before invoking `makepkg`. Add the real project homepage to `url` when a public release location exists.

The recipe builds the locked Rust workspace and frontend, runs both test suites, and installs `tennoscope`, its desktop entry, icon, GPLv3 license, and third-party notices. Dependency resolution currently needs network access. Before AUR submission, publish immutable source archives and use a literal URL and checksum rather than environment variables or `SKIP`.
