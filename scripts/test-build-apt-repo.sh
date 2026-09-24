#!/bin/sh
# Fixture tests for scripts/build-apt-repo.sh: index contents, Release shape,
# idempotent re-runs, and the fail-closed signing rules.
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-apt-test.XXXXXX")
trap 'rm -rf "$test_root"' 0
trap 'exit 1' HUP INT TERM

pass=0
fail=0
check() {
  desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL: $desc" >&2
  fi
}

# Minimal real .deb: dpkg-scanpackages reads its control fields for real.
pkgdir="$test_root/pkg/DEBIAN"
mkdir -p "$pkgdir" "$test_root/pkg/usr/bin"
printf 'Package: tenno-scope\nVersion: 0.11.0\nArchitecture: amd64\nMaintainer: TennoScope\nDescription: fixture\n' >"$pkgdir/control"
printf '#!/bin/sh\necho hi\n' >"$test_root/pkg/usr/bin/tennoscope"
chmod 755 "$test_root/pkg/usr/bin/tennoscope"
dpkg-deb --build "$test_root/pkg" "$test_root/TennoScope_0.11.0_amd64.deb" >/dev/null

repo="$test_root/repo"
sh "$script_dir/build-apt-repo.sh" --repo "$repo" \
  --deb "$test_root/TennoScope_0.11.0_amd64.deb" --no-sign >/dev/null

check "pool holds the deb" test -f "$repo/pool/main/TennoScope_0.11.0_amd64.deb"
check "Packages lists the version" grep -q "Version: 0.11.0" "$repo/dists/stable/main/binary-amd64/Packages"
check "Filenames are repo-relative" grep -q "^Filename: pool/main/TennoScope_0.11.0_amd64.deb$" "$repo/dists/stable/main/binary-amd64/Packages"
check "Release pins SHA256" grep -q "SHA256:" "$repo/dists/stable/Release"
check "Release carries a Date header" grep -q "^Date: " "$repo/dists/stable/Release"
check "Release has no signature without key" sh -c "! test -f '$repo/dists/stable/InRelease'"

# Second release accumulates: both versions stay installable.
pkgdir2="$test_root/pkg2/DEBIAN"
mkdir -p "$pkgdir2" "$test_root/pkg2/usr/bin"
printf 'Package: tenno-scope\nVersion: 0.12.0\nArchitecture: amd64\nMaintainer: TennoScope\nDescription: fixture\n' >"$pkgdir2/control"
printf '#!/bin/sh\necho hi\n' >"$test_root/pkg2/usr/bin/tennoscope"
chmod 755 "$test_root/pkg2/usr/bin/tennoscope"
dpkg-deb --build "$test_root/pkg2" "$test_root/TennoScope_0.12.0_amd64.deb" >/dev/null
sh "$script_dir/build-apt-repo.sh" --repo "$repo" \
  --deb "$test_root/TennoScope_0.12.0_amd64.deb" --no-sign >/dev/null
check "old version retained" grep -q "Version: 0.11.0" "$repo/dists/stable/main/binary-amd64/Packages"
check "new version added" grep -q "Version: 0.12.0" "$repo/dists/stable/main/binary-amd64/Packages"

check "non-deb rejected" sh -c "! sh '$script_dir/build-apt-repo.sh' --repo '$repo' --deb '$test_root/pkg/usr/bin/tennoscope' --no-sign >/dev/null 2>&1"
check "signing without key fails closed" sh -c "! sh '$script_dir/build-apt-repo.sh' --repo '$repo' --deb '$test_root/TennoScope_0.12.0_amd64.deb' >/dev/null 2>&1"

echo "apt tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
