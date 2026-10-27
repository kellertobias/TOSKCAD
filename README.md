# MultiCAD

The first native CAD dependency laboratory is under [spikes/native-foundation](spikes/native-foundation/README.md). It tests real constrained profiles, source-built OCCT solids/STEP, a Rust window and proof-only file/session behavior. It is separate from the native application host and does not establish completed application workflows.

An open-source native parametric CAD application planned for macOS, Windows and Linux, with compiled Rust application code and native geometry adapters. The native host and adapters are implemented; CAD workflows remain under development.

The [product plan](https://plans.tokenet.de/CAD/48) is maintained in PLAINER. [CAD-107](https://plans.tokenet.de/CAD/107) establishes the local repository and kernel-neutral bootstrap. [CAD-49](https://plans.tokenet.de/CAD/49) must prove the native geometry/solver/UI foundation before dependency integration and desktop packaging in [CAD-50](https://plans.tokenet.de/CAD/50).

## Native development

```sh
./build
./build open
./build check
./build archive
```

Windows uses `./build.ps1`. The wrapper acquires the pinned native/Cargo dependencies, builds every shipped component and assembles the host application. Archives include a native runtime layout, notices and matching corresponding source. `./build archive install` is the explicit macOS install action; it is not run automatically. See [BUILDING.md](docs/BUILDING.md) for prerequisites, output paths and validation limits.

The native host, owned OCCT adapter and solver are integrated. The [canonical document](docs/architecture/0004-canonical-document.md) provides stable identities, mixed-material parts/instances and a dependency graph. Typed parameters, transactional commands, a native parameter table and a datum-sketch starter are implemented. See the [native editing contract and limits](docs/architecture/0005-native-editing.md). [Native project containers](docs/architecture/project-files.md) preserve source and provider payloads, with atomic saves, migrations and explicit recovery copies. Solid modeling workflows and MCP transport remain under development. The foundation laboratory remains a separate reference, not the production UI.

[CI and releases](.github/workflows/release.yml) keep Forgejo authoritative and GitHub responsible for builds and tag-only hosted releases. Publishing remains disabled until the repositories and mirror tag reconciliation are verified. No remote is configured and nothing has been pushed. See [RELEASING.md](docs/RELEASING.md).

## UX reference

Open [mockup/index.html](mockup/index.html) directly, or run `node mockup/serve.mjs` and visit `http://127.0.0.1:4173`. Run `node mockup/verify.mjs` for the prototype's illustrative checks. Node.js is used by prototype and release development tools, not by the native application.

The prototype illustrates **Parameters, Sketch, Model, Drawings, Render, Export**, contextual ribbons, and global Measure/Library/MCP controls. Use **Explore workflows** for examples. CAD geometry and outputs are simulated.

The [current native layout](docs/architecture/0005-native-editing.md) has five modes, a full-width Parameters table with an action ribbon, and rendering under Export. Sketch tools appear after creating or opening a sketch; Model has no Face Sketch command. The prototype remains an illustrative reference and retains its earlier layout.

See the [workflow guide and prototype boundaries](mockup/README.md).

## Contributing and license

Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) before development. The application source is licensed under [GNU GPL v3 or later](LICENSE) (`GPL-3.0-or-later`). Future native dependencies require their own version/license/distribution review before integration.
