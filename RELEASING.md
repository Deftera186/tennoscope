# Releasing

Releases are cut by hand. Nothing publishes on a merge to `main`; pushing a tag is the only thing
that builds an artifact, and even then the GitHub release is created as a **draft**.

## Versioning

[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html). The first release is `0.1.0`.

While the major version is `0`, the public surface — application behaviour, the SQLite schema, and
setup state — may change in any minor release:

| Change | Bump |
| --- | --- |
| Bug fix, no behaviour change a user would have to adapt to | `0.1.0` → `0.1.1` |
| New feature, or any change to the schema, setup state, or an existing behaviour | `0.1.0` → `0.2.0` |
| First release the project is willing to keep compatible | `1.0.0` |

Tags are `v`-prefixed: `v0.1.0`. The version inside the repository is not.

## Cutting a release

1. **Confirm the tree is green.** CI runs this, but run it locally too — the reward reader's tests
   need ImageMagick and Tesseract, and a machine missing one fails differently than CI does.

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

3. **Close the changelog section.** Rename `## [Unreleased]` to `## [0.1.0] - YYYY-MM-DD`, open a
   fresh empty `## [Unreleased]` above it, and update the link definitions at the bottom.

4. **Commit and tag.**

   ```bash
   git commit -am "chore: release v0.1.0"
   git tag -a v0.1.0 -m "v0.1.0"
   git push origin main --follow-tags
   ```

5. **Wait for the release workflow**, then edit the draft it created. Take the release notes from
   the changelog section rather than the generated commit list, and say plainly what is untested —
   window managers other than sway, ultrawide displays, anything else that has never run outside the
   maintainer's machine.

6. **Publish the draft.**

7. **Bump the Gentoo overlay.** `games-util/tennoscope-bin` and `games-util/tennoscope` live in
   [deftera-overlay](https://github.com/Deftera186/deftera-overlay), not here. Both need a checksum
   of a published artifact, so this can only happen after step 6. Copy the ebuilds to the new
   version, regenerate the Manifests, run `pkgcheck scan`, and push.

## Packaging

The bundles the workflow attaches are built by `scripts/build-linux-bundles.sh`, which runs the
test suite and asserts the AppImage still forces `GDK_BACKEND=x11` before anything is uploaded.
The Arch `PKGBUILD` and the overlay ebuilds fetch the tag's own archive, so they only work once
the tag is pushed.

## Yanking

There is no unpublish. If a release has to be withdrawn, delete the GitHub release, leave the tag,
and cut a patch release that says what happened in its changelog entry.
