#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

# CI already ran the test suite, clippy and `pnpm check` against this very commit, and the release
# workflow refuses to build a tag whose CI did not pass -- so repeating them here costs about
# fifteen minutes to learn what is already known. `--skip-gates` is for that caller and no other:
# it drops only the checks CI duplicates, never the two that inspect the AppImage itself, because
# nothing but this script looks at those. Off by default, so running this by hand still gates.
skip_gates=false
if [ "${1-}" = "--skip-gates" ]; then
  skip_gates=true
  shift
fi

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

appimage_patch_tmp=
appimage_extract_tmp=
package_extract_tmp=
package_archive_tmp=
package_listing_tmp=
cleanup() {
  [ -z "$appimage_patch_tmp" ] || rm -f "$appimage_patch_tmp"
  [ -z "$appimage_extract_tmp" ] || rm -rf "$appimage_extract_tmp"
  [ -z "$package_extract_tmp" ] || rm -rf "$package_extract_tmp"
  [ -z "$package_archive_tmp" ] || rm -f "$package_archive_tmp"
  [ -z "$package_listing_tmp" ] || rm -f "$package_listing_tmp"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "$1 is required to inspect the requested bundle" >&2
    exit 127
  }
}

for bundle in "$@"; do
  case "$bundle" in
    appimage)
      require_command awk
      require_command mktemp
      ;;
    deb)
      require_command awk
      require_command dpkg-deb
      require_command mktemp
      ;;
    rpm)
      require_command awk
      require_command rpm
      require_command cpio
      require_command mktemp
      require_command rpm2cpio
      ;;
  esac
done

assert_exact_desktop_field() {
  desktop_file=$1
  field_prefix=$2
  expected_line=$3
  description=$4

  actual_line=$(desktop_field_line "$desktop_file" "$field_prefix") || {
    echo "$description must occur exactly once in $desktop_file" >&2
    exit 1
  }
  [ "$actual_line" = "$expected_line" ] || {
    echo "$description must be '$expected_line' in $desktop_file" >&2
    exit 1
  }
}

desktop_field_line() {
  desktop_file=$1
  field_prefix=$2

  awk -v prefix="$field_prefix" '
    index($0, prefix) == 1 { matches++; value = $0 }
    END {
      if (matches != 1) exit 1
      print value
    }
  ' "$desktop_file"
}

effective_root_desktop() {
  desktop_root=$1
  desktop_count=0
  desktop_path=

  for desktop_candidate in "$desktop_root"/*.desktop; do
    [ -f "$desktop_candidate" ] || continue
    desktop_count=$((desktop_count + 1))
    desktop_path=$desktop_candidate
  done

  [ "$desktop_count" -eq 1 ] || {
    echo "expected exactly one effective root desktop entry in $desktop_root, found $desktop_count" >&2
    exit 1
  }
  printf '%s\n' "$desktop_path"
}

single_bundle_artifact() {
  artifact_dir=$1
  artifact_extension=$2
  artifact_count=0
  artifact_path=

  for artifact_candidate in "$artifact_dir"/*."$artifact_extension"; do
    [ -f "$artifact_candidate" ] || continue
    artifact_count=$((artifact_count + 1))
    artifact_path=$artifact_candidate
  done

  [ "$artifact_count" -eq 1 ] || {
    echo "expected exactly one .$artifact_extension artifact in $artifact_dir, found $artifact_count" >&2
    exit 1
  }
  printf '%s\n' "$artifact_path"
}

assert_single_payload_desktop() {
  listing_file=$1
  payload_count=$(awk '
    {
      path = $NF
      sub(/^\.\//, "", path)
      sub(/^\//, "", path)
      if (path ~ /^usr\/share\/applications\/[^/]+\.desktop$/) matches++
    }
    END { print matches + 0 }
  ' "$listing_file")

  [ "$payload_count" -eq 1 ] || {
    echo "expected exactly one desktop entry in /usr/share/applications in the package payload, found $payload_count" >&2
    exit 1
  }
}

assert_deb_dependency() {
  package=$1
  expected=$2

  dpkg-deb -f "$package" Depends | awk -v expected="$expected" '
    BEGIN { RS = "," }
    {
      sub(/^[[:space:]]+/, "")
      split($0, fields, /[[:space:](]/)
      if (fields[1] == expected) found = 1
    }
    END { exit(found ? 0 : 1) }
  ' || {
    echo "the deb metadata must depend on $expected" >&2
    exit 1
  }
}

assert_rpm_dependency() {
  package=$1
  expected=$2

  rpm -qp --requires "$package" | awk -v expected="$expected" '
    $1 == expected { found = 1 }
    END { exit(found ? 0 : 1) }
  ' || {
    echo "the rpm metadata must depend on $expected" >&2
    exit 1
  }
}

assert_single_installed_desktop() {
  extract_root=$1
  expected_exec=$2
  installed_desktop_count=0
  installed_desktop=

  for desktop_candidate in "$extract_root"/usr/share/applications/*.desktop; do
    [ -f "$desktop_candidate" ] || continue
    installed_desktop_count=$((installed_desktop_count + 1))
    installed_desktop=$desktop_candidate
  done

  [ "$installed_desktop_count" -eq 1 ] || {
    echo "expected exactly one installed desktop entry, found $installed_desktop_count" >&2
    exit 1
  }
  assert_authorized_desktop "$installed_desktop" "$expected_exec"
}

assert_authorized_desktop() {
  desktop_file=$1
  expected_exec=$2

  [ -f "$desktop_file" ] || {
    echo "desktop entry was not found at $desktop_file" >&2
    exit 1
  }
  assert_exact_desktop_field "$desktop_file" \
    "X-KDE-DBUS-Restricted-Interfaces=" \
    "X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2" \
    "the KWin restricted-interface key"
  assert_exact_desktop_field "$desktop_file" "Exec=" "$expected_exec" \
    "the desktop Exec field"
  assert_exact_desktop_field "$desktop_file" "StartupWMClass=" \
    "StartupWMClass=tennoscope" "the desktop StartupWMClass field"
}

assert_appimage_runs_on_x11() {
  appdir="$repo_root/target/release/bundle/appimage/TennoScope.AppDir"
  hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"

  [ -f "$hook" ] || { echo "generated AppImage GTK hook was not found" >&2; exit 1; }

  # The overlay has to run on X11 to sit above the game, so upstream's own
  # `GDK_BACKEND=x11` is what we want -- but the env var overrides the request
  # the app makes for itself, so a future plugin release that drops or changes
  # it would silently take the overlay with it.
  awk '$0 ~ /^export GDK_BACKEND=x11 / { found = 1 } END { exit(found ? 0 : 1) }' "$hook" || {
    echo "the AppImage GTK hook no longer forces X11; the reward overlay needs it" >&2
    exit 1
  }
}

# linuxdeploy bundles whatever the build host linked against, filtered by an
# excludelist compiled into it. Tauri pins a 2024 linuxdeploy, and upstream
# added libwayland-client.so.0 to that list after it was built. The host's Mesa
# EGL vendor cannot use that older bundled copy, so remove it before the one
# repack operation. The repack is still required when the library is absent:
# it also removes the KWin permission claim that an AppImage cannot satisfy.
patch_and_repack_appimage() {
  bundle_dir="$repo_root/target/release/bundle/appimage"
  appdir="$bundle_dir/TennoScope.AppDir"
  cache="${XDG_CACHE_HOME:-$HOME/.cache}/tauri"
  packer="$cache/linuxdeploy-plugin-appimage.AppImage"
  built=$(single_bundle_artifact "$bundle_dir" AppImage)
  desktop=$(effective_root_desktop "$appdir")

  [ -x "$packer" ] || {
    echo "linuxdeploy's AppImage plugin was not found or executable at $packer" >&2
    exit 1
  }
  appimage_exec_line=$(desktop_field_line "$desktop" "Exec=") || {
    echo "the generated AppImage desktop entry must have exactly one Exec field" >&2
    exit 1
  }
  appimage_icon_line=$(desktop_field_line "$desktop" "Icon=") || {
    echo "the generated AppImage desktop entry must have exactly one Icon field" >&2
    exit 1
  }

  appimage_patch_tmp=$(mktemp "${TMPDIR:-/tmp}/tennoscope-desktop.XXXXXX") || {
    echo "could not create temporary storage for the AppImage desktop entry" >&2
    exit 1
  }
  awk '
    $0 !~ /^X-KDE-DBUS-Restricted-Interfaces=/ { print }
  ' "$desktop" >"$appimage_patch_tmp" || {
    echo "could not apply the AppImage desktop policy" >&2
    exit 1
  }
  cat "$appimage_patch_tmp" >"$desktop" || {
    echo "could not replace the AppImage desktop entry" >&2
    exit 1
  }
  rm -f "$appimage_patch_tmp"
  appimage_patch_tmp=

  if desktop_field_line "$desktop" "X-KDE-DBUS-Restricted-Interfaces=" >/dev/null; then
    echo "the AppImage desktop entry must not claim KWin screenshot permission" >&2
    exit 1
  fi
  assert_exact_desktop_field "$desktop" "Exec=" "$appimage_exec_line" \
    "the generated AppImage launch command"
  assert_exact_desktop_field "$desktop" "Icon=" "$appimage_icon_line" \
    "the generated AppImage icon"

  rm -f "$appdir/usr/lib/libwayland-client.so.0"
  (cd "$bundle_dir" && APPIMAGE_EXTRACT_AND_RUN=1 OUTPUT="$built" \
    "$packer" --appdir "$appdir") || {
    echo "failed to repack the patched AppImage" >&2
    exit 1
  }

  [ -x "$built" ] || {
    echo "the repacked AppImage is not executable: $built" >&2
    exit 1
  }
  appimage_extract_tmp=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-appimage.XXXXXX") || {
    echo "could not create temporary storage to extract the final AppImage" >&2
    exit 1
  }
  (cd "$appimage_extract_tmp" && "$built" --appimage-extract >/dev/null) || {
    echo "failed to extract the final AppImage" >&2
    exit 1
  }
  extracted_desktop=$(effective_root_desktop "$appimage_extract_tmp/squashfs-root")
  if desktop_field_line "$extracted_desktop" "X-KDE-DBUS-Restricted-Interfaces=" >/dev/null; then
    echo "the final AppImage desktop entry must not claim KWin screenshot permission" >&2
    exit 1
  fi
  assert_exact_desktop_field "$extracted_desktop" "Exec=" "$appimage_exec_line" \
    "the final AppImage launch command"
  assert_exact_desktop_field "$extracted_desktop" "Icon=" "$appimage_icon_line" \
    "the final AppImage icon"
  rm -rf "$appimage_extract_tmp"
  appimage_extract_tmp=
}

assert_deb_artifact() {
  bundle_dir="$repo_root/target/release/bundle/deb"
  built=$(single_bundle_artifact "$bundle_dir" deb)
  package_listing_tmp=$(mktemp "${TMPDIR:-/tmp}/tennoscope-deb-list.XXXXXX") || {
    echo "could not create temporary storage for the deb payload listing" >&2
    exit 1
  }
  dpkg-deb -c "$built" >"$package_listing_tmp" || {
    echo "failed to list deb artifact $built" >&2
    exit 1
  }
  assert_single_payload_desktop "$package_listing_tmp"
  rm -f "$package_listing_tmp"
  package_listing_tmp=
  package_extract_tmp=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-deb.XXXXXX") || {
    echo "could not create temporary storage to extract the deb" >&2
    exit 1
  }
  dpkg-deb -x "$built" "$package_extract_tmp" || {
    echo "failed to extract deb artifact $built" >&2
    exit 1
  }
  assert_single_installed_desktop "$package_extract_tmp" "Exec=/usr/bin/tennoscope"
  assert_deb_dependency "$built" "libpipewire-0.3-0"
  rm -rf "$package_extract_tmp"
  package_extract_tmp=
}

assert_rpm_artifact() {
  bundle_dir="$repo_root/target/release/bundle/rpm"
  built=$(single_bundle_artifact "$bundle_dir" rpm)
  package_archive_tmp=$(mktemp "${TMPDIR:-/tmp}/tennoscope-rpm.XXXXXX.cpio") || {
    echo "could not create temporary storage for the rpm payload" >&2
    exit 1
  }
  rpm2cpio "$built" >"$package_archive_tmp" || {
    echo "failed to read rpm artifact $built" >&2
    exit 1
  }
  package_listing_tmp=$(mktemp "${TMPDIR:-/tmp}/tennoscope-rpm-list.XXXXXX") || {
    echo "could not create temporary storage for the rpm payload listing" >&2
    exit 1
  }
  cpio -it --quiet <"$package_archive_tmp" >"$package_listing_tmp" || {
    echo "failed to list rpm artifact $built" >&2
    exit 1
  }
  assert_single_payload_desktop "$package_listing_tmp"
  rm -f "$package_listing_tmp"
  package_listing_tmp=
  package_extract_tmp=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-rpm.XXXXXX") || {
    echo "could not create temporary storage to extract the rpm" >&2
    exit 1
  }
  (cd "$package_extract_tmp" && cpio -id --quiet <"$package_archive_tmp") || {
    echo "failed to extract rpm artifact $built" >&2
    exit 1
  }
  assert_single_installed_desktop "$package_extract_tmp" "Exec=/usr/bin/tennoscope"
  assert_rpm_dependency "$built" "pipewire-libs"
  rm -rf "$package_extract_tmp"
  package_extract_tmp=
  rm -f "$package_archive_tmp"
  package_archive_tmp=
}

cd "$repo_root"
if [ "$skip_gates" = false ]; then
  cargo test --workspace
  cargo clippy --workspace --all-targets -- -D warnings
  ( cd "$repo_root/app" && pnpm check )
fi

cd "$repo_root/app"

for bundle in "$@"; do
  if [ "$bundle" = appimage ]; then
    # linuxdeploy's bundled strip may not understand newer RELR sections
    # emitted by rolling-release distributions. Skipping this optional size
    # optimization keeps the build portable and does not alter the binary.
    NO_STRIP=${NO_STRIP:-true} pnpm tauri build --bundles "$bundle"
    assert_appimage_runs_on_x11
    patch_and_repack_appimage
  else
    pnpm tauri build --bundles "$bundle"
    case "$bundle" in
      deb) assert_deb_artifact ;;
      rpm) assert_rpm_artifact ;;
    esac
  fi
done
