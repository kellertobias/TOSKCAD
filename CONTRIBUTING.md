# Contributing

Start with the [product contract](https://plans.tokenet.de/CAD/48) and the current PLAINER item's prerequisites and acceptance criteria. Keep independently reviewable changes in separate Conventional Commits, for example `feat(core): ...`, `fix(app): ...`, `test(app): ...` or `ci: ...`.

Follow [BUILDING.md](docs/BUILDING.md) for the available build and verification commands. Preserve the offline UX prototype and its documented boundaries. New dependencies need an explicit reason, version/lockfile review and license review. CAD/UI dependencies are selected by CAD-49, not by the initial bootstrap.

Changes should explain the problem, resulting behavior, verification and material limitations. Include a focused regression test for behavior changes, especially geometry/reference failures and file round trips. For diagrams or mockups, clearly distinguish illustrative results from actual CAD computation.

Contributions are made under `GPL-3.0-or-later`, the project's license. Keep third-party copyright and license notices. The future native dependency review must record any additional distribution obligations before those dependencies ship.

Release versions follow Conventional Commits: `fix`/`perf` are patch, `feat` is minor, and `!`/`BREAKING CHANGE` is major. Other types do not release by default. Forgejo owns branches; GitHub builds and mints tag-only releases. Never add a release commit or bump tracked manifests for a release. See [RELEASING.md](docs/RELEASING.md) for safe previews and activation requirements.

Do not include credentials, local project files, build output or private infrastructure configuration. This repository's current setup work is local-only: no remote repository creation, push, live release, signing or installation is authorized.
