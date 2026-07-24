# Arch Linux packaging guidance

There is no official AUR package yet. Until a release URL and checksum exist, users should build the AppImage or compile from this repository rather than install a pretend package.

Typical build prerequisites are:

```bash
sudo pacman -S --needed base-devel cargo curl file librsvg libxdo nodejs npm openssl webkit2gtk-4.1 wget
corepack enable
```

Depending on the desktop integration enabled by a future release, `libappindicator-gtk3` may also be required.

## Future PKGBUILD shape

A source-based `PKGBUILD` should:

1. use a signed release archive or immutable commit as `source`, with a real `sha256sums` value;
2. declare `webkit2gtk-4.1` and the generated bundle's actual shared-library requirements in `depends`;
3. declare the Rust, Node.js, pnpm/Corepack, and Tauri build dependencies in `makedepends`;
4. run `pnpm install --frozen-lockfile` and `pnpm build` without modifying lockfiles;
5. build with Cargo/Tauri under the package build user, never with `sudo`;
6. install the executable, desktop entry, icons, `LICENSE`, and `THIRD_PARTY_NOTICES.md` into normal Arch paths; and
7. avoid setuid bits and `cap_sys_ptrace` capabilities.

Arch packaging policy normally expects reproducible, non-interactive dependency acquisition. Before publishing a PKGBUILD, maintainers should decide whether to vendor Cargo/npm sources or use an approved prepare step. This repository intentionally does not ship a placeholder PKGBUILD with fake URLs or checksums.

For local development, follow the root README and run `pnpm tauri dev` from `app/`.
