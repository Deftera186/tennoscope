#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
windows_target=x86_64-pc-windows-gnu
if ! command -v rustup >/dev/null 2>&1; then
  echo "rustup is required to verify the $windows_target target" >&2
  exit 127
fi
if ! rustup target list --installed | while IFS= read -r target; do
  [ "$target" = "$windows_target" ] && exit 0
done; then
  echo "Rust target $windows_target is required; install it with: rustup target add $windows_target" >&2
  exit 127
fi
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "x86_64-w64-mingw32-gcc is required; install your distribution's MinGW-w64 GCC toolchain" >&2
  exit 127
fi

build_root=${TENNOSCOPE_BUILD_ROOT:-/var/tmp/tennoscope-$(id -u)}
target_dir=${CARGO_TARGET_DIR:-"$build_root/windows-target"}
temp_dir=${TMPDIR:-"$build_root/tmp"}

# Direct Cargo invocations do not pass through scripts/tauri.mjs. Reuse its discovery seam so
# bindgen can reach the Windows-gated TennoScope code on version-slotted LLVM installations.
if [ "${LIBCLANG_PATH+x}" != x ]; then
  discovered_libclang=$(
    node --input-type=module --eval '
      import { pathToFileURL } from "node:url";
      const { configureTauriEnvironment } = await import(pathToFileURL(process.argv[1]));
      process.stdout.write(configureTauriEnvironment(process.env).LIBCLANG_PATH ?? "");
    ' "$repo_root/scripts/tauri-env.mjs"
  )
  if [ -n "$discovered_libclang" ]; then
    LIBCLANG_PATH=$discovered_libclang
    export LIBCLANG_PATH
  fi
fi

umask 077
mkdir -p "$target_dir" "$temp_dir"
cd "$repo_root"
CARGO_TARGET_DIR="$target_dir" TMPDIR="$temp_dir" \
  cargo clippy --workspace --all-targets --target "$windows_target" -- -D warnings
