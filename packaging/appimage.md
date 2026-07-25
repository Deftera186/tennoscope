# AppImage

AppImage is the recommended cross-distribution artifact for early releases.

Build it from the repository root with:

```bash
./scripts/build-linux-bundles.sh appimage
```

The artifact is written under `target/release/bundle/appimage/`. Run it as the same Unix user that runs Warframe:

```bash
chmod +x "TennoScope_0.1.0_amd64.AppImage"
./"TennoScope_0.1.0_amd64.AppImage"
```

Some distributions no longer install FUSE 2 compatibility by default. Prefer installing the distribution's FUSE 2 compatibility package. For a one-off fallback, AppImage supports extraction-and-run mode:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./"TennoScope_0.1.0_amd64.AppImage"
```

The AppImage does not bypass `/proc` or Yama restrictions, does not contain Warframe, and should never be run as root or made setuid. The first catalog download still requires network access.

Tauri may download its AppImage packaging tools during the build. This is a build-time operation; release builders should archive checksums and build logs when publishing artifacts.

The repository helper sets `NO_STRIP=true` for AppImage assembly. The `linuxdeploy` binary currently used by Tauri contains an older `strip` that cannot read the newer ELF RELR sections found on some rolling-release systems, including current Gentoo installations. Skipping this optional packaging-time strip step produces a larger artifact but preserves the already optimized Rust executable and allows the bundle to complete.

The helper also repairs the GTK plugin's generated launcher before the final AppImage is assembled. Upstream currently forces `GDK_BACKEND=x11`; TennoScope changes that generated value to `wayland,x11` so GTK layer-shell can provide the reward overlay on Wayland while collection-only X11 sessions retain a fallback. Build AppImages through the helper rather than invoking `pnpm tauri build --bundles appimage` directly.
