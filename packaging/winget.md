# Winget packaging

Manifests for [`winget-pkgs`](https://github.com/microsoft/winget-pkgs), targeting the per-user
NSIS installer attached to each GitHub release. It is not code-signed; SmartScreen reputation is
disclosed in [`../docs/install.md`](../docs/install.md). Nothing is merged upstream, so
`winget install` does not resolve the package and the install guide points at the
release asset instead.

## Layout

These files map 1:1 onto a winget-pkgs version directory:

| Here | In `manifests/d/Deftera186/TennoScope/<version>/` |
| --- | --- |
| `Deftera186.TennoScope.yaml` | `Deftera186.TennoScope.yaml` |
| `Deftera186.TennoScope.installer.yaml` | `Deftera186.TennoScope.installer.yaml` |
| `Deftera186.TennoScope.locale.en-US.yaml` | `Deftera186.TennoScope.locale.en-US.yaml` |

Keep this directory manifest-only. CI validates it as a directory manifest with
`winget validate --manifest packaging/winget`, and a README parked here makes winget
reject the whole set as rootless.

## New release checklist

1. Download the `*-setup.exe` for the new tag and hash it:
   `sha256sum TennoScope_*_x64-setup.exe`.
2. Copy the three manifests, bump `PackageVersion`, the `InstallerUrl`, and the
   `InstallerSha256` (uppercase hex, as `winget-create` emits).
3. Local Windows VM first: `winget validate --manifest <dir>`, then `winget install --manifest
   <dir>` once `winget settings --enable LocalManifestFiles` is on. CI's `windows-latest` runner
   repeats the validate.
4. Open the winget-pkgs PR (manifests only, one version per PR) and sign the
   Microsoft CLA once. A repository bot nudges an inactive PR after 5 days and closes it
   3 more days later if the reminder goes unanswered, so shepherd it.

## Scope notes

- `Scope: user` matches the installer (per-user, no UAC). `UpgradeBehavior:
  install`: NSIS reinstalls over the same directory.
- Silent switches are `/S` (stock NSIS). Never add a script installer.
- The manifests pin a version-specific release URL, never `releases/latest`.
