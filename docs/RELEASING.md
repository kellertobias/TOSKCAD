# Forgejo source, GitHub CI and releases

## Ref ownership

This follows ToskLight's repository model: the developer upstream (`origin`, once created) is Forgejo. Its infrastructure-managed push mirror sends branches to GitHub. GitHub Actions runs the builds/tests and creates `vVERSION` tags plus hosted GitHub Releases on the exact mirrored, tested commit.

GitHub must not create release commits, update tracked manifests/changelogs or push branches back to Forgejo. There is no Forgejo Actions release workflow and no Forgejo credential in GitHub. The repository URLs/owner names are not guessed: the repositories do not exist yet, no remote is configured, and the publisher derives the GitHub mirror from `GITHUB_REPOSITORY` when Actions eventually runs.

## Activation prerequisites — publication is off by default

Before enabling releases:

1. Create the authoritative Forgejo repository and GitHub build/release mirror when authorized. Configure the actual developer remote and a main-branch push mirror; do not treat GitHub as a second developer upstream.
2. Verify that the **deployed** mirror imports eligible GitHub-only tag objects into Forgejo before every outward mirror. The exact tag must resolve to a commit already reachable from a Forgejo branch. Branches remain Forgejo-owned; conflicting tag objects must fail visibly rather than be force-overwritten.
3. Verify a branch round trip and a test tag reconciliation in the actual infrastructure. Source inspection or ToskLight's existing setup does not establish that a new repository has this behavior. Without this prerequisite, stop before publication: an outward mirror could otherwise remove GitHub-minted tags.
4. Confirm main/tag protection allows the ephemeral GitHub repository token to create release tags and hosted release assets; the release job alone has `contents: write`.
5. Set the GitHub repository variable `FORGEJO_TAG_RECONCILIATION_VERIFIED=true` only after that verification, then `MULTICAD_RELEASES_ENABLED=true` when release publication is wanted.

Neither variable is preset here. Unset/false variables keep publication skipped while builds/checks still run. No repository, mirror, secret, tag, hosted release, signing setup or push is created by this local configuration. No token values belong in source control.

## Version contract

| Commit | Release |
| --- | --- |
| `fix:` / `perf:` | Patch |
| `feat:` | Minor |
| `type!:` / `BREAKING CHANGE:` | Major |
| `docs:`, `test:`, `style:`, `refactor:`, `build:`, `ci:`, `chore:` without breaking intent | None |

Git tags are authoritative release versions. `Cargo.toml`/`Cargo.lock` retain the development workspace version; the private npm manifest has no product version and is never published. `MULTICAD_RELEASE_VERSION` and `MULTICAD_BUILD_COMMIT` stamp the built binary without changing tracked files. Archive names, embedded diagnostics and release manifests must agree with the selected version and tested SHA.

Like ToskLight, a history with no semantic tag uses a disposable `v0.0.0` analysis baseline on its root commit. It is removed before publication and never exported as a real release. The first feature selects `0.1.0`; a first patch selects `0.0.1`. There are no existing MultiCAD releases to renumber. The current setup-only commits correctly produce no release.

## Pipeline

`.github/workflows/release.yml` validates mirrored branch pushes, pull requests and manual builds. Tag pushes do not recursively trigger it.

- Metadata fetches full history and previews a release only for a main-branch push; other events use the latest reachable version or development version.
- Quality runs formatting, Clippy, actual release-engine contract tests and prototype checks.
- The reusable native-foundation workflow separately source-builds the CAD-49 laboratory on all three operating systems, runs its checks and retains proof reports/STEP output. It gates publication but does not add the lab to product release assets. Until hosted runs exist, this is configuration rather than platform validation.
- The native matrix tests/builds/smoke-checks version-stamped executables on Linux x64, macOS ARM64 and Windows x64, then produces versioned ZIPs, per-target identity records and SHA-256 checksums.
- Publication waits for every quality/build result and both activation variables. It verifies all three archives' version/SHA/target/checksum records, rejects missing/stale/extra files and writes the aggregate release manifest.
- The publisher verifies the checkout/current GitHub main SHA and the expected version again before tagging. It creates no branch commit. Hosted release assets are only the verified ZIPs, checksums and aggregate manifest; success/failure issue comments are disabled.

Current archives are **native development hosts**, with the linked kernel/solver and native window. CAD workspaces and native project files remain unimplemented. Each runtime ZIP has a matching corresponding-source ZIP containing exact application source, vendored Cargo dependencies, the pinned OCCT archive and rebuild/relink instructions. Aggregate verification requires both archives and their checksums; macOS uses ad-hoc signing without notarization. Node.js/npm are CI/developer release tools, not native runtime dependencies. The pinned release-engine/plugin versions match the inspected ToskLight lockfile.

## Local verification without publishing

```sh
npm ci --ignore-scripts --no-audit --no-fund
npm test
npm run release:dry-run
```

The preview uses disposable local clones/bare storage; it neither changes source refs/index nor contacts GitHub/Forgejo for publication. Semantic-release may log an internal push-permission dry run against that temporary local storage; it does not push this repository. Tests exercise patch/feature/breaking/non-release histories and an already-tagged retry, plus publication/asset rejection conditions.

From a clean committed source checkout, `npm run release:package -- 0.1.0` produces the current host's native runtime and corresponding-source archives under `artifacts/release/`. It runs native tests, builds and executes that binary, and validates its version/SHA/target before archiving. This is local artifact generation, not a release. Remove/move older local output versions before aggregating a complete package set; verification deliberately rejects stale extras.

Completed releases are no-ops on repeated semantic analysis. An interrupted provider upload can leave a tag/draft requiring explicit inspection and repair of that same release; do not move/delete public tags or mint a different version to conceal it. Hosted API behavior, private mirror access, protected tags, deployed reconciliation, Windows/Linux archives and actual GitHub runs cannot be validated until the repositories and infrastructure exist.
