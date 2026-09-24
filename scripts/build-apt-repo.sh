#!/bin/sh
# Build (or extend) the TennoScope APT repository: pool + Packages index +
# signed Release files. Re-run per release against a checkout of the repo
# branch (gh-pages); existing pool entries are kept, indexes regenerated.
set -eu

repo=""
suite="stable"
component="main"
key_id=""
no_sign=false
deb=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo) repo="${2-}"; shift 2 ;;
    --suite) suite="${2-}"; shift 2 ;;
    --component) component="${2-}"; shift 2 ;;
    --key-id) key_id="${2-}"; shift 2 ;;
    --no-sign) no_sign=true; shift ;;
    --deb) deb="${deb} ${2-}"; shift 2 ;;
    -h|--help)
      echo "usage: $0 --repo DIR --deb FILE [--deb FILE] [--suite stable] [--component main] [--key-id ID | --no-sign]" >&2
      exit 0
      ;;
    *)
      echo "unknown argument '$1'" >&2
      exit 2
      ;;
  esac
done

[ -n "$repo" ] || { echo "--repo is required" >&2; exit 2; }
[ -n "$deb" ] || { echo "at least one --deb is required" >&2; exit 2; }
if [ "$no_sign" = false ]; then
  [ -n "$key_id" ] || { echo "--key-id is required without --no-sign" >&2; exit 2; }
fi

command -v dpkg-scanpackages >/dev/null 2>&1 || { echo "dpkg-scanpackages is required" >&2; exit 127; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 127; }

pool="$repo/pool/$component"
bindir="$repo/dists/$suite/$component/binary-amd64"
mkdir -p "$pool" "$bindir"

for file in $deb; do
  [ -f "$file" ] || { echo "deb '$file' missing" >&2; exit 1; }
  base=$(basename -- "$file")
  case "$base" in
    *.deb) ;;
    *) echo "'$base' is not a .deb" >&2; exit 1 ;;
  esac
  cp -- "$file" "$pool/$base"
done
# Filenames in Packages must be repo-root-relative: apt resolves them against
# the suite root, so an absolute (or site-prefixed) dir here 404s every install.
# Scan from inside the repo with a relative pool path.
(
  cd -- "$repo" || exit 1
  dpkg-scanpackages --multiversion --arch amd64 "pool/$component" >"dists/$suite/$component/binary-amd64/Packages"
)
# Deterministic gzip: identical inputs yield identical trees.
gzip -n -9kf "$bindir/Packages"

export REPO_DIR="$repo" REPO_SUITE="$suite" REPO_COMPONENT="$component"
python3 <<'EOF'
import email.utils
import hashlib
import os
import time as _time

repo = os.environ["REPO_DIR"]
suite = os.environ["REPO_SUITE"]
component = os.environ["REPO_COMPONENT"]
dist = os.path.join(repo, "dists", suite)
entries = []
for dirpath, _, filenames in os.walk(dist):
    for name in sorted(filenames):
        if name in ("Release", "InRelease", "Release.gpg"):
            continue
        full = os.path.join(dirpath, name)
        rel = os.path.relpath(full, dist)
        with open(full, "rb") as handle:
            blob = handle.read()
        entries.append((rel, len(blob),
                        hashlib.md5(blob).hexdigest(),
                        hashlib.sha256(blob).hexdigest()))
lines = [
    "Origin: TennoScope",
    "Label: TennoScope",
    f"Suite: {suite}",
    f"Codename: {suite}",
    f"Date: {email.utils.formatdate(_time.time(), usegmt=True)}",
    "Architectures: amd64",
    f"Components: {component}",
    "Description: Local-first Warframe collection and relic companion",
]
lines.append("MD5Sum:")
lines += [f" {md5} {size:16d} {rel}" for rel, size, md5, _ in entries]
lines.append("SHA256:")
lines += [f" {sha} {size:16d} {rel}" for rel, size, _, sha in entries]
with open(os.path.join(dist, "Release"), "w", encoding="utf-8") as handle:
    handle.write("\n".join(lines) + "\n")
print(f"wrote Release with {len(entries)} entries")
EOF

if [ "$no_sign" = true ]; then
  echo "unsigned repository at $repo (local test only -- never publish this)"
  exit 0
fi

command -v gpg >/dev/null 2>&1 || { echo "gpg is required for signing" >&2; exit 127; }
gpg --batch --yes --local-user "$key_id" --clearsign \
  -o "$repo/dists/$suite/InRelease" "$repo/dists/$suite/Release"
gpg --batch --yes --local-user "$key_id" --armor --detach-sign \
  -o "$repo/dists/$suite/Release.gpg" "$repo/dists/$suite/Release"
echo "signed repository at $repo"
