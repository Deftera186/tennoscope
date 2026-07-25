#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

if [ "$#" -eq 0 ]; then
  set -- appimage
fi

for bundle in "$@"; do
  case "$bundle" in
    appimage|deb|rpm) ;;
    *)
      echo "unsupported bundle '$bundle' (expected appimage, deb, or rpm)" >&2
      exit 2
      ;;
  esac
done

command -v cargo >/dev/null 2>&1 || { echo "cargo is required" >&2; exit 127; }
command -v pnpm >/dev/null 2>&1 || { echo "pnpm is required" >&2; exit 127; }

repair_appimage_backend() {
  appdir="$repo_root/target/release/bundle/appimage/TennoScope.AppDir"
  hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
  bundle_dir="$repo_root/target/release/bundle/appimage"
  plugin="${XDG_CACHE_HOME:-$HOME/.cache}/tauri/linuxdeploy-plugin-appimage.AppImage"

  [ -f "$hook" ] || { echo "generated AppImage GTK hook was not found" >&2; exit 1; }
  [ -x "$plugin" ] || { echo "Tauri AppImage plugin was not found at $plugin" >&2; exit 1; }

  # Tauri's GTK plugin currently forces X11, which prevents GTK layer-shell
  # from attaching the reward overlay on Wayland. Prefer Wayland while keeping
  # X11 as a fallback for collection-only sessions.
  sed -i 's/^export GDK_BACKEND=x11 .*/export GDK_BACKEND="${GDK_BACKEND:-wayland,x11}" # TennoScope: layer-shell requires Wayland/' "$hook"
  grep -q 'GDK_BACKEND="${GDK_BACKEND:-wayland,x11}"' "$hook" || {
    echo "could not repair the generated AppImage GTK backend hook" >&2
    exit 1
  }

  artifact=$(find "$bundle_dir" -maxdepth 1 -type f -name 'TennoScope_*.AppImage' -print -quit)
  [ -n "$artifact" ] || { echo "generated AppImage artifact was not found" >&2; exit 1; }
  replacement=$(mktemp "$bundle_dir/.TennoScope.XXXXXX.AppImage")
  APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="$replacement" "$plugin" --appdir "$appdir"
  chmod 755 "$replacement"
  mv "$replacement" "$artifact"
}

cd "$repo_root"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cd "$repo_root/app"
pnpm check

for bundle in "$@"; do
  if [ "$bundle" = appimage ]; then
    # linuxdeploy's bundled strip may not understand newer RELR sections
    # emitted by rolling-release distributions. Skipping this optional size
    # optimization keeps the build portable and does not alter the binary.
    NO_STRIP=${NO_STRIP:-true} pnpm tauri build --bundles "$bundle"
    repair_appimage_backend
  else
    pnpm tauri build --bundles "$bundle"
  fi
done
