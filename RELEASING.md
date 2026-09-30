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
   need Tesseract, and a machine missing it fails differently than CI does.

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

## Packaging

The bundles the workflow attaches are built by `scripts/build-linux-bundles.sh`. Run by hand it
gates on the test suite, clippy and `pnpm check` first; the release workflow passes `--skip-gates`
because it refuses to start until CI has passed on that very commit. Either way the script asserts
the AppImage still forces `GDK_BACKEND=x11` before anything is uploaded. That check runs against
the artifact itself and nothing else covers it. The source Arch `PKGBUILD` and the overlay ebuilds
fetch the tag's own archive, so they only work once the tag is pushed; the `-bin` recipe fetches
the release `.deb`, so it only works once the release workflow has attached it.

## Yanking

There is no unpublish. If a release has to be withdrawn, delete the GitHub release, leave the tag,
and cut a patch release that says what happened in its changelog entry.
