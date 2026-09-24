#!/bin/sh
# Fixture tests for scripts/build-update-feed.sh: feed contents, file selection
# per tag kind, and the fail-closed signature rules.
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-feed-test.XXXXXX")
trap 'rm -rf "$test_root"' 0
trap 'exit 1' HUP INT TERM

pass=0
fail=0
check() {
  # $1 = description, rest = assertion command
  desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL: $desc" >&2
  fi
}

artifacts="$test_root/artifacts"
out="$test_root/out"
mkdir -p "$artifacts/appimage" "$artifacts/nsis" "$out"
printf 'fake-appimage' >"$artifacts/appimage/TennoScope-0.12.0-x86_64.AppImage"
printf 'SIG-APPIMAGE' >"$artifacts/appimage/TennoScope-0.12.0-x86_64.AppImage.sig"
printf 'fake-exe' >"$artifacts/nsis/TennoScope-0.12.0-x86_64-setup.exe"
printf 'SIG-EXE' >"$artifacts/nsis/TennoScope-0.12.0-x86_64-setup.exe.sig"

feed="$repo_root/scripts/build-update-feed.sh"
sh "$feed" --version 0.12.0 --tag v0.12.0 --artifacts "$artifacts" --out "$out" >/dev/null

check "stable writes latest.json" test -f "$out/latest.json"
check "stable writes latest-beta.json" test -f "$out/latest-beta.json"
check "linux entry pinned to tag asset" grep -q "releases/download/v0.12.0/TennoScope-0.12.0-x86_64.AppImage" "$out/latest.json"
check "windows entry pinned to tag asset" grep -q "releases/download/v0.12.0/TennoScope-0.12.0-x86_64-setup.exe" "$out/latest.json"
check "sig contents embedded, not a path" grep -q "SIG-APPIMAGE" "$out/latest.json"
check "version field matches tag" grep -q '"version": "0.12.0"' "$out/latest.json"
check "no deb or rpm keys" sh -c "! grep -Eq '\"(deb|rpm)' '$out/latest.json'"

rm -f "$out"/*.json
sh "$feed" --version 0.12.0 --tag v0.12.0-rc1 --artifacts "$artifacts" --out "$out" >/dev/null
check "rc writes only the beta feed" sh -c "! test -f '$out/latest.json' && test -f '$out/latest-beta.json'"

rm -f "$artifacts/nsis/TennoScope-0.12.0-x86_64-setup.exe.sig"
check "missing sig fails closed" sh -c "! sh '$feed' --version 0.12.0 --tag v0.12.0 --artifacts '$artifacts' --out '$out' >/dev/null 2>&1"
: >"$artifacts/nsis/TennoScope-0.12.0-x86_64-setup.exe.sig"
check "empty sig fails closed" sh -c "! sh '$feed' --version 0.12.0 --tag v0.12.0 --artifacts '$artifacts' --out '$out' >/dev/null 2>&1"

printf 'SIG-EXE' >"$artifacts/nsis/TennoScope-0.12.0-x86_64-setup.exe.sig"
printf 'fake-appimage-dup' >"$artifacts/appimage/TennoScope-0.12.0-x86_64-duplicate.AppImage"
printf 'SIG-DUP' >"$artifacts/appimage/TennoScope-0.12.0-x86_64-duplicate.AppImage.sig"
check "duplicate-artifact-fails closed" sh -c "! sh '$feed' --version 0.12.0 --tag v0.12.0 --artifacts '$artifacts' --out '$out' >/dev/null 2>&1"
check "duplicate message names culprits" sh -c "sh '$feed' --version 0.12.0 --tag v0.12.0 --artifacts '$artifacts' --out '$out' 2>&1 | grep -q 'TennoScope-0.12.0-x86_64-duplicate.AppImage'"
rm "$artifacts/appimage/TennoScope-0.12.0-x86_64-duplicate.AppImage" "$artifacts/appimage/TennoScope-0.12.0-x86_64-duplicate.AppImage.sig"
printf 'fake-exe-dup' >"$artifacts/nsis/TennoScope-0.12.0-x86_64-duplicate-setup.exe"
printf 'SIG-EXE-DUP' >"$artifacts/nsis/TennoScope-0.12.0-x86_64-duplicate-setup.exe.sig"
check "duplicate nsis fails closed" sh -c "! sh '$feed' --version 0.12.0 --tag v0.12.0 --artifacts '$artifacts' --out '$out' >/dev/null 2>&1"
rm "$artifacts/nsis/TennoScope-0.12.0-x86_64-duplicate-setup.exe" "$artifacts/nsis/TennoScope-0.12.0-x86_64-duplicate-setup.exe.sig"

echo "feed tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
