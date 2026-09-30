# TennoScope desktop UI

This directory holds the Tauri 2 shell and the React, TypeScript and Vite front end. The
Rust workspace it sits in, the install routes, the access modes and the known limits are
documented in the [root README](../README.md) and the [install guide](../docs/install.md).

Common commands, all run from here:

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm tauri dev
```

`pnpm check` runs oxlint, `tsc --noEmit`, the vitest suite and `scripts/tauri-env.test.mjs`.
`pnpm tauri` is `scripts/tauri.mjs`, which normalises the Linux toolchain environment.

Build Linux bundles through the repository helper rather than `pnpm tauri build`, which
skips two AppImage post-processing steps:

```bash
../scripts/build-linux-bundles.sh appimage
```

It accepts `appimage`, `deb` and `rpm`, defaults to `appimage`, and writes to
`target/release/bundle/`. See [`packaging/`](../packaging) for what else it does.
