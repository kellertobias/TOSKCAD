# Contributing

Start with the [product contract](https://plans.tokenet.de/CAD/48) and the current PLAINER item's prerequisites and acceptance criteria. Keep independently reviewable changes in separate Conventional Commits, for example `feat(core): ...`, `fix(app): ...`, `test(app): ...` or `ci: ...`.

Follow the repository README for the available build and verification commands. Preserve the offline UX prototype and its documented boundaries. New dependencies need an explicit reason, version/lockfile review and license review. CAD/UI dependencies are selected by CAD-49, not by the initial bootstrap.

Changes should explain the problem, resulting behavior, verification and material limitations. Include a focused regression test for behavior changes, especially geometry/reference failures and file round trips. For diagrams or mockups, clearly distinguish illustrative results from actual CAD computation.

Contributions are made under `GPL-3.0-or-later`, the project's license. Keep third-party copyright and license notices. The future native dependency review must record any additional distribution obligations before those dependencies ship.

Do not include credentials, local project files, build output or private infrastructure configuration. This repository's current bootstrap work is local-only: no remote repository, push, release, signing or installation is part of it.
