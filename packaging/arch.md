# Arch Linux packages

Two recipes for any Arch-based distribution (Arch, Manjaro, EndeavourOS, CachyOS),
since both only need `pacman` and `makepkg`. No AUR package or binary repository is
published.

[`arch-bin/PKGBUILD`](arch-bin/PKGBUILD) (`tennoscope-bin`) repacks the release
`.deb` and installs in seconds with no toolchain. This is the recommended route
for anyone who just wants to run it:

```bash
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch-bin/PKGBUILD
makepkg -si
```

[`arch/PKGBUILD`](arch/PKGBUILD) (`tennoscope`) builds the native package from
the release tarball instead. It needs the full toolchain and compiles the whole
Rust workspace, so it takes a while:

```bash
sudo pacman -S --needed base-devel
curl -O https://raw.githubusercontent.com/Deftera186/tennoscope/main/packaging/arch/PKGBUILD
makepkg -si
```

Both commands above fetch a recipe from `main`, and `main` carries the recipes bumped to the
version being prepared, ahead of the tag and the release asset they name. They resolve once
that release is published; between releases they do not build, and the [AppImage](appimage.md)
has no such wait.

The two packages conflict (`provides`/`conflicts`), so install one or the other.
`makepkg -s` installs the source recipe's `makedepends` and `checkdepends`
itself, so they are not listed above; `base-devel` is the one thing it assumes
you already have. The `-bin` recipe has no build-time dependencies at all.

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

## The recipes

Both recipes carry a `pkgver` that has to match the release and a `sha256sums` that has to
match what that version publishes, and the two move at different times. `main` is bumped to the
version being prepared before that version exists anywhere, so a release-prep commit leaves a
stale digest on purpose. `RELEASING.md` step 8 re-pins each digest with `updpkgsums` from the
published artifact, and it can only do that once the tag is pushed and the release workflow has
attached the `.deb`, which is the first moment either file is fetchable at all.
`./scripts/check-arch-digests.sh` is what CI runs to catch a pin that never got refreshed; inside
that window it warns rather than fails, because the release gate needs CI green before it can
build the `.deb` in the first place.

The source recipe's `source` points at the `v${pkgver}` GitHub archive, so `makepkg` fetches
it, and its `sha256sums` is that tarball's digest. Repackaging a different commit means
pointing `source` at that commit and replacing the digest with a fresh one.

The source recipe builds the locked Rust workspace and frontend, runs both test suites, and installs
`tennoscope`, its desktop entry, icon, GPLv3 license and third-party notices. Dependency resolution
needs network access, so `makepkg` will not work in an offline chroot without vendored sources.

The `-bin` recipe's `source` points at `TennoScope_${pkgver}_amd64.deb` on the GitHub
release instead, and its `sha256sums` is that `.deb`'s digest, re-pinned the same way and
only once the release has attached it. There is no `build()` or `check()`: `package()`
unpacks the deb data archive with `bsdtar`, which ships with `pacman`, and links the Arch
license path at the license the `.deb` already carries under `/usr/lib/TennoScope`.

The relic overlay's OCR toolchain is in `optdepends`, not `depends`, because the collection
browser runs without it. `xorg-xwininfo` sits there for the same reason: only Wine's
virtual-desktop mode reaches it, so the recipe recommends it rather than requiring it, matching
`Recommends` on deb and rpm.

`check()` does need tesseract, so `tesseract` and `tesseract-data-eng` are in `checkdepends`; skip
that step with `makepkg --nocheck` if you would rather not pull them in to build.

## Three things the source recipe has to do that are not obvious

None of them show up on a developer machine that already runs a desktop, which is why all three
only surfaced in a clean container.

**`libpipewire` and `clang` in `makedepends`.** A clean chroot has neither of them before
`build()` runs, and without them the build dies in `libspa-sys`'s build script with
`Package 'libpipewire-0.3' was not found`. They are `xcap`'s: its Linux capture path depends on
pipewire unconditionally for the portal route and generates its bindings with bindgen.
`libglvnd` is there for `egl.pc`.

**`options=('!lto')`.** `makepkg.conf` ships `lto` in the default `OPTIONS`, which puts
`-flto=auto` in `CFLAGS`, including for the C that `rusqlite` bundles. GCC then emits `.gnu.lto_*`
IR instead of machine code, and `rustc` links with `ld.lld`, which cannot read those sections: the
link fails with ~20 undefined `sqlite3_*` symbols. Rust's own LTO comes from the Cargo profile, not
`CFLAGS`, so nothing is lost. The Gentoo ebuild filters the same flag for the same reason.

**One test is skipped in `check()`.** `a_16_10_screen_is_read_where_a_16_10_screen_actually_sits`
asserts every card reads at >= 0.9. `tesseract-data-eng` ships upstream's combined legacy+LSTM
`tessdata` (23MB), and on that fixture `2X Forma Blueprint` reads 0.875; Gentoo's `tessdata_fast`
(4MB) and CI's both clear the floor. That is a traineddata difference, not misplaced geometry, and
0.875 is still well above the 0.6 the reader actually publishes at, so the card reads normally on
Arch. Skipped rather than loosened, because that floor is what proves the crop geometry everywhere
else.

Before any AUR submission: add a `.SRCINFO` per recipe (`makepkg --printsrcinfo`
in each directory, needs an Arch box).
