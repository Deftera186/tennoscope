#!/bin/sh
# Fixture tests for scripts/verify-update-feed.sh. Signatures are real Ed25519
# via pynacl (needs the `nacl` module, like the gate itself); the gate checks
# the file signature only, matching what the updater verifies at runtime.
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tennoscope-verify-test.XXXXXX")
trap 'rm -rf "$test_root"' 0
trap 'exit 1' HUP INT TERM

pass=0
fail=0
check() {
  desc="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL: $desc" >&2
  fi
}

export TEST_ROOT="$test_root" VERIFY_SCRIPT="$script_dir/verify-update-feed.sh"
python3 <<'EOF'
import base64
import json
import os
from nacl.signing import SigningKey

root = os.environ["TEST_ROOT"]
os.makedirs(f"{root}/artifacts/appimage")
os.makedirs(f"{root}/feeds")

sk = SigningKey.generate()
vk = sk.verify_key
keynum = b"\x01\x02\x03\x04\x05\x06\x07\x08"
pub_line = base64.b64encode(b"Ed" + keynum + vk.encode()).decode()
pub_text = f"untrusted comment: minisign public key: 0102030405060708\n{pub_line}\n"
outer = base64.b64encode(pub_text.encode()).decode()

conf = {"plugins": {"updater": {"pubkey": outer}}}
open(f"{root}/tauri.conf.json", "w").write(json.dumps(conf))

payload = b"fake-appimage-bytes"
open(f"{root}/artifacts/appimage/TennoScope-0.12.0-x86_64.AppImage", "wb").write(payload)

sig = sk.sign(payload).signature
sig_line = base64.b64encode(b"Ed" + keynum + sig).decode()
import time as _time
trusted = f"trusted comment: timestamp:{int(_time.time())}\tfile:TennoScope-0.12.0-x86_64.AppImage"
global_sig = base64.b64encode(b"\x00" * 64).decode()
sig_text = f"untrusted comment: fixture signature\n{sig_line}\n{trusted}\n{global_sig}\n"
blob = base64.b64encode(sig_text.encode()).decode()

feed = {
    "version": "0.12.0",
    "notes": "",
    "pub_date": "2026-09-22T00:00:00Z",
    "platforms": {
        "linux-x86_64": {
            "signature": blob,
            "url": "https://github.com/Deftera186/tennoscope/releases/download/v0.12.0/TennoScope-0.12.0-x86_64.AppImage",
        }
    },
}
open(f"{root}/feeds/latest.json", "w").write(json.dumps(feed))
open(f"{root}/env", "w").write(f"{outer}\n{keynum.hex()}\n")
print("fixture ready")
EOF

run_gate() {
  sh "$script_dir/verify-update-feed.sh" --feeds "$test_root/feeds" --artifacts "$test_root/artifacts" --tauri-conf "$test_root/tauri.conf.json"
}

check "valid feed passes" run_gate
printf 'x' >>"$test_root/artifacts/appimage/TennoScope-0.12.0-x86_64.AppImage"
check "tampered artifact fails" sh -c "! sh '$script_dir/verify-update-feed.sh' --feeds '$test_root/feeds' --artifacts '$test_root/artifacts' --tauri-conf '$test_root/tauri.conf.json' >/dev/null 2>&1"
truncate -s -1 "$test_root/artifacts/appimage/TennoScope-0.12.0-x86_64.AppImage"
python3 -c "
import json
p = '$test_root/feeds/latest.json'
d = json.load(open(p))
d['platforms']['linux-x86_64']['url'] = 'https://example.com/evil.AppImage'
json.dump(d, open(p, 'w'))
"
check "unpinned url fails" sh -c "! sh '$script_dir/verify-update-feed.sh' --feeds '$test_root/feeds' --artifacts '$test_root/artifacts' --tauri-conf '$test_root/tauri.conf.json' >/dev/null 2>&1"
python3 -c "
import json
p = '$test_root/feeds/latest.json'
d = json.load(open(p))
d['platforms']['linux-x86_64']['url'] = 'https://github.com/Deftera186/tennoscope/releases/download/v0.12.0/TennoScope-0.12.0-x86_64.AppImage'
d['version'] = 'not-a-version'
json.dump(d, open(p, 'w'))
"
check "non-semver version fails" sh -c "! sh '$script_dir/verify-update-feed.sh' --feeds '$test_root/feeds' --artifacts '$test_root/artifacts' --tauri-conf '$test_root/tauri.conf.json' >/dev/null 2>&1"
python3 -c "
import base64, json
p = '$test_root/feeds/latest.json'
d = json.load(open(p))
d['version'] = '0.12.0'
d['platforms']['linux-x86_64']['url'] = 'https://github.com/Deftera186/tennoscope/releases/download/v0.12.0/TennoScope-0.12.0-x86_64.AppImage'
text = base64.b64decode(d['platforms']['linux-x86_64']['signature']).decode()
lines = text.splitlines()
lines[2] = 'trusted comment: timestamp:1000\tfile:TennoScope-0.12.0-x86_64.AppImage'
d['platforms']['linux-x86_64']['signature'] = base64.b64encode(('\n'.join(lines) + '\n').encode()).decode()
"
check "stale pre-repack signature fails" sh -c "! sh '$script_dir/verify-update-feed.sh' --feeds '$test_root/feeds' --artifacts '$test_root/artifacts' --tauri-conf '$test_root/tauri.conf.json' >/dev/null 2>&1"

echo "verify tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
