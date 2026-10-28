# Native Move / Copy checkpoint, 2026-10-28

The production macOS app moved two Parts together, rotated an Instance through
its Object-axis ring, created a linked copy through the Inspector checkbox,
and moved that copy along World and Custom-datum axes. Saved projects regenerated
the same component geometry and independent occurrence poses.

The native UI operator supplied the interaction observations below. Independent
inspection of the saved native reports and executable digests supports the
artifact results. These are agent-run checks; human acceptance remains separate.
The earlier [numeric placement and alignment checkpoint](native-placement-2026-10-28.md)
owns the original formula and face-capture workflow.

## Executables and integrated checks

The reviewed bundles were local builds under `artifacts/development/` in the
primary checkout. The executable in each bundle was `Contents/MacOS/multicad`.
Its SHA-256 digest was independently checked with `shasum -a 256`.

| Bundle | Source evidence | Executable SHA-256 |
|---|---|---|
| `build-8mgPyJ/MultiCAD.app` | `b873996` | `e017abecd3c1ace8312b17a893fac8e1aab4aa3bd3b5a82f6a31ede5a6c719aa` |
| `build-CTvRKD/MultiCAD.app` | `b251414` | `0dd49a091f142d972f709e1e33b6db86dce30dd514eb5a2ca69ed953aa12f060` |
| `build-Fd6A5g/MultiCAD.app` | `e1ce67f` | `2bb6d06193518d80feec245a6383e00ff74abc49214ae3405a173c1d86893c1d` |

The final integrated source passed these commands:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

The test log records 948 passed, zero failed and two existing ignored tests.
The logs are `/private/tmp/multicad-coalesced-drag-{fmt,clippy,tests}.log`.
The count was checked against those logs on the checkpoint date.

## Native interactions and document history

The first bundle moved two selected Parts through the single Move / Copy tool.
A World-X arrow drag set their shared translation to 46.99215228426331 mm and
committed revision 1. Undo and Redo reached revisions 2 and 3.
Save As wrote `move-copy-batch-native.multicad`.

The second bundle reopened that file in a fresh session at revision 0.
The operator selected the existing Instance, chose Object coordinates and dragged
its Z ring. The Inspector showed 90.66591109835122 degrees at revision 1.
Undo and Redo reached revisions 2 and 3, followed by Save As to
`move-copy-rotation-native.multicad`.

With Create copy checked, a World-X arrow drag committed one new Instance at
revision 4. Its translation control was 71.77882385253906 mm.
Undo at revision 5 removed that copy; Redo at revision 6 restored it.
Save As wrote `move-copy-checkbox-native.multicad`.
The original Instance and both Part outputs remained unchanged by this copy.

The final bundle reopened the checkbox file at revision 0. In the positive-Z
view, a World-Y drag committed one edit at revision 1, with a Y translation of
66.04018020629896 mm. The camera stayed fixed through the edit.
The next placement pivot therefore showed Y=86.04018020629896 mm.

An attempted automated Option gesture produced a Move at revision 2.
Undo at revision 3 removed that attempt. A subsequent Custom YZ datum selection
displayed its X arrow along world Y. Dragging that arrow committed revision 4,
with an additional world-Y translation of 53.258216857910256 mm.
Save As wrote `move-copy-gizmo-native.multicad`.

The physical Option attempt does not verify modifier-copy behavior. The computer
control tool could not hold Option alone while dragging and serialized the
combined key/drag attempt. The reverted Move contributes no acceptance evidence
for Option; the raw pointer-event tests below cover the modifier contract.

## Saved native geometry

The four saved projects and matching `*-native-report.json` files are under
the primary checkout's `artifacts/model-review/` directory. The
[`model_file_review` reporter](../../crates/multicad-app/examples/model_file_review.rs)
loads a native project and invokes the production compiler and sketch solver.
Its occurrence bounds, topology and volume come from world B-reps.
Report inspection compared the JSON values directly, including canonical sources.

| Native report | Original Instance bounds, mm | New copy bounds, mm | Resolved reference entries |
|---|---|---|---:|
| `move-copy-batch-native-report.json` | (266.992152, 0, 25) to (326.992152, 40, 37) | No copy | 2 |
| `move-copy-rotation-native-report.json` | (276.644841, -10.230415, 25) to (317.339464, 50.230415, 37) | No copy | 2 |
| `move-copy-checkbox-native-report.json` | Same as rotation | (348.423665, -10.230415, 25) to (389.118288, 50.230415, 37) | 4 |
| `move-copy-gizmo-native-report.json` | Same as rotation | (348.423665, 109.067982, 25) to (389.118288, 169.528812, 37) | 4 |

The table rounds bounds to six decimal places. Each occurrence contains one
solid with rounded volume 28,800 mm³. All reports declare their current geometry
references resolved. The original occurrence's complete report object compares
equal between the rotation, checkbox and final gizmo reports.

Both Part report objects compare equal across all four files. The source block
remains bounded by (46.992152, 0, 0) and (106.992152, 40, 12) mm.
The pedestal remains bounded by (266.992152, 0, 0) and (346.992152, 60, 20) mm.
Their rounded volumes are 28,800 and 96,000 mm³ respectively, each with one solid.

The saved identities are:

| Entity | Identity | Relationship |
|---|---|---|
| Document | `ad0b1e0c-ed13-445a-a0e1-5f9e1a0fc396` | Retained in every saved project |
| Source Part | `072f0877-6d66-4eef-8654-51136fc7da45` | Component shared by both Instances |
| Source producer | `3317159f-8971-43ef-a857-f7a2d3f2a304` | Retained across all four reports |
| Original Instance | `c9c50d68-9159-4cb1-b00e-fae785f3a941` | Receives the Object-Z rotation |
| New Instance | `bd6d578e-5406-4c8f-b256-ce67f307fe9e` | Receives independent copied placement drivers |

The parameter objects also compare equal across the reports. The copied driver
chain retains its bound `=$distance + 20` and `=$gap` expressions, with independent
driver and reference identities. Its added translation controls leave the
original occurrence's driver chain unchanged.

The inspected report digests are:

| Native report suffix | SHA-256 |
|---|---|
| `batch` | `da3daf6b06f95a90c4c6d41e8435e771f80cdea5dd31798b73e1debb0c23b505` |
| `rotation` | `d7dfc800407f8fb1a35ee1766e9b2970d41dc471de3447ae5d79b9858730e4ed` |
| `checkbox` | `dcf57d27ef185c2af67ae3142cbb45718afd2a7073e9060b38cfca7f44ff6e96` |
| `gizmo` | `bb2867f3881e82f16944c824f3b850320666aa12adfa9c868702da684ff064dc` |

## Pointer regressions and remaining limits

The [native gesture tests](../../crates/multicad-ui/src/modeling/controller/placement_tests/gizmo.rs)
cover delayed native receipts, mixed Part/Instance movement and one transaction
per gesture. They verify Option/Alt copy with a same-frame modifier release,
native rotation and plane movement, Undo/Redo, cancellation, stale sources,
failed request submission and overlay ownership.

A fast vertical drag exposed a dropped-input defect in the second reviewed
bundle. The coalesced-input regression failed before the fix pinned by
`454f353` (isolated evidence commit `fb522cc`). The gizmo retains the actual
primary-down position and processes ordered motion samples before confirmation.
The final bundle physically passed the World-Y and Custom-datum drags above.

The regression reproduces the operator's camera, viewport, pivot and 2x scale.
It covers World Y and Custom YZ X-to-world-Y with separate and coalesced events,
including sustained hover. A coalesced 450-degree ring gesture preserves winding
and native geometry through one Undo.

The [target-acquisition tests](../../crates/multicad-ui/src/model_target_selection.rs)
cover selection-first and tool-first entry, Shift selection and Measure priority.
The [Object-basis tests](../../crates/multicad-ui/src/modeling/object_basis/tests.rs)
reject stale or forged poses and resolve authored Part frames.
[Native-bounds tests](../../crates/multicad-ui/src/modeling/placement/native_bounds_tests.rs)
verify initial selection-center pivots separately from tessellation bounds and
preserve explicitly saved pivots.

Physical Option/Alt dragging remains unverified. Windows/Linux interaction and
performance on low-power computers remain separate checks. This checkpoint
does not certify general CAD manufacturing output or assembly constraint solving.
