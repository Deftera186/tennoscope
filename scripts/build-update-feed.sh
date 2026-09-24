#!/bin/sh
# Build the updater feeds (`latest.json`, and `latest-beta.json` on stable tags)
# from the signed bundles a release job just produced. Per-entry strict: a
# missing or empty `.sig` fails the build, because whole-file validation would
# otherwise disable updates on every platform. Nothing is published by hand.
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

version=""
tag=""
artifacts=""
out=""
notes_file=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) version="${2-}"; shift 2 ;;
    --tag) tag="${2-}"; shift 2 ;;
    --artifacts) artifacts="${2-}"; shift 2 ;;
    --out) out="${2-}"; shift 2 ;;
    --notes-file) notes_file="${2-}"; shift 2 ;;
    -h|--help)
      echo "usage: $0 --version X.Y.Z --tag vX.Y.Z[-rcN] --artifacts DIR --out DIR [--notes-file FILE]" >&2
      exit 0
      ;;
    *)
      echo "unknown argument '$1'" >&2
      exit 2
      ;;
  esac
done

[ -n "$version" ] || { echo "--version is required" >&2; exit 2; }
[ -n "$tag" ] || { echo "--tag is required" >&2; exit 2; }
[ -n "$artifacts" ] || { echo "--artifacts is required" >&2; exit 2; }
[ -n "$out" ] || { echo "--out is required" >&2; exit 2; }
[ -d "$artifacts" ] || { echo "artifacts dir '$artifacts' missing" >&2; exit 1; }
mkdir -p "$out"

command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 127; }

is_prerelease=false
case "$tag" in
  *"-"*) is_prerelease=true ;;
esac
if [ "$is_prerelease" = true ]; then
  # An RC tag builds from the plain workspace version, so the installed RC
  # reports e.g. 0.12.0 -- and a beta feed stamped 0.12.0 would offer the RC
  # its own running build. Stamp prerelease feeds with the tag suffix
  # (0.12.0-rc1 < 0.12.0) so the self-offer can never happen; the later
  # stable 0.12.0 still satisfies the beta comparator.
  version="${tag#v}"
  case "$version" in
    *"-"*) ;;
    *) echo "prerelease tag '$tag' has no version suffix -- refusing to guess" >&2; exit 1 ;;
  esac
fi

# One updater entry per artifact kind present. deb/rpm are installable system
# packages and deliberately never appear here.
entries_tmp=$(mktemp "${TMPDIR:-/tmp}/tennoscope-feed.XXXXXX")
trap 'rm -f "$entries_tmp"' 0
trap 'exit 1' HUP INT TERM
add_entry() {
  # $1 = platform key, $2 = artifact path
  artifact="$2"
  sig="$artifact.sig"
  [ -f "$artifact" ] || { echo "artifact '$artifact' missing" >&2; exit 1; }
  [ -s "$sig" ] || { echo "signature '$sig' missing or empty -- refusing to publish" >&2; exit 1; }
  printf '%s\t%s\t%s\n' "$1" "$(dirname -- "$artifact")" "$(basename -- "$artifact")" >>"$entries_tmp"
}

found_appimage=$(find "$artifacts" -maxdepth 3 -type f -name '*.AppImage' ! -name '*.sig' | head -5)
found_nsis=$(find "$artifacts" -maxdepth 4 -type f -name '*-setup.exe' | head -5)

if [ -z "$found_appimage" ] && [ -z "$found_nsis" ]; then
  echo "no updater artifacts (AppImage or NSIS setup) under '$artifacts'" >&2
  exit 1
fi

# Fail closed on ambiguity: at most one artifact per updater platform kind.
# A second AppImage or NSIS bundle means the feed would silently pick one,
# so abort and name the duplicates instead.
if [ -n "$found_appimage" ]; then
  n_appimage=$(printf '%s\n' "$found_appimage" | wc -l)
  if [ "$n_appimage" -gt 1 ]; then
    echo "multiple AppImage artifacts for linux-x86_64 -- refusing to guess: $(printf '%s' "$found_appimage" | tr '\n' ' ')" >&2
    exit 1
  fi
fi
if [ -n "$found_nsis" ]; then
  n_nsis=$(printf '%s\n' "$found_nsis" | wc -l)
  if [ "$n_nsis" -gt 1 ]; then
    echo "multiple NSIS artifacts for windows-x86_64 -- refusing to guess: $(printf '%s' "$found_nsis" | tr '\n' ' ')" >&2
    exit 1
  fi
fi

for file in $found_appimage; do
  add_entry "linux-x86_64" "$file"
done
for file in $found_nsis; do
  add_entry "windows-x86_64" "$file"
done

notes=""
if [ -n "$notes_file" ]; then
  [ -f "$notes_file" ] || { echo "notes file '$notes_file' missing" >&2; exit 1; }
  notes=$(cat -- "$notes_file")
fi

pub_date=$(date -u +%Y-%m-%dT%H:%M:%SZ)
repo="https://github.com/Deftera186/tennoscope"

export FEED_VERSION="$version" FEED_TAG="$tag" FEED_DATE="$pub_date" FEED_NOTES="$notes"
export FEED_ENTRIES="$entries_tmp" FEED_OUT="$out" FEED_PRERELEASE="$is_prerelease" FEED_REPO="$repo"

# Stable tags publish both feeds (the beta file rides along so the latest-pointer
# always serves one); prerelease tags publish only the beta feed.
python3 - "$artifacts" <<'EOF'
import json
import os
entries = []
with open(os.environ["FEED_ENTRIES"], encoding="utf-8") as handle:
    for line in handle.read().splitlines():
        platform, directory, base = line.split("\t")
        with open(os.path.join(directory, base + ".sig"), encoding="utf-8") as sig:
            # The `.sig` artifact already holds the base64 blob the updater
            # wants: it decodes to the full minisign text (untrusted comment /
            # signature / trusted comment / global signature). Embed verbatim.
            signature = sig.read().strip()
        if not signature:
            raise SystemExit(f"empty signature for {base}")
        entries.append((platform, base, signature))
repo = os.environ["FEED_REPO"]
tag = os.environ["FEED_TAG"]
platforms = {
    platform: {
        "signature": signature,
        "url": f"{repo}/releases/download/{tag}/{base}",
    }
    for platform, base, signature in entries
}
feed = {
    "version": os.environ["FEED_VERSION"],
    "notes": os.environ["FEED_NOTES"],
    "pub_date": os.environ["FEED_DATE"],
    "platforms": platforms,
}
out = os.environ["FEED_OUT"]
names = ["latest-beta.json"] if os.environ["FEED_PRERELEASE"] == "true" else ["latest.json", "latest-beta.json"]
for name in names:
    with open(os.path.join(out, name), "w", encoding="utf-8") as handle:
        json.dump(feed, handle, indent=2)
        handle.write("\n")
    print(f"wrote {name} with platforms: {', '.join(sorted(platforms))}")
EOF
