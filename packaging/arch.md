# Arch Linux package

[`arch/PKGBUILD`](arch/PKGBUILD) builds and installs a native package from the release tarball. It
works on any Arch-based distribution — Arch, Manjaro, EndeavourOS, CachyOS — since it only needs
`pacman` and `makepkg`. No AUR package or binary repository is published.

```bash
sudo pacman -S --needed base-devel
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch/PKGBUILD
makepkg -si
```

`makepkg -s` installs the `makedepends` and `checkdepends` itself, so they are not listed above;
`base-devel` is the one thing it assumes you already have.

## AUR helpers

`yay -S tennoscope` and `paru -S tennoscope` cannot work: helpers install *from* the AUR, and
nothing is published there. Both can drive a local `PKGBUILD` instead, with `-B` pointed at the
directory holding it:

```bash
paru -B .    # or: yay -B .
```

`yay -B` runs `git reset`/`git merge` against that directory first, so it wants a git checkout with
a remote and fails in a bare directory holding only a downloaded `PKGBUILD`. `paru -B` and plain
`makepkg -si` do not care. Prefer `makepkg -si` unless you specifically want a helper to track it.

## The recipe

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

## Three things this recipe has to do that are not obvious

None of them show up on a developer machine that already runs a desktop, which is why all three
only surfaced in a clean container.

**`libpipewire` and `clang` in `makedepends`.** Neither is pulled in by anything in `depends`, and
without them `build()` dies in `libspa-sys`' build script with `Package 'libpipewire-0.3' was not
found`. They are `xcap`'s: its Linux capture path depends on pipewire unconditionally for the
portal route and generates its bindings with bindgen. `libglvnd` is there for `egl.pc`.

**`options=('!lto')`.** `makepkg.conf` ships `lto` in the default `OPTIONS`, which puts
`-flto=auto` in `CFLAGS` — including for the C that `rusqlite` bundles. GCC then emits `.gnu.lto_*`
IR instead of machine code, and `rustc` links with `ld.lld`, which cannot read those sections: the
link fails with ~20 undefined `sqlite3_*` symbols. Rust's own LTO comes from the Cargo profile, not
`CFLAGS`, so nothing is lost. The Gentoo ebuild filters the same flag for the same reason.

**One test is skipped in `check()`.** `a_16_10_screen_is_read_where_a_16_10_screen_actually_sits`
asserts every card reads at >= 0.9. `tesseract-data-eng` ships upstream's combined legacy+LSTM
`tessdata` (23MB), and on that fixture `2X Forma Blueprint` reads 0.875; Gentoo's `tessdata_fast`
(4MB) and CI's both clear the floor. That is a traineddata difference, not misplaced geometry, and
0.875 is still well above the 0.6 the reader actually publishes at — so the card reads normally on
Arch. Skipped rather than loosened, because that floor is what proves the crop geometry everywhere
else.

Before any AUR submission: use a literal `sha256sums` digest, and add a `.SRCINFO`.
