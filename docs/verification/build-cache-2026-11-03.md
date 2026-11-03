# Native build-cache verification, 2026-11-03

CAD-253 replaces the development Rust output directory and fixes bundle assembly
to use Cargo's actual output path. Commit `54dedf3` contains the implementation
on local `main`. No old artifacts were deleted and no application was installed
or published.

## Observed cause

The historical `target/debug/deps` contained 1,623,514 entries. A single directory
listing took 99,818 ms. All 273 compiler-worker samples in a three-second capture
were in `FileSearch::new -> readdir -> __getdirentries64`; the active compiler's
file descriptor and dependency-search arguments identified that directory.
Memory pressure was low. Evidence: `/private/tmp/cad213-rustc-sample.txt`.

The historical cause of that accumulated artifact count is unproven. Cargo
retains products from different source/configuration hashes; the fix supplies
an explicit maintenance lifecycle without deleting active build products.

## Configuration and maintenance

Repository Cargo commands now default to `target/cargo`. `CARGO_TARGET_DIR`
continues to override that path. The expensive native OCCT cache remains under
`target/native`, independently reusable. The development wrapper queries
`cargo metadata` rather than copying a hardcoded `target/debug/multicad`.
The standalone native-foundation CI job explicitly retains its own target
directory so its existing three-platform launch commands remain consistent.

[BUILDING.md](../BUILDING.md) documents idle-only `cargo clean` and use of a
fresh override when directory growth becomes excessive. No automatic live
cleanup runs. The existing GitHub tag-only publication policy is unchanged.

## Controlled compiler comparison

A standalone metadata-only probe used the same pinned Rust 1.99.1 compiler,
67-byte source and standard-library inputs. Only the dependency-search
directory and temporary output pathname differed. Both exited zero without
stderr and produced byte-identical 2,238-byte output.

| Dependency directory | Elapsed time |
| --- | ---: |
| Fresh development output | 0.038848 s |
| Historical output | 8.519709 s |

Exact commands, source/compiler identity, output hashes and timing are recorded
in `/private/tmp/cad253-rustc-probe-_sw_fs35/report.json`. This single paired
measurement isolates directory-search overhead under the current filesystem
cache and load; it is not a whole-workspace speedup claim. A separate fresh
directory listing found 25,688 entries in 16.47 ms.

## Build and packaging evidence

- `npm test`: 34 passed, zero failed; `/private/tmp/cad253-node-contracts.log`.
  Contracts exercise actual Cargo metadata for default and overridden paths.
- `CARGO_TARGET_DIR=/private/tmp/multicad-cargo-20261103-clean ./build`:
  successful workspace build in 24.02 s, retaining the existing native cache.
  `/private/tmp/cad253-final-wrapper-build.log` records the command result.
- Output: `artifacts/development/build-tSFP6G/MultiCAD.app`, containing native
  source subsequently committed as `1727eb7`.
- Bundle signing changes the executable's signature and related Mach-O size
  fields. Comparing the complete 256,796,256-byte pre-signature payload after
  normalizing only those three signature-size fields proved byte equality
  with Cargo's configured output. Original files were untouched. Evidence:
  `/private/tmp/cad253-wrapper-binary-proof.json`.
- Packaged `--diagnostics` exited zero. Packaged `--self-test` with a fresh
  output directory passed native construction, STEP round-trip and four
  bundled-example opens, then exited zero. Logs are
  `/private/tmp/cad253-final-diagnostics.json` and
  `/private/tmp/cad253-final-self-test.log`.

The sandbox initially denied the native-cache filesystem lock; the authorized
build succeeded with the required local access. This was an execution
restriction, not a product failure. macOS headless verification does not prove
Windows/Linux runtime or actual OS-window behavior. Human acceptance remains
separate from this agent verification.
