# Building the bootstrap

## Prerequisites

Install [Rust through rustup](https://www.rust-lang.org/tools/install) and the host platform's C/C++ linker tools: Apple Command Line Tools on macOS, Visual Studio Build Tools with C++ support on Windows, or a C toolchain on Linux. Installing these system tools can require administrator access.

`rust-toolchain.toml` pins Rust **1.99.1**, Clippy and rustfmt. Rustup selects the toolchain inside the checkout and downloads it if missing; it does not require changing the global default. The pin was checked against the [official stable manifest](https://static.rust-lang.org/dist/channel-rust-stable.toml), dated 2026-10-15. Keep toolchain updates explicit and validate them on all CI targets.

The bootstrap has no crates.io dependencies and no Python, Node.js, FreeCAD, OCCT, solver or GUI runtime requirement. Node.js is optional for the separate UX prototype only.

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
