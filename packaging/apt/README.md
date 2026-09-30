# APT repository (Debian/Ubuntu)

Published from the release `.deb`s to the `gh-pages` branch by
`.github/workflows/apt.yml` on every published stable release, and served at
`https://deftera186.github.io/tennoscope` (`stable` suite, `main` component). The
package is `tenno-scope`; the workflow refuses to publish a `.deb` that names anything
else, because a renamed bundle would otherwise reach users as a `Packages` index they
cannot install from.

The pool is seeded from every published stable release that has a `.deb` attached, so it fills
in over time. Nothing is done by hand: each release adds one `.deb` to the same pool and the
workflow re-signs the whole repository.

## Keys

- Signing identity: `TennoScope APT` (ed25519, 2-year expiry). Verify out of
  band against fingerprint `FE3674F54C65B279EF5C5E8EEE1607F85A9EB802`.
- The public key is committed here as `key.asc` and served at the repo root;
  the install guide pins it via `signed-by` (never the deprecated `apt-key`).
- Rotation: publish a successor key the same way and re-run the workflow from its
  `workflow_dispatch` trigger, which exists for exactly that, then tell users to
  re-fetch `key.asc`. The workflow refuses to sign when the secret's fingerprint is
  not the one in `key.asc`, so a half-rotated key fails the run rather than shipping
  signatures no installed key can verify.

## After a publish

1. The branch has a new commit, so `dists/stable/InRelease` carries a fresh `Date` and
   a fresh signature and the old index cannot survive.
2. The new version is in `dists/stable/main/binary-amd64/Packages` under
   `Package: tenno-scope`.
3. From a Debian or Ubuntu client, the install guide's `key.asc` URL resolves and
   `apt update` offers the new version.

## Local verification

`./scripts/test-build-apt-repo.sh` covers generation (unsigned). The signing
path was proven by hand against a real `.deb`: `InRelease`/`Release.gpg`
verify under the committed key. No `apt` client exists on the maintainer's
Gentoo box, so a real `apt update` against the published repository is still
unverified from here; the workflow's own shape
(`dpkg-scanpackages` index + standard signed Release) is the portable part.
