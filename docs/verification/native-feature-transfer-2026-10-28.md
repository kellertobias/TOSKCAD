# Native associative feature transfer checkpoint, 2026-10-28

The macOS application transferred a rail's holes into three receiving Parts,
edited their controls, rebuilt them after source parameter changes, and repaired
an explicitly missing source circle through current native source inspection.
Independent native reads of the GUI-saved files confirmed the resulting solids,
profile counts and canonical identities.

This is engineering verification. Human acceptance, manufacturing suitability,
connected MCP transport, Windows/Linux interaction and low-power performance
remain separate.

## Executable and automated evidence

Final production source is `1a72dad`. The review bundle is
`artifacts/development/build-6EsClD/MultiCAD.app`; executable SHA-256:
`35523fcbdf8670f7ccead40681c1c22927fd19d700b0d229952727f5a06cbf7b`.
The bundle was built locally with `CARGO_INCREMENTAL=0 ./build`, opened directly
for review and left running with the saved final project. The older review
instances were saved and closed. Nothing was installed, pushed or published.

Formatting, strict workspace Clippy and the locked workspace test suite passed:
1,171 tests passed, none failed, and two existing tests remained ignored. The
release-contract suite passed 25 tests. Local logs are:

- `/private/tmp/multicad-cad68-fmt-final-complete.log`
- `/private/tmp/multicad-cad68-clippy-final-complete.log`
- `/private/tmp/multicad-cad68-tests-final-complete.log`
- `/private/tmp/multicad-cad68-release-contracts.log`
- `/private/tmp/multicad-cad68-native-final-build.log`

The [core tests](../../crates/multicad-core/tests/model_transfer.rs) cover shared
assembly paths, current pose invalidation, repairable missing members, real
geometry feedback, forged deferred roles and receiving-history ownership. A
separate structural stress case bounds a malformed 8,193-component chain.
The [native model tests](../../crates/multicad-model/tests/transfer.rs) check actual
circle centers, volumes, stable derived topology, STEP/container reopening,
Undo/Redo, empty patterns and editable cutout clearances. The native evaluator
also tests antiparallel line/arc projection and explicit tilted rejection.

The [UI tests](../../crates/multicad-ui/src/transfer/tests.rs) and
[host tests](../../crates/multicad-ui/src/sketch_transfer_host_tests.rs) exercise
real source/candidate workers, pointer picks, exact scope guards, forged receipt
rejection, missing-hole repair, copy labels and camera restoration. Their final
focused run passed 16 tests. An eligible circle remains selectable where its rim
meets an unsupported cylinder seam.

## Editable rail and receiving materials

The [review generator](../../crates/multicad-app/examples/transfer_review.rs)
created nine editable projects and a checked manifest under
`artifacts/transfer-review-20261028-1946/`. A reusable assembly places the rail
above wood, printed and sheet receivers; an outer Instance adds translation and
rotation. The receiving settings are explicit:

| Receiver | Hole setting | Depth |
|---|---|---|
| Wood, 18 mm | 5 mm diameter override | 12 mm blind |
| Printed, 8 mm | Source diameter plus 0.2 mm radial clearance | Through |
| Sheet, 2 mm | 6.5 mm diameter override | Through |

Native cases change pitch from 32 to 40 mm, count from two to three, and the
rail member's X offset from 10 to 15 mm. The shared outer transform cancels
before computing receiving-local cuts. Source hardware remains independent.
The boundary case transfers only the captured face's outer wire and regenerates
the three receiving volumes as 184,800, 75,200 and 18,800 mm³ respectively.

## Native GUI edits and independent rereading

Opening `02-transferred.multicad`, selecting Printed receiver and Edit latest
feature reopened the complete saved transfer group. Changing the wood diameter
to 5.5 mm, Preview and Apply committed one transaction. Native Undo and Redo
buttons restored it; Save As produced `10-transfer-native-gui.multicad` at
revision 3. The source rail and other two receivers retained their geometry.

Parameters then changed pitch to 40 mm, count to three and rail offset to 15 mm.
Apply and rebuild committed them together. Save As produced
`11-upstream-native-gui.multicad` at revision 4. These values were independently
read from the saved canonical source, and every receiving feature reported three
actual native profiles.

The [model report reader](../../crates/multicad-app/examples/model_file_review.rs)
regenerated the GUI-saved files. All four physical leaves had resolved references:

| Saved GUI result | Rail, mm³ | Wood, mm³ | Printed, mm³ | Sheet, mm³ |
|---|---:|---:|---:|---:|
| Diameter edit, two holes | 10,173.805329 | 215,429.800933 | 95,485.281460 | 23,867.267710 |
| Source edits, three holes | 10,060.707993 | 215,144.701400 | 95,227.922189 | 23,800.901566 |
| Explicit repair, one hole | 10,286.902664 | 215,764.380551 | 95,742.640730 | 23,933.633855 |

Reports `10-transfer-native-gui-report.json`,
`11-upstream-native-gui-report.json` and
`13-final-repaired-native-gui-report.json` are in the same artifact folder.
They use exact B-reps rather than display mesh measurements.

## Missing-reference repair and full-model proof

Opening `08-missing-explicit-hole.multicad` displayed an explicit Missing
diagnostic. Its selected third source hole had been removed; no nearby hole was
silently substituted. Edit latest feature remained usable while full compilation
failed. Reselect opened the labeled component-local source view and showed the
current rail's single remaining hole.

Selecting that actual rim, Preview and Apply repaired the group in one
transaction. The final build automatically fitted the repaired complete preview
without a manual camera action. Save As wrote
`13-final-repaired-native-gui.multicad` at revision 1. Independent regeneration
found one profile per receiving feature and the repaired volumes above.

The source-container report retained group
`83777cca-98f5-4a1d-8cba-ec6df6e396d7`, all three target row/output IDs, frozen
input producers, receiving face IDs and occurrence paths. Capture
`a1d3a18f-bd96-4b89-a6e3-631793286106` kept its identity and resolved to the
explicit new source rim at local `(20, 10, 4)` mm with radius 3 mm. The detached
container report is `13-final-source-container-report.json`.

The final Inspector showed Independent drawer rail and Wood receiver as distinct
selected values despite identical default Instance names. Tests additionally
check eight duplicate-name paths and full UUID hover help. Relationships exposes
the implemented Transfer features tool with its own icon. Starting it with no
Part selected prompted for source and receivers; Cancel left revision 1 unchanged.

Apply requires a current complete native source/count/physical-path receipt.
Source-only inspection cannot authorize it. A retained consumed Boolean tool
definition is excluded from standalone physical output, while any placed copy
remains required by proof. Tests reject omitted receiving copies and phantom
extra leaves.

## Supported boundary

The [transfer decision](../architecture/0021-associative-feature-transfer.md)
defines Part-level machining, explicit path/capture ownership and pose dependency
projection. Initial native projection requires parallel or antiparallel axes and
planes. Nonzero outline offsets support circles and strictly convex line loops;
unsupported splines, tilted drilling and topology-changing offsets fail visibly.
Editing a receiving definition affects all of its placed copies.

Shared typed commands are exercised with attributable UI and MCP channels.
This checkpoint does not establish a running MCP server, general CAM, physical
fabrication approval or unrestricted offset geometry.
