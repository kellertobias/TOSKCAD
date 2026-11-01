# GitHub CI and desktop releases

## Repository and publication

GitHub is the primary repository at [kellertobias/TOSKCAD](https://github.com/kellertobias/TOSKCAD), as selected by the user on October 31, 2026. The release flow follows ToskLight: GitHub Actions tests and builds the application, then creates a `vVERSION` tag and hosted GitHub Release on the exact tested `main` commit. The publisher creates no release commit and leaves tracked manifests and changelogs unchanged. Forgejo mirroring is not part of this flow.

The release job alone has `contents: write` and uses the ephemeral GitHub repository token. Publication requires the repository variable `MULTICAD_RELEASES_ENABLED=true`. Unset or false keeps publication skipped while all checks and builds still run. Main/tag protection must permit this token to create release tags and assets. Tokens and signing keys never belong in source control.

## Version contract

| Commit | Release |
| --- | --- |
| `fix:` / `perf:` | Patch |
| `feat:` | Minor |
| `type!:` / `BREAKING CHANGE:` | Major |
| `docs:`, `test:`, `style:`, `refactor:`, `build:`, `ci:`, `chore:` without breaking intent | None |

Git tags are authoritative release versions. `Cargo.toml`/`Cargo.lock` retain the development workspace version; the private npm manifest has no product version and is never published. `MULTICAD_RELEASE_VERSION` and `MULTICAD_BUILD_COMMIT` stamp the built binary without changing tracked files. Archive names, embedded diagnostics and release manifests must agree with the selected version and tested SHA.

Like ToskLight, a history with no semantic tag uses a disposable `v0.0.0` analysis baseline on its root commit. It is removed before publication and never exported as a real release. The first feature selects `0.1.0`; a first patch selects `0.0.1`. A setup-only commit does not select a new version.

## Pipeline

`.github/workflows/release.yml` validates branch pushes, pull requests and manual builds. Tag pushes do not recursively trigger it.

- Metadata fetches full history and previews a release only for a main-branch push; other events use the latest reachable version or development version.
- Quality runs formatting, Clippy, actual release-engine contract tests and prototype checks.
- The reusable native-foundation workflow separately source-builds the CAD-49 laboratory on all three operating systems, runs its checks and retains proof reports/STEP output. It gates publication but does not add the lab to product release assets. Interactive desktop validation is recorded separately.
- The native matrix tests the workspace before building and smoke-checking version-stamped executables on Linux x64, macOS ARM64 and Windows x64, then produces versioned ZIPs, per-target identity records and SHA-256 checksums. Tests use the optimized `ci` Cargo profile with debug assertions and overflow checks retained. Shipped executables use the normal `release` profile.
- Windows splits the complete suite between two required jobs: example-regeneration tests and every remaining test. Both use the same substring filter from `tools/native-test-shard.mjs`, once as a positive filter and once with `--skip`. The two selections cover every test exactly once. Linux, macOS and local packaging run the full suite in one job. The Windows jobs have six-hour limits for cold native builds and geometry evaluation.
- Reused regression jobs run the named focused suites and budget probes in development mode. Their caller disables the duplicate full-suite run because the required package and Windows generator jobs already cover the entire suite. A standalone regression run also runs the complete suite.
- Publication waits for every quality/build result and the publication activation variable. It verifies all three archives' version/SHA/target/checksum records, rejects missing/stale/extra files and writes the aggregate release manifest.
- The publisher verifies the checkout/current GitHub main SHA and the expected version again before tagging. It creates no branch commit. Hosted release assets are only the verified ZIPs, checksums and aggregate manifest; success/failure issue comments are disabled.

Packaging produces **native development builds**, with Parameters, constrained Sketch, initial modeling tools and native project save/recovery. Advanced workspaces and manufacturing exports remain under development; the local MCP server is included (`multicad --mcp`); current interactive evidence is recorded in the [modeling checkpoint](verification/native-modeling-2026-10-28.md). Release jobs package each runtime ZIP with a matching corresponding-source ZIP containing exact application source, vendored Cargo dependencies, the pinned OCCT archive and rebuild/relink instructions. Aggregate verification requires both archives and their checksums; macOS uses ad-hoc signing without notarization. Node.js/npm are CI/developer release tools, not native runtime dependencies. The pinned release-engine/plugin versions match the inspected ToskLight lockfile.

## Local verification without publishing

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm test
npm run release:dry-run
```

The preview uses disposable local clones/bare storage; it neither changes source refs/index nor contacts GitHub/Forgejo for publication. Semantic-release may log an internal push-permission dry run against that temporary local storage; it does not push this repository. Tests exercise patch/feature/breaking/non-release histories and an already-tagged retry, plus publication/asset rejection conditions.

From a clean committed source checkout, `npm run release:package -- 0.1.0` produces the current host's native runtime and corresponding-source archives under `artifacts/release/`. It runs native tests, builds and executes that binary, and validates its version/SHA/target before archiving. This is local artifact generation, not a release. Remove/move older local output versions before aggregating a complete package set; verification deliberately rejects stale extras.

Completed releases are no-ops on repeated semantic analysis. An interrupted provider upload can leave a tag/draft requiring explicit inspection and repair of that same release; do not move/delete public tags or mint a different version to conceal it. Check the actual workflow run and hosted assets before reporting a release as available. CI builds and smoke tests are separate from interactive Windows/Linux testing.

## Project website publication

GitHub Pages serves `website/` at https://kellertobias.github.io/TOSKCAD/. The separate `.github/workflows/pages.yml` publishes a static artifact on website changes to `main` or a manual dispatch. Its deployment job has `pages: write` and `id-token: write`; application release activation remains independent.

The [download page](https://kellertobias.github.io/TOSKCAD/releases.html) reads published releases from the public GitHub API. It matches the versioned native archive names, links their corresponding sources and checksums, and displays an empty or error state when downloads are unavailable. JavaScript is required for the live list; a direct GitHub Releases link remains available.
