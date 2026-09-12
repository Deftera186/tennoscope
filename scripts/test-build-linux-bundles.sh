#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-bundle-test.XXXXXX")
trap 'rm -rf "$test_root"' 0
trap 'exit 1' HUP INT TERM

fixture_root="$test_root/repo"
bin_dir="$test_root/bin"
mkdir -p "$fixture_root/scripts" "$fixture_root/app" \
  "$fixture_root/target/release/bundle/rpm" "$bin_dir"
cp "$repo_root/scripts/build-linux-bundles.sh" "$fixture_root/scripts/"
: >"$fixture_root/target/release/bundle/rpm/TennoScope-test.x86_64.rpm"

cat >"$bin_dir/cargo" <<'EOF'
#!/bin/sh
exit 0
EOF
cat >"$bin_dir/pnpm" <<'EOF'
#!/bin/sh
exit 0
EOF
cat >"$bin_dir/rpm" <<'EOF'
#!/bin/sh
printf '%s\n' pipewire-libs
EOF
cat >"$bin_dir/rpm2cpio" <<'EOF'
#!/bin/sh
echo 'rpm2cpio must not be used' >&2
exit 99
EOF
cat >"$bin_dir/bsdtar" <<'EOF'
#!/bin/sh
case "$1" in
  -tf)
    cat <<'LISTING'
./usr/bin/tennoscope
./usr/share/applications/TennoScope.desktop
LISTING
    ;;
  -xf)
    while [ "$#" -gt 0 ]; do
      if [ "$1" = -C ]; then
        extract_root=$2
        break
      fi
      shift
    done
    mkdir -p "$extract_root/usr/share/applications"
    cat >"$extract_root/usr/share/applications/TennoScope.desktop" <<'DESKTOP'
[Desktop Entry]
Exec=/usr/bin/tennoscope
StartupWMClass=tennoscope
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
DESKTOP
    ;;
  *)
    echo "unexpected bsdtar arguments: $*" >&2
    exit 2
    ;;
esac
EOF
chmod +x "$bin_dir"/*

PATH="$bin_dir:$PATH" "$fixture_root/scripts/build-linux-bundles.sh" --skip-gates rpm

appimage_dir="$fixture_root/target/release/bundle/appimage"
appdir="$appimage_dir/TennoScope.AppDir"
cache_dir="$test_root/cache/tauri"
mkdir -p "$appdir/apprun-hooks" "$appdir/usr/share/applications" \
  "$appdir/usr/lib" "$cache_dir"

cat >"$appdir/usr/share/applications/TennoScope.desktop" <<'DESKTOP'
[Desktop Entry]
Exec=/usr/bin/tennoscope
Icon=tennoscope
StartupWMClass=tennoscope
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
DESKTOP
ln -s usr/share/applications/TennoScope.desktop "$appdir/TennoScope.desktop"
cat >"$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh" <<'HOOK'
export GDK_BACKEND=x11 # required by the reward overlay
HOOK
: >"$appdir/usr/lib/libwayland-client.so.0"

cat >"$appimage_dir/TennoScope-test.AppImage" <<EOF
#!/bin/sh
if [ "\${1-}" != --appimage-extract ]; then
  echo "unexpected AppImage arguments: \$*" >&2
  exit 2
fi
snapshot=\$0.contents
[ -d "\$snapshot" ] || { echo "AppImage was not repacked" >&2; exit 1; }
mkdir -p squashfs-root
cp -R "\$snapshot"/. squashfs-root/
EOF
cat >"$cache_dir/linuxdeploy-plugin-appimage.AppImage" <<'EOF'
#!/bin/sh
if [ "${1-}" != --appdir ] || [ "$#" -ne 2 ]; then
  echo "unexpected appimage packer arguments: $*" >&2
  exit 2
fi
snapshot=$OUTPUT.contents
rm -rf "$snapshot"
mkdir -p "$snapshot"
cp -R "$2"/. "$snapshot"/
EOF
chmod +x "$appimage_dir/TennoScope-test.AppImage" \
  "$cache_dir/linuxdeploy-plugin-appimage.AppImage"

XDG_CACHE_HOME="$test_root/cache" PATH="$bin_dir:$PATH" \
  "$fixture_root/scripts/build-linux-bundles.sh" --skip-gates appimage

appimage_exec=$(awk '/^Exec=/ { matches++; value = $0 } END { if (matches != 1) exit 1; print value }' \
  "$appdir/TennoScope.desktop")
[ "$appimage_exec" = "Exec=tennoscope" ] || {
  echo "AppImage must launch its bundled executable, found '$appimage_exec'" >&2
  exit 1
}

# The fake packer snapshots the AppDir into artifact-owned data. Corrupt staging after the build
# so extracting the artifact cannot accidentally reread the same tree that post-processing edited.
cat >"$appdir/usr/share/applications/TennoScope.desktop" <<'DESKTOP'
[Desktop Entry]
Exec=/usr/bin/tennoscope
Icon=tennoscope
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
DESKTOP
artifact_extract="$test_root/final-artifact"
mkdir -p "$artifact_extract"
(cd "$artifact_extract" && "$appimage_dir/TennoScope-test.AppImage" --appimage-extract)
artifact_exec=$(awk '/^Exec=/ { matches++; value = $0 } END { if (matches != 1) exit 1; print value }' \
  "$artifact_extract/squashfs-root/TennoScope.desktop")
[ "$artifact_exec" = "Exec=tennoscope" ] || {
  echo "repacked AppImage must contain its bundled launch command, found '$artifact_exec'" >&2
  exit 1
}
