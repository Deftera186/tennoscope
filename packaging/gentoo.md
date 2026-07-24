# Gentoo packaging guidance

There is no official Gentoo overlay or ebuild yet. The AppImage can be used across distributions, while Gentoo users who prefer native compilation can build from source.

Likely native build/runtime requirements include:

- `net-libs/webkit-gtk:4.1`
- `gnome-base/librsvg`
- OpenSSL, GTK 3, and the standard Tauri Linux toolchain
- Rust 1.85 or newer
- a Node.js version accepted by `app/package.json`
- pnpm 10/Corepack

Exact atoms and USE flags depend on the active Gentoo profile and should be validated with `pkgdev` before publication.

## Future ebuild shape

A release ebuild should:

1. use an immutable release archive with a populated Manifest rather than a moving branch;
2. use `cargo.eclass` with the locked crate set, and a Gentoo-compliant strategy for locked pnpm dependencies;
3. declare WebKitGTK 4.1 and all actual linked libraries in `RDEPEND`/`DEPEND`;
4. run frontend checks and the release build without network access in compile phases;
5. install the binary, desktop file, icons, GPLv3 license reference, and `THIRD_PARTY_NOTICES.md`;
6. preserve WFCD runtime-data attribution; and
7. avoid setuid installation and file capabilities for ptrace access.

Because the repository does not yet publish release tarballs or a vendored pnpm dependency set, a policy-compliant ebuild would be premature. These are concrete maintainer requirements, not a claim of current Portage availability.
