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

assert_appimage_runs_on_x11() {
  appdir="$repo_root/target/release/bundle/appimage/TennoScope.AppDir"
  hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"

  [ -f "$hook" ] || { echo "generated AppImage GTK hook was not found" >&2; exit 1; }

  # The overlay has to run on X11 to sit above the game, so upstream's own
  # `GDK_BACKEND=x11` is what we want -- but the env var overrides the request
  # the app makes for itself, so a future plugin release that drops or changes
  # it would silently take the overlay with it.
  grep -q '^export GDK_BACKEND=x11 ' "$hook" || {
    echo "the AppImage GTK hook no longer forces X11; the reward overlay needs it" >&2
    exit 1
  }
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
    assert_appimage_runs_on_x11
  else
    pnpm tauri build --bundles "$bundle"
  fi
done
