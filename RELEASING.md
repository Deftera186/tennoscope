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

2. **Set the version in all four places.** They must agree; nothing checks this automatically.

   - `app/src-tauri/tauri.conf.json` — `version`
   - `app/package.json` — `version`
   - every `Cargo.toml` under `crates/` and `app/src-tauri/`
   - `packaging/arch/PKGBUILD` — `pkgver`, and rename `packaging/gentoo/tennoscope-<version>.ebuild`

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

## Packaging

The bundles the workflow attaches are built by `pnpm tauri build`. The Arch and Gentoo recipes in
[`packaging/`](packaging) still consume a local source archive: they need an immutable release URL,
which the first published tag is what finally provides. Update them after the first release, not
before.

## Yanking

There is no unpublish. If a release has to be withdrawn, delete the GitHub release, leave the tag,
and cut a patch release that says what happened in its changelog entry.
