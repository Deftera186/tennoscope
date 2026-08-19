# TennoScope desktop UI

This directory contains the Tauri 2, React, TypeScript, and Vite desktop application. Project setup, run commands, privacy behavior, limitations, and packaging instructions are documented in the [root README](../README.md).

Common commands:

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm tauri dev
```

Build Linux bundles through the repository-root helper rather than `pnpm tauri build`, which skips
two AppImage post-processing steps:

```bash
../scripts/build-linux-bundles.sh appimage
```
