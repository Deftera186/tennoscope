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
