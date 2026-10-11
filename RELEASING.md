# Releasing

Releases are cut by hand. Nothing publishes on a merge to `main`; pushing a tag is the only thing
that builds an artifact, and even then the GitHub release is created as a **draft**.

## Versioning

[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The first release is `0.1.0`.

While the major version is `0`, per semver §4 anything may change, but bumps mean:

| Change | Bump |
| --- | --- |
| Backwards-compatible bug fix, including corrections to existing behaviour | patch |
| New functionality, backwards compatible | minor |
| Incompatible change to the schema, setup state, API, or behaviour | minor, or major once `1.0.0` declares stability |
| First release the project is willing to keep compatible | `major`, which semver names `1.0.0` |

A fix that restores intended behaviour is a patch, even when the user can see the difference.

Tags are `v`-prefixed: `vX.Y.Z`. The version inside the repository is not. GitHub release titles
are `TennoScope vX.Y.Z`, the product name followed by the exact tag, and `release.yml` builds
that string from the tag itself so the two cannot drift. `X.Y.Z` is a placeholder throughout this
document: substitute the version you are cutting.

## Cutting a release

1. **Confirm the tree is green.** CI runs this, but run it locally too: the reward reader's tests
   need Tesseract, and a box that has never built a Tauri app is also missing libclang and pnpm,
   which fails the first three commands at once rather than one at a time.

   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cd app && pnpm check
   ```

   CI's remaining gates, the packaging and updater ones, are listed in
   [CONTRIBUTING.md](CONTRIBUTING.md).

2. **Set the version in all five places**, then confirm with `./scripts/check-versions.sh`. CI
   runs it too, so drift fails the build rather than shipping mislabelled bundles.

   - `Cargo.toml`: `[workspace.package] version`, which every crate inherits
   - `app/src-tauri/tauri.conf.json`: `version`
   - `app/package.json`: `version`
   - `packaging/arch/PKGBUILD`: `pkgver`
   - `packaging/arch-bin/PKGBUILD`: `pkgver`

   Two more carry the version and nothing checks them. `packaging/winget/*.yaml` wants
   `PackageVersion` and `InstallerUrl`, and after step 6 the `InstallerSha256` too;
   `packaging/copr/tennoscope.spec` wants `Version:` and a `%changelog` entry. The script compares
   only the five above, so a winget manifest left at the old version ships a pull request whose
   installer is the previous release's.

   Then `cargo update --workspace --offline`, so `Cargo.lock` names the new version for every local
   package too. The GitHub release build passes no `--locked`, so it will not catch a lockfile left
   at the old version, and the next cargo call silently rewrites it. The Arch and COPR recipes do
   pass `--locked` (`packaging/arch/PKGBUILD`, `packaging/copr/tennoscope.spec`) and either one
   fails outright on a lockfile that was not updated.

3. **Close the changelog section.** Rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`, open a
   fresh empty `## [Unreleased]` above it, and update the link definitions at the bottom. That
   version has to be the one in step 4, because the release gate compares the tag against the
   workspace version and refuses to build when they differ.

   The changelog is what a player reads to decide whether to update, so write it for one: what
   changed, and what it was getting wrong before, in as few words as that takes. The reasoning
   behind a change belongs in its commit message and the design docs, not here.

4. **Commit and tag.**

   ```bash
   git commit -am "chore(release): prepare vX.Y.Z"
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin main --follow-tags
   ```

5. **Wait for the release workflow**, then edit the draft it created. The draft arrives
   pre-filled with GitHub's generated commit list, and that is not the release notes. Replace them
   with **the changelog section and nothing else**. Add the install commands for this version, and
   a line on anything untested only if the changelog does not already say it.

6. **Publish the draft.**

7. **Bump the Gentoo overlay.** `games-util/tennoscope-bin` and `games-util/tennoscope` live in
   [deftera-overlay](https://github.com/Deftera186/deftera-overlay), not here. Both need a checksum
   of a published artifact, so this can only happen after step 6. Copy the ebuilds to the new
   version, regenerate the Manifests, run `pkgcheck scan`, and push.

8. **Re-pin both Arch digests.** The `pkgver` bumps in step 2 leave the old `sha256sums` behind,
   and neither digest exists until the tag is pushed and the release workflow has attached its
   bundles. Refresh each recipe from its published artifact, then confirm with
   `./scripts/check-arch-digests.sh`. CI runs it too: while the new version is still unpublished a
   mismatch warns and passes, but once the `.deb` is attached a stale pin fails the build rather
   than shipping a `PKGBUILD` whose checksum does not match:

   ```bash
   cd packaging/arch && updpkgsums && cd ../..
   cd packaging/arch-bin && updpkgsums && cd ../..
   ./scripts/check-arch-digests.sh
   ```

   The source recipe pins the tag's `v${pkgver}.tar.gz` archive; the `-bin`
   recipe pins `TennoScope_${pkgver}_amd64.deb` from the release. Commit the
   refreshed `PKGBUILD`s on top of the release.

9. **Update the remaining install channels.** The workflow attaches the bundles and stops there.
   Each of these needs the release published, so they all follow step 6.

   - **COPR.** Submit an SRPM exactly as [packaging/copr/README.md](packaging/copr/README.md)
     spells out. It builds the tag's own archive, so it cannot go before step 4.
   - **winget.** Re-pin `InstallerSha256` in `packaging/winget` from the published installer, then
     open a pull request from your own fork of
     [winget-pkgs](https://github.com/microsoft/winget-pkgs) and close the previous version's.
     Two of its validator's asks are easy to miss: `LicenseUrl` has to be the raw license
     (`https://raw.githubusercontent.com/<owner>/<repo>/main/LICENSE`), because URL validation
     reports the `blob/` page as `ServiceUnavailable`, and the CLA needs one
     `@microsoft-github-policy-service agree` comment, which the bot answers by clearing
     `Needs-CLA`. After that the pull request waits on a moderator, and nothing here can hurry it.
   - **APT.** Nothing to run: the workflow publishes the pool itself. Confirm the new `.deb` is in
     `dists/stable/main/binary-amd64/Packages` as
     [packaging/apt/README.md](packaging/apt/README.md) describes.
   - **AppImage catalog.** Post `/retest` on the existing pull request in
     [appimage.github.io](https://github.com/AppImage/appimage.github.io) so the listed AppImage
     is the new one, and read the bot's verdict before calling the channel updated.

## Packaging

The bundles the workflow attaches are built by `scripts/build-linux-bundles.sh`. Run by hand it
gates on the test suite, clippy and `pnpm check` first; the release workflow passes `--skip-gates`
because it refuses to start until CI has passed on that very commit. Either way the script asserts
the AppImage still forces `GDK_BACKEND=x11` before anything is uploaded. That check runs against
the artifact itself and nothing else covers it. The source Arch `PKGBUILD` and the overlay ebuilds
fetch the tag's own archive, so they only work once the tag is pushed; the `-bin` recipe fetches
the release `.deb`, so it only works once the release workflow has attached it.

Which runner builds which bundle is not interchangeable. `bundles` runs on 24.04 and produces the
`.deb` and the `.rpm`; `appimage` runs on 22.04 and produces the AppImage, and the split exists
because the catalog's own `code/check-libc.sh` rejects any ELF in the AppDir that references a
GLIBC newer than jammy's 2.35. A 24.04 host bakes 2.39 into the binary and into every library
linuxdeploy bundles beside it, so the AppImage cannot be built there even though the installable
packages can: those need 2.39, and Ubuntu 22.04 plus Debian 12 take the
[AppImage](docs/install.md) instead. The jammy job builds the pipewire that
`release.yml` pins, because jammy's own 0.3.48 predates the bindings this app compiles against.
Bumping that pin, or moving the AppImage off jammy, changes the floor for every user, so it needs
the published artifact to still report `GLIBC_2.35` before the release goes out.

## Yanking

There is no unpublish. If a release has to be withdrawn, delete the GitHub release, leave the tag,
and cut a patch release that says what happened in its changelog entry.
