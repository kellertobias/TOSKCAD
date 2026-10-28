# Native occurrence-reference verification, 2026-10-29

Occurrence-aware sketch references passed exact native geometry, transactional
and macOS interaction checks. These results cover the fixtures below; human
workflow acceptance and Windows/Linux runtime verification remain separate.

## Source and build evidence

Production source through `e5a2f42` passed formatting, strict workspace/all-target
Clippy and locked offline workspace tests: 1,275 passed, zero failed and two
existing ignored tests. All 25 release-contract tests passed.

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline
npm test
CARGO_INCREMENTAL=0 ./build
```

Logs are `/private/tmp/multicad-cad128-final-{clippy,tests,npm,build}.log`.
Native builds require access to the shared OCCT build cache; a sandbox cache-lock
denial was resolved through the approved command escalation. The known upstream
OCCT `sprintf` deprecation remains outside the Rust lint scope.

| Bundle | Executable SHA-256 | Evidence |
| --- | --- | --- |
| `artifacts/development/build-9wss3z/MultiCAD.app` | `9ed750074f810afd01119f942d936479764afd14d5098b833189ad484e95925d` | Actual native pointer capture, Cancel, Apply, Undo/Redo, face section and Save As |
| `artifacts/development/build-5ZEsOm/MultiCAD.app` | `fcf21aa32e44532be460bc6cc7e2885abcb915cdb6fe6ccb0cfc6965d285e0af` | Final integrated build; fresh reopen, explicit native repair preview and Cancel |

The first bundle contains production through `39df611`. The final bundle includes
subsequent context guards, source-ownership revalidation and clearer occurrence
labels. Nothing was installed, pushed or published.

## Native geometry and transaction coverage

The [occurrence-reference decision](../architecture/0023-occurrence-sketch-references.md)
and [source-reference contract](../architecture/sketch-external.md) own the
architecture and supported capability boundaries.

| Behavior | Evidence |
| --- | --- |
| Independent translated/rotated copies and nested source/consumer paths | [Native review tests](../../crates/multicad-model/tests/occurrence_external_review.rs), two nested roots with different rotations and translations |
| Consumer-local projection cancels common ancestors once | Same native tests: endpoints (20, 20) to (100, 20) mm; a source parameter edit moves them to (35, 20) to (115, 20) mm |
| Exact finite line, arc, circle and rational/non-rational spline contacts | [Native kernel tests](../../crates/multicad-kernel/tests/sketch_references.rs), including finite domains, rotated planes, empty and coincident results |
| Face sections respect finite boundaries and holes | Kernel tests and real curved-face-to-closed-solid model fixture |
| Exact tilted conics and weighted spline projections | Native pole transforms preserve rational geometry without polyline fitting |
| Tangency, singular projection and changed result count reject capture | Kernel, model and [production backend tests](../../crates/multicad-app/tests/occurrence_reference_backend.rs) |
| Point references drive genuine downstream solids | Native panel volume changes from 1,000 to 1,400 mm³; broken source rejects evaluation; compatible explicit repair restores 3,800 mm³ |
| Canonical guesses and compatible output IDs survive source edits | Native spline/point fixtures, Undo/Redo and archive regeneration |
| Source and consumer ownership is revalidated when consuming receipts | [Occurrence tests](../../crates/multicad-model/tests/occurrence_external.rs); the membership regression failed with the guard removed and passed with it restored |
| Unconsumed broken references block a complete candidate | Real backend capture failure and explicit removal recovery, rather than publication of an otherwise valid solid |
| Latest-only background work, cancellation, stale Session/resources and worker panic recovery | Production worker tests, UI controller guards and real native request pipelines |
| Chosen context and actual world pose control sketch display | UI context tests, native leaf-view receipt checks and explicit pending-pose gates |

The native report remains bound to its source through Scene and SceneFeed.
Display meshes and native topology ordinals do not authorize persistent bindings.

## Native interaction and saved-source readback

The native app opened
`artifacts/external-reference-review/reference-start.multicad` through the system
file picker. Opening its top sketch exposed the References workspace. Project
reference was activated before clicking the box edge. Exact discovery returned
one explicitly selected 30 mm line. Preview enabled Apply after the complete
candidate check. Cancel restored zero elements at revision 0.

Repeating and applying committed revision 1. Header Undo restored the empty
sketch at revision 2; Redo restored the line at revision 3. Native Save As wrote
`projection-occurrence-native-gui.multicad` without replacing the input.

The wall sketch at Z = 5 mm used Intersect plane with the Faces filter. A viewport
click from the −Y view selected the actual box face. Exact discovery returned a
30 mm section. Explicit choice, Preview and Apply committed revision 4. Save As
wrote `projection-and-face-section-native-gui.multicad`.

Independent `model_file_review` reads rebuilt both saved projects with the
production native compiler and constrained solver. The final source report is
`artifacts/external-reference-review/projection-and-face-section-native-gui-source-report.json`.
It records both version-2 bindings, source Part identity, explicit occurrence
paths and native ancestry. Both references resolve to lines from (0, 0) to
(30, 0) mm in their respective local sketch planes. The source remains one exact
solid, bounded by (0, 0, 0) and (30, 20, 10) mm, with volume approximately
6,000 mm³. The review app was closed through native controls.

After specific launch approval, the final bundle reopened the combined saved
project in Sketch mode. The top sketch retained one 30 mm element and reported
the current native occurrence result. Repair reference required a fresh exact
query and explicit result selection. Preview displayed the source Part, world
context and checked reference transaction, then enabled Apply. Cancel restored
the fully constrained sketch, disabled Undo and removed the pending transaction.
This verification preview was discarded. The final review instance was quit
through native controls.

The no-overwrite `occurrence_reference_review` generator also produced twelve
editable projects and `manifest.json` under
`artifacts/occurrence-reference-review-20261029-0035/`. They cover nested copies,
placement updates, curved walls, explicit finite contacts, point-driven solids,
broken dependents, same-ID repair and weighted splines. Native tests independently
exercise these workflows. Generation alone is not physical GUI acceptance.

## Remaining boundaries

Each binding captures one explicitly chosen point or curve. General
spline-surface transversality classification is not verified. Unsupported native
families, singular/collapsed projections, lost support and incompatible output
families return diagnostics instead of choosing the nearest result.

Compatible single-reference repair preserves output identity. Multiple broken
references, dependent deletion or source creation may need one staged batch
before a fully valid candidate exists. This is tracked by
[CAD-134](https://plans.tokenet.de/CAD/134), a prerequisite of the generic
integration checkpoint before Room work. The complete-candidate gate remains
intact. These results do not establish low-power performance or universal kernel
robustness.
