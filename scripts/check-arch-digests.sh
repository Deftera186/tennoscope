#!/bin/sh
# The Arch PKGBUILDs pin their sources with sha256sums, but release-prep commits bump pkgver
# without re-pinning, and nothing else checks the digests: check-versions.sh only checks that
# the versions agree with each other, not that the pins match what upstream serves. A stale
# pin is not loud either: makepkg reports a checksum failure that reads like a corrupt
# download rather than a wrong pin. This downloads what the PKGBUILDs point at and compares.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

arch_pkgbuild=packaging/arch/PKGBUILD
bin_pkgbuild=packaging/arch-bin/PKGBUILD

# First match in each file is the declaration.
arch_ver=$(sed -n 's/^pkgver=\(.*\)$/\1/p' "$arch_pkgbuild" | head -1)
arch_sha=$(sed -n "s/^sha256sums=(['\"]\([0-9a-f]\{64\}\)['\"].*/\1/p" "$arch_pkgbuild" | head -1)
bin_ver=$(sed -n 's/^pkgver=\(.*\)$/\1/p' "$bin_pkgbuild" | head -1)
bin_sha=$(sed -n "s/^sha256sums=(['\"]\([0-9a-f]\{64\}\)['\"].*/\1/p" "$bin_pkgbuild" | head -1)

for pair in "arch PKGBUILD pkgver:$arch_ver" "arch PKGBUILD sha256sums:$arch_sha" \
  "arch-bin PKGBUILD pkgver:$bin_ver" "arch-bin PKGBUILD sha256sums:$bin_sha"; do
  case "$pair" in
    *:) echo "no value found for ${pair%:}" >&2; exit 1 ;;
  esac
done

if [ "$arch_ver" != "$bin_ver" ]; then
  echo "arch package versions disagree:" >&2
  echo "  $arch_pkgbuild  $arch_ver" >&2
  echo "  $bin_pkgbuild  $bin_ver" >&2
  exit 1
fi
pkgver=$arch_ver

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-arch-digests.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

# Same flags ci.yml uses for its downloads. The tarball stays fail-closed: the tag exists by
# the time any CI run for its version happens, so a download failure here is never a missing
# asset and set -e aborts the run.
curl --fail --location --retry 3 --output "$tmp_dir/source.tar.gz" \
  "https://github.com/Deftera186/tennoscope/archive/refs/tags/v${pkgver}.tar.gz"
# The .deb is different: the release workflow gates the Linux/Windows builds (which attach
# the .deb) on this job succeeding, so the .deb's presence is the publish signal that tells
# an in-flight release apart from a wrong pin. On a new-version release commit the shas are
# stale by design until the post-publish re-pin, so a source mismatch while the .deb 404s
# warns and exits 0 instead of deadlocking the gate that must go green to build the .deb.
# Once the .deb exists every mismatch fails. This stays sound: a wrong pin can only hide
# while its version is unpublished, and an unpublished version cannot pass the release gate's
# tag==version check without a real verification the moment it publishes, because the pin
# commit's own CI run then sees the .deb present. Any digest mismatch with the .deb present
# still fails.
# curl --fail exits 22 for every HTTP status >= 400, so the exit alone cannot tell "not
# published yet" apart from a rate limit or a broken server: skip only on an exact 404 from
# the server, captured with -w. Anything else fails carrying both the curl exit and the code.
deb_status=0
deb_actual=
deb_http_code=000
deb_skipped=0
deb_http_code=$(curl --fail --location --retry 3 --output "$tmp_dir/binary.deb" \
  -w '%{http_code}' \
  "https://github.com/Deftera186/tennoscope/releases/download/v${pkgver}/TennoScope_${pkgver}_amd64.deb") || deb_status=$?
deb_name="TennoScope_${pkgver}_amd64.deb"
if [ "$deb_status" -eq 0 ]; then
  :
elif [ "$deb_http_code" = "404" ]; then
  echo "$deb_name is not published yet (HTTP 404); verifying source tarball only"
  deb_skipped=1
else
  echo "could not download $deb_name (curl exit $deb_status, HTTP $deb_http_code)" >&2
  exit 1
fi

tarball_actual=$(sha256sum "$tmp_dir/source.tar.gz" | cut -d' ' -f1)
if [ "$deb_skipped" -eq 0 ]; then
  deb_actual=$(sha256sum "$tmp_dir/binary.deb" | cut -d' ' -f1)
fi

fail=0
source_mismatch=0
if [ "$tarball_actual" != "$arch_sha" ]; then
  source_mismatch=1
fi
if [ "$source_mismatch" -eq 1 ] && [ "$deb_skipped" -eq 1 ]; then
  echo "warning: source digest for v$pkgver does not match $arch_pkgbuild," >&2
  echo "  but $deb_name is not published yet (HTTP 404):" >&2
  echo "  pinned:   $arch_sha" >&2
  echo "  actual:   $tarball_actual" >&2
  echo "  in-flight release window: v$pkgver not yet published, re-pin still owed per RELEASING step 8" >&2
  exit 0
fi
if [ "$source_mismatch" -eq 1 ]; then
  echo "digest mismatch in $arch_pkgbuild:" >&2
  echo "  pinned:   $arch_sha" >&2
  echo "  actual:   $tarball_actual" >&2
  echo "  re-pin with: sha256sums=('$tarball_actual') in $arch_pkgbuild" >&2
  fail=1
fi
if [ "$deb_skipped" -eq 0 ] && [ "$deb_actual" != "$bin_sha" ]; then
  echo "digest mismatch in $bin_pkgbuild:" >&2
  echo "  pinned:   $bin_sha" >&2
  echo "  actual:   $deb_actual" >&2
  echo "  re-pin with: sha256sums=('$deb_actual') in $bin_pkgbuild" >&2
  fail=1
fi
if [ "$fail" -ne 0 ]; then
  exit 1
fi

if [ "$deb_skipped" -eq 1 ]; then
  echo "arch source digest agrees for v$pkgver (binary .deb not published yet, skipped)"
else
  echo "arch digests agree for v$pkgver (source tarball and binary .deb)"
fi
