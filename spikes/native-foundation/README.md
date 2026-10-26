# Native foundation laboratory (CAD-49)

This is a separate, locked Cargo workspace for proving dependencies before production integration. It is a real native Rust window and OCCT modeling process, but **not the production application**. The root application's bootstrap and `mockup/` remain unchanged.

## Run

Prerequisites: the root pinned Rust toolchain, CMake, a C++17 compiler/linker, Node.js for source acquisition only, and `tar`. No Python, FreeCAD, Qt or installed OCCT. The first build compiles OCCT and can take several minutes.

```sh
node tools/foundation-deps.mjs
CARGO_BUILD_JOBS=6 cargo build --manifest-path spikes/native-foundation/Cargo.toml --locked
cargo run --manifest-path spikes/native-foundation/Cargo.toml --locked -- --verify artifacts/foundation
cargo run --manifest-path spikes/native-foundation/Cargo.toml --locked -- --benchmark-ui artifacts/foundation
cargo run --manifest-path spikes/native-foundation/Cargo.toml --locked
```

On PowerShell, set `$env:CARGO_BUILD_JOBS = '6'` separately. The Rust workspace has no Python or Node runtime requirement. `foundation-deps.mjs` downloads only the official OCCT archive, validates its committed SHA-256 and extracts into ignored `artifacts/native-deps`. Cargo builds selected OCCT toolkits and their dependencies into the proof's `target/` directory. No system installation or hosted publication occurs. Rust registry sources/checksums are pinned by this workspace's own `Cargo.lock`; OCCT's tag, commit and archive digest are pinned in `native-dependencies.json`.

```sh
cargo fmt --manifest-path spikes/native-foundation/Cargo.toml --all -- --check
cargo clippy --manifest-path spikes/native-foundation/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path spikes/native-foundation/Cargo.toml --workspace --locked
```

## Boundaries

| Module | Owns |
| --- | --- |
| `kernel` | Narrow CXX ownership/exception boundary, valid-solid checks, rectangular profile extrusion, Boolean cut, tessellation and STEP I/O. No UI or document policy. |
| `lab/solver.rs` | Maps a constrained rectangle into the candidate Rust sketch solver and checks residuals. No solver-specific IDs escape. |
| `lab/geometry.rs` | Rebuilds two independent solids from one solved profile; records analytic volume/mesh/alignment/STEP checks and timing. |
| `lab/session.rs` | Versioned **proof-only** document, revision checks, Undo/Redo, dirty state and bounded/validated loading with atomic save. |
| `lab/ui.rs` | Native shell, mode descriptors, docking, input, pending/failed worker results and save/discard interaction. |
| `lab/viewport.rs` | Cached display meshes, projection, orbit/zoom, part picking and projected XY-plane selection. No manufacturing geometry. |

Geometry and solver run on one worker; C++ shape handles stay there. The worker returns owned mesh data. Failed/stale results do not replace the last valid model. Parameter edits require Apply; pending edits cannot be silently saved. Opening a file with unsaved changes requires explicit discard. Two bodies are displayed separated for inspection; this is a display offset, not an assembly placement feature.

The mode descriptor registry is only an initial experiment. It is **not** the production tool/module ABI: new tool rendering still needs UI integration here. Production typed command registration, general feature persistence, migration/recovery, dependency graphs and UI extension APIs belong to CAD-51/53/56/58. This proof format must not become the production format by accident.

## Verification limits

`--verify` produces actual STEP data and `geometry-report.json`: constrained-profile solids, valid tessellation, analytic cut volumes, STEP reimport, tiny exterior-contact offsets and invalid input/export failure checks. These fixtures do not prove all OCCT Boolean cases, stable topology naming, sheet-metal development, manufacturing readiness or large-model performance.

`--benchmark-ui` records CPU layout/tessellation at three pixel scales. It does not measure GPU presentation or end-to-end input latency. Native window interaction, idle CPU, accessibility and physical display transitions need separate evidence. The preliminary viewport uses CPU-projected triangles and a painter; a large-scene retained GPU renderer remains a separate gate.

Read [the architecture decision](../../docs/architecture/0002-native-foundation.md) before promoting any proof module. CAD-50 integration and production workspace items must not be marked complete from this laboratory.
