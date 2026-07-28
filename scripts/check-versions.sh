#!/bin/sh
# The release version is written in four files that nothing else forces to agree. A mismatch is
# not loud: tauri names the bundles from tauri.conf.json, so a stale package.json or PKGBUILD
# produces artifacts that look right and are labelled wrong. This is the thing that notices.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

# First match in each file is the declaration; everything after it is a dependency's version.
workspace=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
tauri=$(sed -n 's/.*"version": *"\(.*\)".*/\1/p' app/src-tauri/tauri.conf.json | head -1)
package=$(sed -n 's/.*"version": *"\(.*\)".*/\1/p' app/package.json | head -1)
pkgbuild=$(sed -n 's/^pkgver=\(.*\)$/\1/p' packaging/arch/PKGBUILD | head -1)

for pair in "Cargo.toml:$workspace" "tauri.conf.json:$tauri" \
  "app/package.json:$package" "PKGBUILD:$pkgbuild"; do
  case "$pair" in
    *:) echo "no version found in ${pair%:}" >&2; exit 1 ;;
  esac
done

if [ "$workspace" = "$tauri" ] && [ "$workspace" = "$package" ] && [ "$workspace" = "$pkgbuild" ]; then
  echo "version $workspace agrees across all four declarations"
  exit 0
fi

echo "release versions disagree:" >&2
echo "  Cargo.toml         $workspace" >&2
echo "  tauri.conf.json    $tauri" >&2
echo "  app/package.json   $package" >&2
echo "  PKGBUILD           $pkgbuild" >&2
exit 1
