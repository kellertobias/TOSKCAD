# Native project file verification, 2026-10-27

Native Open, Save, Save As and explicit recovery preserved editable source and
embedded resources on the available Mac. Automatic recovery updated one session
slot while manual checkpoints and the saved project remained intact. These checks
verify persistence; they do not establish the general modeling, symbol-authoring
or manufacturing workspaces.

## Native binaries and fixtures

The native review executable was
`artifacts/development/MultiCAD-reference-review.app/Contents/MacOS/reference_review`.
Two binary SHA-256 identities separate the observed checkpoints:

| Binary SHA-256 | Observed scope |
| --- | --- |
| `1d129cb6feb3aeb48c24fb2f8f2bb1d84e8322889d279e73b21f91ff1d6442ca` | Legacy import, native Save As/Open/replacement Save, attached sketch parameter changes and mixed-resource preservation |
| `f4195b8c05a64d134c72cde6278eb512401480706ffdeefeb06d12420e6e8c77` | Automatic slot updates, manual checkpoint retention, explicit restored copy and native clean/dirty Quit behavior |

The latter binary includes the native Quit adapter and recovery-slot implementation.
The later shortcut-modal guards, readable recovery labels and STEP concurrency fix
have separate automated evidence below. Their final combined native spot-check is
recorded below.

The [fixture generator](../../crates/multicad-app/examples/project_file_review.rs)
creates a mixed-material assembly with stable parameters/parts/instances, two real
OCCT STEP boxes, a checked nested Door project pin, unavailable-provider definitions,
unknown manifest metadata and an advisory cache record. The nested pin exercises
identity and byte preservation, not a finished door library item. The cache bytes
are a preservation fixture, not a usable display mesh. Unsupported providers remain
unavailable in the application.

## Observed file workflows

1. Imported the existing attached-face JSON checkpoint explicitly and saved
   `artifacts/reference-review/face-sketch-native.multicad`. A fresh process opened
   that native file with a clean baseline. Applying height 18 to 24 mm moved the
   attached sketch origin from 21 to 27 mm and retained all four authored curves.
   Native replacement Save succeeded.
2. Opened `mixed-material.multicad` with a clean baseline. Applied width 600 to 700
   and saved `mixed-material-ui.multicad` through Save As. The checked reader
   confirmed unchanged critical payloads, pin metadata, cache records, unknown
   metadata, document identity and every original entity identity. The only added
   entity was the canonical Parameter table record.
3. In the saved 700 document, applied width 720 at revision 1 and created a manual
   checkpoint. Applied 730 at revision 2 and allowed automatic recovery to run.
   Both recovery entries were available while the saved project remained 700.
4. Applied 740 at revision 3 and allowed automatic recovery again. The existing
   automatic row became revision 3; total displayed entry count stayed four. The
   manual revision-1 row and its timestamp remained intact.
5. Restored the latest revision-3 entry. It opened a new unsaved tab at session
   revision 0 with width 740. Save proposed `Untitled.multicad`, confirming that
   restoration had not adopted the original project destination. Explicitly saved
   the copy as `mixed-material-restored.multicad`.
6. The checked reader confirmed that the restored copy retained the expected STEP
   payloads, pin hash/revision, cache metadata and unknown fields. The original
   `mixed-material-ui.multicad` still contained width 700 and the same complete
   source/resources as before recovery.
7. Ordinary window close displayed the unsaved-document guard. Native Cmd-Q on a
   dirty session displayed the same guard; Cancel kept the application running.
   Native Cmd-Q on a clean session exited. Pending-file-job close protection is
   covered by headless tests, not a physical pending-I/O Quit observation.

## Artifact identities

These SHA-256 values identify this local run. The generated artifacts are ignored
local outputs; regeneration allocates new UUIDs and can change STEP metadata.

| Local artifact under `artifacts/project-file-review/` | SHA-256 |
| --- | --- |
| `mixed-material.multicad` | `25f88b8d4c6b5269ee1d8090ec88eddd48b706292b4b00b1ae596942290a12e0` |
| `mixed-material-ui.multicad` | `c63ea2f6c8d717a020f81fa82a7cb350bc8e97b27b44cf34aba52670b8603176` |
| `mixed-material-restored.multicad` | `6aaca1e17394e90481bfaa317923f65e2a64c754f392df6f0d4377cacd2fed73` |
| `expected-resources.json` | `150abc836fd2e2d6d60b9e2c6988cc081abbe023057bafe7607aa9bc186839ee` |
| `verified-resources-ui.json` | `268a362b629525fb9342b6eaab757e593da1b289b06975ce2c66a5e0f023b92e` |
| `verified-resources-restored.json` | `44e1a0dfdd34b2ee2638221b94bd250d78530c0f1e129dba14cfba06e3d0c94b` |
| `verified-original-after-recovery.json` | `268a362b629525fb9342b6eaab757e593da1b289b06975ce2c66a5e0f023b92e` |

The original report's identical hash before and after recovery confirms that the
checked source/resource report remained identical. The restored report intentionally
changes the parameter value. Both reports retain the following critical resources:

| Resource ID | Kind | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `0ae3fae5-ea9e-4ebc-ba86-334b0ccc49f3` | STEP | 15422 | `5f99095e2c29aeecbb99302395789d700c301f78d29b0256261adc4c7e2aa121` |
| `df1e4f59-8f36-4ddd-b11f-6fda6f3d230a` | STEP | 15422 | `07f9f6162f899e1f4d8255e389ce7f3a9a53ee6e3e622ff901e6c81c85dfb2b5` |
| `559b05dd-1ea6-4a36-8118-72c07edffabc` | Nested project | 485 | `b610a0eb08e35c388f050edfe88ea9ac980d266ca3a7339d6323bd77488ef8e5` |

The symbol pin is `ffad8923-c6ee-48df-87c1-1164c7cce0f0`, revision
`door-revision-42`. Source document identity remains
`4b6d592e-aa16-49f1-ab60-7a8948a5b7e5` across the original and recovered copies;
local session/tab identities remain separate.

## Automated verification

Commands for the checked test surfaces and saved-file inspection:

```sh
cargo test -p multicad-core --test project_file --locked --offline
cargo test -p multicad-app files::native --locked --offline
cargo test -p multicad-ui application::files --locked --offline
cargo test -p multicad-kernel --test step_exchange --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
cargo run -p multicad-app --example project_file_review --locked --offline -- artifacts/project-file-review/mixed-material-restored.multicad
```

Container tests cover supported migration, checksums, limits, capability gates,
opaque data/pins and source/producer/schema/dependency cache rejection. Nine native
filesystem tests cover failed/stale/interrupted saves, external replacement during
serialization, lock contention, real STEP readback, explicit JSON import, 514
updates retaining one automatic slot, and newest discovery among 521 independent
snapshots without deleting any files.

Six file UI tests cover immutable delayed receipts bound to local tabs, Undo/Redo
while saving, resource-preserving Save As, unsaved restored copies, stale-slot
rejection, canonical-only debounce, manual/automatic separation, modal shortcut
ownership, pending-I/O Escape and readable age/clock boundaries.

Parallel fixture execution exposed a real native STEP translation crash. The
[exchange regression](../../crates/multicad-kernel/tests/step_exchange.rs) now passes
96 genuine roundtrips on eight independent owner threads, including unit, translated
bound and volume checks after failed reads/writes. The narrow translator guard is
pinned in commit `433e473`. Twelve repetitions of the previously crashing parallel
native file group passed all 108 file tests after the fix. General geometry remains
outside that lock; native handles retain thread ownership.

Full shared-workspace checks after the exchange fix passed 227 tests, zero failed,
and two intentional ignored tests. The native interruption child helper is invoked
by its parent test; physical GPU validation is separate. The subsequent readable-label
increment passed its six file UI tests and UI all-target Clippy. The final combined check below includes additional sketch tests.

## Final combined native checkpoint

The final executable SHA-256 was
`a827451fdf02165efe70f13955542396ac66f444ff54dfdcc7aa2677b736397f`
at the same review bundle path, built from source checkpoint `2394537`. It includes
the readable labels, file-modal shortcut guards and STEP translator protection.

The native recovery panel displayed the automatic revision-3 entry separately
from manual revision 1. Both showed `mixed-material-ui.multicad`; the observed
relative ages were 18 min and 23 min. Escape closed the panel without changing the
Model mode or source revision 0. This confirms the final panel layout and Escape
behavior physically, in addition to the earlier complete Save/Open/recovery checks.

The integrated workspace test run passed 262 tests, zero failed and two intentionally
ignored. Agent verification remains separate from human workflow acceptance.

## Limits

These are macOS observations. Windows/Linux runtime, filesystem durability and
low-power hardware performance require their own validation. The file codec does
not implement unavailable geometry, symbol or drawing providers. Cache consumers
must validate their own producer/dependency keys and decoded formats before reuse.
Advisory locks coordinate MultiCAD, while arbitrary external writers cannot receive
portable atomic compare-and-swap guarantees. Recovery discovery retains the newest
512 entries in memory; older files remain on disk. No recovery file is deleted
automatically. No push, installation, publication or human acceptance is implied.

The current [project-file reference](../architecture/project-files.md) owns format
and workflow semantics. [Container tests](../../crates/multicad-core/tests/project_file.rs),
[filesystem tests](../../crates/multicad-app/src/files/tests.rs) and
[UI tests](../../crates/multicad-ui/src/application/files.rs) own executable checks.
