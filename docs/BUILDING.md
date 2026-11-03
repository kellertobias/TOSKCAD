# Building the native development host

## Commands

From any working directory, invoke the repository's `build` executable (Windows: `build.ps1`):

```sh
./build                 # acquire pinned source/dependencies; build every component
./build open            # rebuild; launch another native application window
./build check           # formatting, Clippy and tests
./build archive         # clean release products; runtime ZIP and corresponding-source ZIP
./build archive install # explicit macOS install/start into /Applications/MultiCAD.app
```

ZIP output supports the current host: macOS 13+ ARM64, Linux x64 or Windows x64. Other operating systems build on their own GitHub runners; the wrapper does not dispatch remote workflows. On PowerShell use `./build.ps1` with the same arguments. Install is not performed by any other command and has not been authorized/tested in this session.

## Prerequisites and bootstrap

Install Node.js 22.14+ for developer orchestration, Rust through rustup, CMake 3.16+ and the host compiler/SDK. macOS needs Apple Command Line Tools (`xcode-select --install`; CMake: `brew install cmake`). Windows needs Visual Studio Build Tools with C++ and Windows SDK. Linux needs a C++ toolchain plus development packages for Wayland/xkbcommon (Ubuntu: `sudo apt-get install build-essential cmake libwayland-dev libxkbcommon-dev`). System SDK/package-manager installation can need administrator interaction; the wrapper detects missing tools and prints the continuation rather than hiding global installs.

The wrapper selects Rust 1.99.1 from `rust-toolchain.toml` without changing the global default, acquires the checksum-pinned official OCCT source, fetches locked Cargo dependencies and builds the complete workspace. Node.js is build tooling only. The running application needs no Python, Node.js, FreeCAD, Qt or preinstalled OCCT.

OCCT is statically linked and compiled into a cache under `target/native/occt/<target>/<identity>`. Identity includes the source archive hash, compiler version/arguments, target, relevant SDK/ABI variables and build-configuration source. A filesystem lock serializes creation across Cargo check/test/build. Removing Cargo product output does not discard this expensive native cache. Rust package-manager caches remain normal Cargo caches. Cross-compiling from this Mac is not configured.

## Cargo output and maintenance

Repository Cargo commands default to `target/cargo/`. The historical
`target/debug/` and `target/release/` trees are preserved. A large dependency
directory can stall compiler directory reads despite idle CPU cores; this
occurred with 1,623,514 entries and a 99.8-second listing during CAD-253.

`CARGO_TARGET_DIR` can select a separate output directory. The development
wrapper and release packager obtain the executable path from Cargo metadata,
so they package the selected build rather than an older default-path binary.
The native OCCT cache remains at `target/native/` regardless of this override.

Before clearing Rust products, finish or cancel builds and tests using that
output directory. Then `cargo clean` removes the configured Rust output; it
preserves the separate OCCT cache, application bundles and project files.
For an immediate non-destructive recovery, choose a fresh `CARGO_TARGET_DIR`
for subsequent commands. Do not delete or move an active build directory.
Clean unused Rust products during idle maintenance if stale products accumulate
or directory reads become slow; no automatic cleanup runs while building.

## Outputs and runtime checks

Default builds assemble a new development layout under `artifacts/development/build-*/`; `latest.json` records its location. macOS includes `MultiCAD.app`; Windows/Linux include the native executable and resources. `open` starts a new host after a successful build and records its process. Existing editable windows keep their documents and close prompts; the wrapper does not terminate them or unrelated applications.

Archive builds require clean committed source, stamp the requested version/source SHA, run tests, clean/rebuild shipped release products and verify the packaged executable's version/identity, dependency closure and real STEP smoke result. `MULTICAD_RELEASE_VERSION` overrides the development archive version. ZIPs, manifests and SHA-256 checksums are under `artifacts/release/`:

- `multicad-<version>-<target>.zip`: native application layout and third-party notices.
- `multicad-<version>-<target>-sources.zip`: exact application source, lockfile, vendored Cargo sources, pinned OCCT archive and offline build/relink instructions.

Source archives are deliberately larger than runtime artifacts. Static LGPL OCCT does not remove its source/relinking obligations. macOS packages have local ad-hoc signatures and are not notarized; Windows/Linux packages are unsigned portable development layouts, not publisher-signed installers. Linux targets an Ubuntu 24.04+ desktop/system graphics stack; Windows requires a supported system graphics driver. The MSVC target uses the static CRT and packaging rejects imports of an unbundled VC runtime. No Windows/Linux binary is claimed tested until its runner actually executes.

`archive install` requires an explicit invocation. It refuses while an installed application is running, stages the new macOS bundle, retains the previous installation as a sibling backup, restores it on replacement failure and opens the new copy. Existing application configuration is not removed. `/Applications` must be writable through normal OS permissions; the script does not hide privilege escalation. Install/start is not part of the checks performed here.

Useful headless checks:

```sh
cargo run --locked -- --version
cargo run --locked -- --diagnostics
cargo run --locked -- --self-test artifacts/native-smoke
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

With no arguments `multicad` opens the native host. `--self-test` creates an actual constrained-profile solid, cut and STEP round-trip, including clean native shutdown; it is a runtime dependency smoke check, not a completed CAD workflow.

## Components and limits

| Component | Responsibility |
| --- | --- |
| `multicad-core` | Kernel/UI-independent core; canonical document and commands have their own plans. |
| `multicad-native-build` | Build-time OCCT source verification and native caching, excluded from runtime. |
| `multicad-kernel` | Opaque native-owned shapes and audited CXX boundary; general geometry policy remains CAD-54. |
| `multicad-solver` | Replaceable Rust constraint-solver mapping; broad sketch integration remains CAD-61. |
| `multicad-ui` | Native window host; full shell, viewport and domain modules remain CAD-57/58. |
| `multicad-app` | Composition/entry point, diagnostics and packaged runtime smoke checks. |

The canonical document/command registry, native project files, modeled workspaces, MCP, full manufacturing exports and production viewport are not implemented by CAD-50. The older `spikes/native-foundation` workspace stays a bounded reference and is not shipped as the product UI.

GitHub Actions builds three targets using the same package code. Read-only build/check jobs are separate from the gated tag-only publisher. Hosted execution, physical Windows/Linux behavior, signing, low-power performance and human acceptance remain separate evidence. GitHub is primary; see [RELEASING.md](RELEASING.md). Nothing is pushed automatically.
