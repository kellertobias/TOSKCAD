# MultiCAD

An open-source native parametric CAD application planned for macOS, Windows and Linux, with compiled Rust application code and native geometry adapters. The CAD application is not implemented yet.

The [product plan](https://plans.tokenet.de/CAD/48) is maintained in PLAINER. [CAD-107](https://plans.tokenet.de/CAD/107) establishes the local repository and kernel-neutral bootstrap. [CAD-49](https://plans.tokenet.de/CAD/49) must prove the native geometry/solver/UI foundation before dependency integration and desktop packaging in [CAD-50](https://plans.tokenet.de/CAD/50).

## Native project setup

Install Rust through rustup and the host linker tools, then run:

```sh
cargo build --workspace --locked
cargo run --locked -- --diagnostics
cargo test --workspace --locked
```

The checkout selects the pinned Rust 1.99.1 toolchain. This is a dependency-free repository bootstrap: the executable reports its version/target and the unimplemented CAD, UI and MCP capabilities. It does not launch the CAD application yet. See [BUILDING.md](docs/BUILDING.md) for prerequisites, lint/format commands, layout and validation boundaries.

[CI and releases](.github/workflows/release.yml) follow the ToskLight model: Forgejo is the authoritative source, its mirror sends commits to GitHub, and GitHub builds/tests macOS, Windows and Linux artifacts and mints tag-only hosted releases. Publication stays disabled until the repositories and mirror tag reconciliation are verified. No remote is configured and nothing has been pushed. See [RELEASING.md](docs/RELEASING.md).

## UX reference

Open [mockup/index.html](mockup/index.html) directly, or run `node mockup/serve.mjs` and visit `http://127.0.0.1:4173`. Run `node mockup/verify.mjs` for the prototype's illustrative checks. Node.js is used by prototype and release development tools, not by the native application.

The prototype illustrates **Parameters, Sketch, Model, Drawings, Render, Export**, contextual ribbons, and global Measure/Library/MCP controls. Use **Explore workflows** for examples. CAD geometry and outputs are simulated.

The latest agreed corrections are recorded in the [architecture boundaries](docs/architecture/0001-project-boundaries.md): Parameters has no sidebars/ribbon; Sketch starts in the 3D viewer and exposes editing tools only after New Sketch or opening an existing sketch; no Face Sketch command in Model; sheet metal includes attached fasteners. The prototype is preserved as a reference and has not been revised to implement these latest corrections.

See the [workflow guide and prototype boundaries](mockup/README.md).

## Contributing and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) before development. The application source is licensed under [GNU GPL v3 or later](LICENSE) (`GPL-3.0-or-later`). Future native dependencies require their own version/license/distribution review before integration.
