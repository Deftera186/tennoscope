#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-windows-check-test.XXXXXX")
trap 'rm -rf "$test_root"' 0
trap 'exit 1' HUP INT TERM

bin_dir="$test_root/bin"
log="$test_root/cargo.log"
build_root="$test_root/build"
mkdir -p "$bin_dir"
llvm_lib="$test_root/llvm/lib"
mkdir -p "$llvm_lib"
: >"$llvm_lib/libclang.so"

cat >"$bin_dir/cargo" <<'EOF'
#!/bin/sh
{
  printf 'CARGO_TARGET_DIR=%s\n' "$CARGO_TARGET_DIR"
  printf 'TMPDIR=%s\n' "$TMPDIR"
  printf 'LIBCLANG_SET=%s\n' "${LIBCLANG_PATH+x}"
  printf 'LIBCLANG_PATH=%s\n' "${LIBCLANG_PATH-}"
  printf 'ARGS='
  printf '%s ' "$@"
  printf '\n'
} >"$CHECK_WINDOWS_LOG"
EOF

cat >"$bin_dir/rustup" <<'EOF'
#!/bin/sh
printf '%s ' "$@" >"$CHECK_WINDOWS_RUSTUP_LOG"
printf '\n' >>"$CHECK_WINDOWS_RUSTUP_LOG"
printf '%s\n' x86_64-unknown-linux-gnu x86_64-pc-windows-gnu
EOF
cat >"$bin_dir/x86_64-w64-mingw32-gcc" <<'EOF'
#!/bin/sh
exit 0
EOF
cat >"$bin_dir/llvm-config" <<EOF
#!/bin/sh
printf '%s\n' "$llvm_lib"
EOF
chmod +x "$bin_dir/cargo" "$bin_dir/rustup" "$bin_dir/x86_64-w64-mingw32-gcc" "$bin_dir/llvm-config"

env -u CARGO_TARGET_DIR -u LIBCLANG_PATH -u TMPDIR \
  CHECK_WINDOWS_LOG="$log" CHECK_WINDOWS_RUSTUP_LOG="$test_root/rustup.log" \
  TENNOSCOPE_BUILD_ROOT="$build_root" PATH="$bin_dir:$PATH" \
  "$repo_root/scripts/check-windows.sh"

[ -d "$build_root/windows-target" ] || {
  echo "Windows target directory was not created under the configured build root" >&2
  exit 1
}
[ -d "$build_root/tmp" ] || {
  echo "temporary directory was not created under the configured build root" >&2
  exit 1
}

expected=$(cat <<EOF
CARGO_TARGET_DIR=$build_root/windows-target
TMPDIR=$build_root/tmp
LIBCLANG_SET=x
LIBCLANG_PATH=$llvm_lib
ARGS=clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings 
EOF
)
actual=$(cat "$log")
[ "$actual" = "$expected" ] || {
  printf 'unexpected Windows preflight invocation:\n%s\n' "$actual" >&2
  exit 1
}

[ "$(cat "$test_root/rustup.log")" = "target list --installed " ] || {
  echo "Windows preflight did not inspect the installed Rust targets" >&2
  exit 1
}

override_target="$test_root/caller-target"
override_tmp="$test_root/caller-tmp"
override_libclang="$test_root/caller-libclang"
override_log="$test_root/caller.log"
CHECK_WINDOWS_LOG="$override_log" CHECK_WINDOWS_RUSTUP_LOG="$test_root/rustup.log" \
  CARGO_TARGET_DIR="$override_target" LIBCLANG_PATH="$override_libclang" TMPDIR="$override_tmp" \
  TENNOSCOPE_BUILD_ROOT="$build_root" PATH="$bin_dir:$PATH" \
  "$repo_root/scripts/check-windows.sh"
override_actual=$(cat "$override_log")
case "$override_actual" in
  *"CARGO_TARGET_DIR=$override_target"*"TMPDIR=$override_tmp"*"LIBCLANG_PATH=$override_libclang"*) ;;
  *)
    printf 'Windows preflight ignored caller storage overrides:\n%s\n' "$override_actual" >&2
    exit 1
    ;;
esac

# An unsuccessful helper lookup must leave LIBCLANG_PATH absent. clang-sys treats even an empty
# explicit path as authoritative and will not try its own llvm-config/system fallbacks.
cat >"$bin_dir/node" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$bin_dir/node"
unset_log="$test_root/unset-libclang.log"
env -u CARGO_TARGET_DIR -u LIBCLANG_PATH -u TMPDIR \
  CHECK_WINDOWS_LOG="$unset_log" CHECK_WINDOWS_RUSTUP_LOG="$test_root/rustup.log" \
  TENNOSCOPE_BUILD_ROOT="$build_root" PATH="$bin_dir:$PATH" \
  "$repo_root/scripts/check-windows.sh"
case "$(cat "$unset_log")" in
  *'LIBCLANG_SET=x'*)
    echo "Windows preflight exported an empty LIBCLANG_PATH after unsuccessful discovery" >&2
    exit 1
    ;;
esac

cat >"$bin_dir/rustup" <<'EOF'
#!/bin/sh
printf '%s\n' x86_64-unknown-linux-gnu
EOF
missing_target_error="$test_root/missing-target.err"
if TENNOSCOPE_BUILD_ROOT="$build_root" PATH="$bin_dir:$PATH" \
  "$repo_root/scripts/check-windows.sh" 2>"$missing_target_error"; then
  echo "Windows preflight accepted a missing Rust target" >&2
  exit 1
fi
case "$(cat "$missing_target_error")" in
  *'rustup target add x86_64-pc-windows-gnu'*) ;;
  *)
    echo "Windows preflight did not explain how to install the missing Rust target" >&2
    exit 1
    ;;
esac
