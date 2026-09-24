# APT repository (Debian/Ubuntu)

Published from release `.deb`s to the `gh-pages` branch by
`.github/workflows/apt.yml` on every published release. Served at
`https://deftera186.github.io/tennoscope` (`stable` suite, `main` component).

## Keys

- Signing identity: `TennoScope APT` (ed25519, 2-year expiry). Verify out of
  band against fingerprint `FE3674F54C65B279EF5C5E8EEE1607F85A9EB802`.
- The public key is committed here as `key.asc` and served at the repo root;
  the install guide pins it via `signed-by` (never the deprecated `apt-key`).
- Rotation: publish a successor key the same way and re-run the workflow (it
  re-signs the whole repo), then tell users to re-fetch `key.asc`.

## First publish checklist

1. Merge this, cut a release, publish the draft.
2. The workflow creates `gh-pages` on first publish.
3. Enable Pages once: repo Settings → Pages → Deploy from branch → `gh-pages`.
   (Or ask and it gets done via API after the first publish.)
4. Re-run the failed-then-green check: `curl` the `InRelease` URL from the
   install guide and confirm `apt update` resolves `tennoscope`.

## Local verification

`./scripts/test-build-apt-repo.sh` covers generation (unsigned). The signing
path was proven by hand against a real `.deb`: `InRelease`/`Release.gpg`
verify under the committed key. No `apt` client exists on the maintainer's
Gentoo box, so a real `apt update` against the repo is unverified until a
Debian/Ubuntu runner or container runs it — the workflow's own shape
(`dpkg-scanpackages` index + standard signed Release) is the portable part.
