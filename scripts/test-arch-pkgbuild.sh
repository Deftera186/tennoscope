#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-arch-pkgbuild-test.XXXXXX")
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

bin_dir="$test_root/bin"
log="$test_root/cargo.log"
mkdir -p "$bin_dir"

cat >"$bin_dir/cargo" <<'EOF'
#!/bin/sh
{
  printf 'CARGO_TARGET_DIR=%s\n' "${CARGO_TARGET_DIR-}"
  printf 'RUSTFLAGS=%s\n' "${RUSTFLAGS-}"
  printf 'ARGS='
  printf '%s ' "$@"
  printf '\n'
} >>"$ARCH_PKGBUILD_LOG"
EOF
cat >"$bin_dir/pnpm" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$bin_dir/cargo" "$bin_dir/pnpm"
srcdir="$test_root"
ARCH_PKGBUILD_LOG="$log"
export srcdir ARCH_PKGBUILD_LOG
PATH="$bin_dir:$PATH"
export PATH
# shellcheck source=/dev/null
source "$repo_root/packaging/arch/PKGBUILD"
mkdir -p "$srcdir/${pkgname}-${pkgver}"
expected_build_rustflags=${RUSTFLAGS-}
build
check


mapfile -t cargo_commands < <(sed -n 's/^ARGS=//p' "$log")
if [[ ${#cargo_commands[@]} -ne 2 ||
      ${cargo_commands[0]} != 'build --release --locked -p tennoscope --features tauri/custom-protocol ' ||
      ${cargo_commands[1]} != 'test --workspace --locked -- --skip a_16_10_screen_is_read_where_a_16_10_screen_actually_sits ' ]]; then
  printf 'Arch harness did not exercise build() immediately before check():\n%s\n' "$(cat "$log")" >&2
  exit 1
fi
mapfile -t rustflags < <(sed -n 's/^RUSTFLAGS=//p' "$log")
expected_check_rustflags="$expected_build_rustflags -C debug-assertions=on"
if [[ ${#rustflags[@]} -ne 2 ||
      ${rustflags[0]} != "$expected_build_rustflags" ||
      ${rustflags[1]} != "$expected_check_rustflags" ]]; then
  printf 'Arch build must preserve release RUSTFLAGS and check() must add only debug assertions:\n%s\n' "$(cat "$log")" >&2
  exit 1
fi
actual=$(cat "$log")
case "$actual" in
  *'--skip a_16_10_screen_is_read_where_a_16_10_screen_actually_sits '*) ;;
  *)
    printf 'Arch check lost its confirmed confidence-only OCR exception; got:\n%s\n' "$actual" >&2
    exit 1
    ;;
esac
case "$actual" in
  *'--skip reads_stacked_quantity_through_production_basket_geometry '*)
    printf 'Arch check must exercise the production quantity contract; got:\n%s\n' "$actual" >&2
    exit 1
    ;;
esac
