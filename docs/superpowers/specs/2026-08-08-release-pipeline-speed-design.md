# Release pipeline speed

A release takes 50+ minutes. This cuts it to roughly 20 without weakening a single check.

## What the 53 minutes actually was

Measured from `v0.5.1` (run 31021288441), the cleanest recent sample.

The two jobs run back to back, because `bundles` declares `needs: windows` -- not for a build
reason, but so the `.exe` exists to attach to the draft release. 27m + 26m serial.

| Windows installer (27m) | | Linux bundles (26m) | |
| --- | --- | --- | --- |
| clippy | 3m19 | test compile | 8m20 |
| test | 10m51 | test run | 4m50 |
| tauri build | 9m46 | clippy | 1m06 |
| cache save (post) | 1m51 | release build | 6m00 |
| | | appimage/deb/rpm | 4m22 |

Three independent costs, in order of size:

1. **Serialization.** `needs: windows` is an artifact dependency, not a build dependency. ~26m.
2. **Every gate runs twice.** The tag rides a commit that was just pushed to `main`, so CI runs
   `fmt`, `clippy` and `test --workspace` on Linux *and* Windows against the identical SHA.
   Release then recompiles the whole workspace under the `test` profile and reruns it. ~24m.
3. **`rust-cache` misses on every release run.** Logs say `No cache found.` on both jobs.

## Why the cache missed

The keys, read off the run rather than guessed:

| | CI | Release |
| --- | --- | --- |
| Linux | `v0-rust-rust-Linux-x64-…` | `v0-rust-bundles-Linux-x64-…` |
| Windows | `v0-rust-windows-…-530c60a6` | `v0-rust-windows-…-20ba45dd` |

Two different causes, and fixing only one fixes only Linux:

- The prefix embeds `GITHUB_JOB`, so job id `bundles` cannot read what job id `rust` wrote.
  `shared-key` replaces that segment and is the whole fix on Linux.
- Windows' prefix *already agrees*. It misses on the trailing environment hash, because
  `rust-cache` hashes every `CARGO*` variable and `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS` is
  declared at the workflow root in release.yml and nowhere in ci.yml. Moving it onto only the
  steps that build is what makes the hash agree.

A cache saved on a tag ref is scoped to that ref and no later tag can read it, while caches on
the default branch are readable from everywhere. Release can therefore only ever *consume*
CI's caches, never its own. So release sets `save-if: false`: it stops writing 1.5 GB per run
that nothing will ever read, and drops the 1m51 post-step.

Be honest about the size of this win. Cargo fingerprints the profile, so CI's `test`-profile
objects do not satisfy a `release` build. What actually carries over is the registry and the
source downloads, not the codegen. This is the smallest of the three fixes.

## Design

Four jobs. The gate blocks both builds; the builds run concurrently; publishing is its own step
that only attaches what the builds produced.

```
gate ──┬── windows ──┐
       └── bundles ──┴── release (draft, attaches all four artifacts)
```

### gate

Waits for CI to conclude on this exact SHA. It has to *wait*, not read once: the tag and the
`main` commit are pushed together, so at the moment Release starts, CI is still `queued`. A
read-once check would pass vacuously against a suite that has not run yet -- which is worse than
no gate, because it looks like one.

`lewagon/wait-on-check-action`, pinned to a commit SHA rather than a tag. It sits on the release
path, and a tag is a moving pointer.

`fail-on-no-checks` stays at its default of true. A tag whose SHA has no CI at all must fail, not
sail through.

The action's filter only *selects*: it fails when the matched set is empty, never when it is
merely smaller than intended. So a job renamed in ci.yml would fall out of the allowlist and the
gate would go green having waited on two jobs instead of three -- silently, because the regexp and
the job names live in different files with nothing holding them together. A second step counts the
matched successes and fails if there are not three. That is the difference between a gate and the
appearance of one.

CI's `cancel-in-progress` needed a matching change. It grouped by ref, so on `main` it would
cancel the very run the gate waits for as soon as another commit landed -- and `cancelled` is not
an allowed conclusion, so an unrelated merge would fail a release. `main` now groups by SHA, giving
each commit its own group; branches and pull requests still collapse to the newest push.

The gate also asserts the tag equals the workspace version. `check-versions.sh` proves the four
in-tree declarations agree with each other, but nothing has ever proved they agree with the tag
being built -- and only the release workflow knows the tag. `v0.5.3` building `0.5.2` artifacts
is exactly the silent mislabelling `check-versions.sh` was written to prevent, one level up. A
pre-release tag carries a suffix the manifests cannot hold, so the comparison uses the part before
the first `-`. This doubles as the guard on `workflow_dispatch`: dispatched on a branch the ref is
`main`, which names no version, so it fails before building.

### windows / bundles

Identical to today minus the duplicated gates, plus `shared-key` and `save-if: false`.

The Windows job also drops its `choco install tesseract`. That existed to put a Tesseract on
PATH for the test suite; the bundle's own engine comes from `vendor-windows-tesseract.ps1`, which
downloads and verifies its own copy. With the tests gone the install has no remaining consumer.

`build-linux-bundles.sh` grows a `--skip-gates` flag. It skips `cargo test`, `cargo clippy` and
`pnpm check` -- the three CI just ran -- and keeps everything that inspects the artifact:

- `assert_appimage_runs_on_x11`, the only check anywhere that the shipped AppImage still forces
  `GDK_BACKEND=x11`, which the reward overlay depends on
- `drop_bundled_wayland_client`, which repacks the AppDir

Those two run against the file users download. Nothing else covers them, so they are not
duplication and they stay. The flag defaults to off, so a developer running the script by hand
gets the full gauntlet exactly as before.

### release

Downloads both artifacts, attaches all four files to a draft. Unchanged behaviour: still a draft,
still `prerelease` from the tag, still hand-written notes.

`upload-artifact` roots a multi-path artifact at the least common ancestor of its globs, so the
Linux bundles arrive under `appimage/`, `deb/` and `rpm/` subdirectories rather than flat.
`fail_on_unmatched_files` turns a wrong guess about that into a failed release instead of a draft
quietly missing its Linux downloads.

Only this job keeps `contents: write`. The two build jobs are dropped to `contents: read`.

## Verification

Before pushing:

- `actionlint` clean on both workflows
- the check-name regexp run against the real check runs on the last release commit: matches CI's
  three jobs, excludes all four release jobs, so it cannot deadlock
- the count assertion run against that same commit: passes at three, and fails at two when a
  renamed job is simulated
- the tag comparison run over `v0.5.3`, `v0.5.3-rc1`, `v0.5.3-beta.2` (pass) and `v0.5.2`,
  `v0.6.0`, `v0.5.2-rc1` (fail)
- `build-linux-bundles.sh` run both ways against stub `cargo`/`pnpm`: `--skip-gates` invokes only
  the build, the default still runs tests, clippy and `pnpm check`
- the X11 assertion proved to still fire under `--skip-gates` by breaking the hook in a real
  AppDir and watching it exit 1
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace`, `pnpm check` all green

## Expected result

`max(windows ≈ 11m, bundles ≈ 12m)` behind a gate that costs whatever CI costs, most of which is
already spent by the time the tag lands. Roughly 20m wall clock, from 53m.

Nothing is checked less. The same clippy, the same tests, on the same commit -- once instead of
twice, with release blocked until they pass.
