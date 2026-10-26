# Building the bootstrap

## Prerequisites

Install [Rust through rustup](https://www.rust-lang.org/tools/install) and the host platform's C/C++ linker tools: Apple Command Line Tools on macOS, Visual Studio Build Tools with C++ support on Windows, or a C toolchain on Linux. Installing these system tools can require administrator access.

`rust-toolchain.toml` pins Rust **1.99.1**, Clippy and rustfmt. Rustup selects the toolchain inside the checkout and downloads it if missing; it does not require changing the global default. The pin was checked against the [official stable manifest](https://static.rust-lang.org/dist/channel-rust-stable.toml), dated 2026-10-15. Keep toolchain updates explicit and validate them on all CI targets.

The bootstrap has no crates.io dependencies and no Python, Node.js, FreeCAD, OCCT, solver or GUI runtime requirement. Node.js is development tooling for the separate UX prototype and release automation; it is not part of the compiled application. Cargo-only development does not require Node.js. Release tools require Node.js 22.14 or newer (CI uses 24) and `npm ci --ignore-scripts --no-audit --no-fund`.

## Commands

From the repository root:

```sh
cargo build --workspace --locked
cargo run --locked -- --diagnostics
cargo run --locked -- --version
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
```

The executable is `target/debug/multicad` (or `multicad.exe` on Windows); release output is under `target/release/`. With no arguments it prints help. The diagnostics command explicitly reports the repository-bootstrap stage and the unimplemented CAD/UI/MCP capabilities. It does not launch a GUI or open a modeling project.

After the pinned toolchain is installed, the current dependency-free workspace also supports `cargo build --workspace --locked --offline` and `cargo test --workspace --locked --offline`. Commit `Cargo.lock`; do not delete it as part of normal setup.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/multicad-core` | Kernel-neutral bootstrap metadata; shared modeling APIs arrive in their own plans. |
| `crates/multicad-app` | Native executable and command-line smoke tests. GUI integration is gated. |
| `docs/architecture` | Product boundaries and recorded architecture decisions. |
| `mockup` | Offline HTML/SVG UX reference, excluded from the native runtime. |

## What this setup does not validate

The native application, CAD kernel, sketch solver, viewport, MCP server, project file format and manufacturing exporters are not implemented. A successful bootstrap build does not satisfy their acceptance criteria.

The foundation gate [CAD-49](https://plans.tokenet.de/CAD/49) selects/proves native dependencies. [CAD-50](https://plans.tokenet.de/CAD/50) adds their clean bootstrap, complete desktop runtime bundling and installable platform packages. A whole-product `./build` wrapper and native `open/archive/install` behavior belong with that packaging work, when the actual product components and archive formats are known.

macOS local execution, Windows/Linux CI execution, physical GPU performance, installer behavior, signing and human workflow acceptance are separate results. No push, public release or installation is part of the repository bootstrap.

## CI

`.github/workflows/release.yml` defines quality checks and a native build/test/archive matrix for `ubuntu-24.04` (x64), `macos-15` (ARM64) and `windows-2025` (x64). Each build selects `rust-toolchain.toml`, tests the workspace, builds the release bootstrap and checks its compiled version, source commit and target. A separate job checks formatting, Clippy, release contracts and the prototype. Actions are pinned to verified upstream commit revisions, with read-only build permissions and no persisted checkout credentials.

Artifacts named `release-<target>-<commit>` retain versioned ZIP archives, identity manifests and SHA-256 checksums for seven days. The ZIP includes the executable, license, diagnostics and this guide. These are diagnostic bootstrap binaries, not desktop installers or a completed CAD application. `npm run release:package -- 0.1.0` builds a local host archive from committed source; it does not create a Git tag or publish anything.

Forgejo owns branches and mirrors them to GitHub. GitHub alone builds and mints tag-only releases on the tested commit. Publishing requires explicit activation variables and verified mirror tag reconciliation; both remain unset until repositories exist. Only the release job receives write permission. See [RELEASING.md](RELEASING.md) for topology, versioning, activation and recovery. There is no deployment or signing configuration.

The workflow can execute only after the repository is hosted on GitHub and a run is triggered. Local setup does not prove those hosted runs passed. Kernel/solver runtime bundling and full desktop artifacts remain CAD-50 work.
