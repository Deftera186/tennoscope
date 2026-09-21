# Releasing

Releases are cut by hand. Nothing publishes on a merge to `main`; pushing a tag is the only thing
that builds an artifact, and even then the GitHub release is created as a **draft**.

## Versioning

[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The first release is `0.1.0`.

While the major version is `0`, per semver §4 anything may change, but bumps mean:

| Change | Bump |
| --- | --- |
| Backwards-compatible bug fix, including corrections to existing behaviour | `0.10.0` → `0.10.1` |
| New functionality, backwards compatible | `0.10.0` → `0.11.0` |
| Incompatible change to the schema, setup state, API, or behaviour | `0.10.0` → `0.11.0` until `1.0.0` declares stability |
| First release the project is willing to keep compatible | `1.0.0` |

Prior releases through `0.11.0` treated any user-visible behaviour change as minor; from here on a fix that restores intended behaviour is a patch even when the user can see the difference.

Tags are `v`-prefixed: `v0.1.0`. The version inside the repository is not.
GitHub release titles are `TennoScope v0.1.0`: the product name followed by the exact tag.

## Cutting a release

1. **Confirm the tree is green.** CI runs this, but run it locally too — the reward reader's tests
   need Tesseract, and a machine missing it fails differently than CI does.

   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cd app && pnpm check
   ```

2. **Set the version in all four places**, then confirm with `./scripts/check-versions.sh` — CI
   runs it too, so drift fails the build rather than shipping mislabelled bundles.

   - `Cargo.toml` — `[workspace.package] version`, which every crate inherits
   - `app/src-tauri/tauri.conf.json` — `version`
   - `app/package.json` — `version`
   - `packaging/arch/PKGBUILD` — `pkgver`

   Then `cargo update --workspace --offline`, so `Cargo.lock` carries the new version too. The
   release build runs `--locked` and fails on a lockfile that still names the old one.

3. **Close the changelog section.** Rename `## [Unreleased]` to `## [0.1.0] - YYYY-MM-DD`, open a
   fresh empty `## [Unreleased]` above it, and update the link definitions at the bottom.

   The changelog is what a player reads to decide whether to update, so write it for one: what
   changed, and what it was getting wrong before, in as few words as that takes. The reasoning
   behind a change belongs in its commit message and the design docs, not here.

4. **Commit and tag.**

   ```bash
   git commit -am "chore: release v0.1.0"
   git tag -a v0.1.0 -m "v0.1.0"
   git push origin main --follow-tags
   ```

5. **Wait for the release workflow**, then edit the draft it created. **The release notes are the
   changelog section and nothing else** — not the generated commit list, and nothing written fresh
   for the occasion. Add the install commands for this version, and a line on anything untested
   only if the changelog does not already say it.

6. **Publish the draft.**

7. **Bump the Gentoo overlay.** `games-util/tennoscope-bin` and `games-util/tennoscope` live in
   [deftera-overlay](https://github.com/Deftera186/deftera-overlay), not here. Both need a checksum
   of a published artifact, so this can only happen after step 6. Copy the ebuilds to the new
   version, regenerate the Manifests, run `pkgcheck scan`, and push.

## Packaging

The bundles the workflow attaches are built by `scripts/build-linux-bundles.sh`. Run by hand it
gates on the test suite, clippy and `pnpm check` first; the release workflow passes `--skip-gates`
because it refuses to start until CI has passed on that very commit. Either way the script asserts
the AppImage still forces `GDK_BACKEND=x11` before anything is uploaded -- that check runs against
the artifact itself and nothing else covers it. The Arch `PKGBUILD` and the overlay ebuilds fetch
the tag's own archive, so they only work once the tag is pushed.

## Yanking

There is no unpublish. If a release has to be withdrawn, delete the GitHub release, leave the tag,
and cut a patch release that says what happened in its changelog entry.
