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

# linuxdeploy bundles whatever the build host linked against, filtered by an
# excludelist compiled into it. Tauri pins a 2024 linuxdeploy, and upstream
# added libwayland-client.so.0 to that list after it was built:
#
#   /usr/lib64/libEGL_mesa.so.0: undefined symbol: wl_fixes_interface (fatal)
#
# The host's Mesa EGL vendor is loaded against our older bundled copy, fails
# that lookup, leaves libglvnd with no vendor, and WebKit aborts on
# EGL_BAD_PARAMETER with a blank window. Drop the library so the host supplies
# it -- every system that can run a GTK application already has one -- and
# repack the AppDir the plugin left behind.
drop_bundled_wayland_client() {
  bundle_dir="$repo_root/target/release/bundle/appimage"
  appdir="$bundle_dir/TennoScope.AppDir"
  cache="${XDG_CACHE_HOME:-$HOME/.cache}/tauri"
  packer="$cache/linuxdeploy-plugin-appimage.AppImage"

  [ -f "$appdir/usr/lib/libwayland-client.so.0" ] || return 0
  [ -x "$packer" ] || { echo "linuxdeploy's AppImage plugin was not found in $cache" >&2; exit 1; }

  built=$(ls -t "$bundle_dir"/*.AppImage 2>/dev/null | head -1)
  [ -n "$built" ] || { echo "no AppImage was produced to repack" >&2; exit 1; }

  rm -f "$appdir/usr/lib/libwayland-client.so.0"
  ( cd "$bundle_dir" && APPIMAGE_EXTRACT_AND_RUN=1 OUTPUT="$built" "$packer" --appdir "$appdir" )
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
    drop_bundled_wayland_client
  else
    pnpm tauri build --bundles "$bundle"
  fi
done
