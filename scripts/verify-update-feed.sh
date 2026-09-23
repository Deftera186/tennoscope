#!/bin/sh
# Release gate for the updater feeds: schema, pinned URLs, signature freshness,
# and cryptographic verification against the pubkey committed in tauri.conf.json.
# Fails closed on anything unexpected. Needs python3 and the `nacl` module
# (`pip install pynacl`); the release job installs it before calling this.
set -eu

feeds=""
artifacts=""
tauri_conf=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --feeds) feeds="${2-}"; shift 2 ;;
    --artifacts) artifacts="${2-}"; shift 2 ;;
    --tauri-conf) tauri_conf="${2-}"; shift 2 ;;
    -h|--help)
      echo "usage: $0 --feeds DIR --artifacts DIR --tauri-conf tauri.conf.json" >&2
      exit 0
      ;;
    *)
      echo "unknown argument '$1'" >&2
      exit 2
      ;;
  esac
done

[ -n "$feeds" ] || { echo "--feeds is required" >&2; exit 2; }
[ -n "$artifacts" ] || { echo "--artifacts is required" >&2; exit 2; }
[ -n "$tauri_conf" ] || { echo "--tauri-conf is required" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 127; }

export VERIFY_FEEDS="$feeds" VERIFY_ARTIFACTS="$artifacts" VERIFY_CONF="$tauri_conf"

python3 <<'EOF'
import base64
import datetime
import json
import os
import re
import sys

try:
    from nacl.signing import VerifyKey
    from nacl.exceptions import BadSignatureError
except ImportError:
    sys.exit("pynacl is required (pip install pynacl) -- refusing to gate without crypto")

SEMVER = re.compile(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$")
KNOWN_PLATFORMS = {"linux-x86_64", "windows-x86_64"}

with open(os.environ["VERIFY_CONF"], encoding="utf-8") as handle:
    conf = json.load(handle)
committed = conf["plugins"]["updater"]["pubkey"]
key_raw = base64.b64decode(committed).decode("utf-8").splitlines()[-1]
pub_raw = base64.b64decode(key_raw)
assert pub_raw[:2] == b"Ed", "committed pubkey is not a minisign Ed key"
vk = VerifyKey(pub_raw[10:42])
key_id = pub_raw[2:10]

errors = []


def fail(message):
    errors.append(message)


feeds = os.environ["VERIFY_FEEDS"]
artifacts = os.environ["VERIFY_ARTIFACTS"]
names = sorted(f for f in os.listdir(feeds) if f.endswith(".json"))
if not names:
    fail(f"no feed files in {feeds}")

for name in names:
    with open(os.path.join(feeds, name), encoding="utf-8") as handle:
        try:
            feed = json.load(handle)
        except json.JSONDecodeError as exc:
            fail(f"{name}: invalid JSON ({exc})")
            continue
    version = feed.get("version", "")
    if not SEMVER.match(str(version)):
        fail(f"{name}: version {version!r} is not semver")
    try:
        datetime.datetime.fromisoformat(str(feed.get("pub_date", "")).replace("Z", "+00:00"))
    except ValueError:
        fail(f"{name}: pub_date {feed.get('pub_date')!r} is not RFC 3339")
    platforms = feed.get("platforms", {})
    if not platforms or not set(platforms) <= KNOWN_PLATFORMS:
        fail(f"{name}: platform keys {sorted(platforms)} outside {sorted(KNOWN_PLATFORMS)}")
        continue
    for platform, entry in platforms.items():
        url = entry.get("url", "")
        base = url.rsplit("/", 2)
        if len(base) != 3 or "/releases/download/" not in url:
            fail(f"{name}/{platform}: url {url!r} is not a pinned release asset")
            continue
        tag, asset = base[1], base[2]
        found = None
        for root, _, files in os.walk(artifacts):
            if asset in files:
                found = os.path.join(root, asset)
                break
        if found is None:
            fail(f"{name}/{platform}: asset {asset!r} absent from artifacts")
            continue
        try:
            text = base64.b64decode(entry.get("signature", "")).decode("utf-8")
        except Exception:
            fail(f"{name}/{platform}: signature is not base64 of minisign text")
            continue
        lines = text.splitlines()
        if (
            len(lines) != 4
            or not lines[0].startswith("untrusted comment: ")
            or not lines[2].startswith("trusted comment: ")
        ):
            fail(f"{name}/{platform}: signature is not 4-line minisign text")
            continue
        sig = base64.b64decode(lines[1])
        if sig[2:10] != key_id:
            fail(f"{name}/{platform}: signature key id does not match committed pubkey")
            continue
        data = open(found, "rb").read()
        if sig[:2] == b"ED":
            from nacl.encoding import RawEncoder
            from nacl.hash import blake2b
            data = blake2b(data, digest_size=64, encoder=RawEncoder)
        try:
            vk.verify(sig[10:74] + data)
        except BadSignatureError:
            fail(f"{name}/{platform}: file signature INVALID for {asset}")
            continue
        # Freshness: the trusted comment pins the filename and signing time. A
        # stale build-time signature (pre-repack) names the right file but an
        # older instant than the artifact it ships beside.
        trusted = lines[2]
        if f"file:{asset}" not in trusted.replace(" ", ""):
            fail(f"{name}/{platform}: trusted comment does not pin {asset}")
            continue
        stamp = re.search(r"timestamp:(\d+)", trusted)
        mtime = os.path.getmtime(found)
        if not stamp or int(stamp.group(1)) < int(mtime) - 120:
            fail(f"{name}/{platform}: signature predates the artifact (stale re-sign?)")
            continue
        print(f"{name}/{platform}: ok ({asset})")

if errors:
    print("FEED VERIFICATION FAILED:", file=sys.stderr)
    for message in errors:
        print(f"  - {message}", file=sys.stderr)
    sys.exit(1)
print("feed verification passed")
EOF
