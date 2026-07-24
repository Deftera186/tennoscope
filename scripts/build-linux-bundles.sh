#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)

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

cd "$repo_root"
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

cd "$repo_root/app"
pnpm check

for bundle in "$@"; do
  if [ "$bundle" = appimage ]; then
    # linuxdeploy's bundled strip may not understand newer RELR sections
    # emitted by rolling-release distributions. Skipping this optional size
    # optimization keeps the build portable and does not alter the binary.
    NO_STRIP=${NO_STRIP:-true} pnpm tauri build --bundles "$bundle"
  else
    pnpm tauri build --bundles "$bundle"
  fi
done
