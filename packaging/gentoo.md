# Gentoo local ebuild

[`gentoo/warframe-helper-0.1.0.ebuild`](gentoo/warframe-helper-0.1.0.ebuild) is usable in a local overlay. It is not published in the Gentoo repository and intentionally consumes a local source archive from `DISTDIR` until this project has an immutable release URL.

Enable Corepack for Node.js and install the native prerequisites:

```bash
echo 'net-libs/nodejs corepack' | sudo tee -a /etc/portage/package.use/warframe-helper
sudo emerge --ask virtual/rust net-libs/nodejs net-libs/webkit-gtk:4.1
```

Create the release-shaped archive from a clean checkout and copy it to the configured distfiles directory (commonly `/var/cache/distfiles`):

```bash
git archive --format=tar.gz --prefix=warframe-helper-0.1.0/ \
  --output=/tmp/warframe-helper-0.1.0.tar.gz HEAD
sha256sum /tmp/warframe-helper-0.1.0.tar.gz
sudo install -m644 /tmp/warframe-helper-0.1.0.tar.gz /var/cache/distfiles/
```

Copy `packaging/gentoo/` into an initialized local repository as `app-misc/warframe-helper/`, generate its Manifest, and install it:

```bash
sudo ebuild /var/db/repos/local/app-misc/warframe-helper/warframe-helper-0.1.0.ebuild manifest
sudo FEATURES="-network-sandbox" emerge --ask app-misc/warframe-helper
```

The one-command `FEATURES` override is necessary because this local ebuild resolves the Cargo and pnpm lockfiles during the build; it affects only that invocation. The ebuild installs the canonical binary, desktop entry, icon, GPLv3 license, and third-party notice. Before submitting it to a public overlay, add a real `HOMEPAGE`, replace the local `src_unpack` path with an immutable `SRC_URI`, enumerate/vend all Rust and pnpm sources, and let the Manifest carry the release archive checksum.
