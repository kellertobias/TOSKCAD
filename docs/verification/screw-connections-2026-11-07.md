# Screw connection verification, 2026-11-07

All five connection modes passed actual macOS graphical creation, native Preview,
Apply and save. Each saved fixture reopened in a separate process, passed another
native Preview and survived Cancel without a document revision change.
Formatting, strict Clippy and the full locked workspace suite passed. The suite
recorded 2,939 passes, zero failures and four ignored tests. Human acceptance
remains separate.

The current executable contains source stamp `eddad648bf29e3ae1ffa4761494c5e55aa5b8460`.
Its SHA-256 is `2e12be362064fc8bab8cb9b227a59c4e9d0c4570b1a65955828be913d2f0c5d6`.
The [local application](../../artifacts/screw-connections-20261106/native-eddad64/MultiCAD.app)
was packaged and launched without installation or publication.
The [current GUI receipt](../../artifacts/screw-connections-20261106/checks/ux-eddad64-summary.json)
records a saved hanger reopen, ready preview, unchanged Cancel and clean quit.

## Native latency measurements

The [standalone default M3 screw](../../artifacts/screw-connections-20261106/checks/standalone-m3/summary.json)
was measured on the packaged `08d1b67` executable in three fresh MCP processes.
Creating the modeled 10 mm screw took a median 939 ms. An unchanged edit took
0.79 ms and adding a second identical screw took 2.20 ms. Changing the first
screw to 11 mm under the head while retaining 10 mm of thread took 606 ms.
These are stdio MCP request-to-response times, excluding pointer input and screen drawing.

At measured build `a2920b7`, the unchanged through-nut preview in the actual
application compiled its complete native model in 538 ms.
Fresh-process reconstruction took 5,128 ms.
These are model-worker measurements, including display generation, rather than
OS event-to-frame timings. The
[raw GUI timings](../../artifacts/screw-connections-20261106/checks/ux-a2920b7-through-reopen-timings.jsonl)
and [reopen receipt](../../artifacts/screw-connections-20261106/checks/ux-a2920b7-all-five-reopen-summary.json)
identify the saved graphical fixture and measured build.

The separate CLI comparison uses the same complete fitted M3 connection request
in three new processes per executable. The integrated executable is `c200f41`.
All returned native validation results are identical across the baseline,
prototype-only and integrated runs.

| Complete cold CLI connection | Median, seconds | Evidence |
| --- | ---: | --- |
| Fresh native thread construction | 25.231 | [Baseline](../../artifacts/screw-connections-20261106/checks/fitted-prototypes/baseline-summary.json) |
| Compiled fitted thread prototypes, original validation path | 12.266 | [Prototype comparison](../../artifacts/screw-connections-20261106/checks/fitted-prototypes/cold-comparison.json) |
| Compiled prototypes and parallel native collision checks | 4.685 | [Integrated runs](../../artifacts/screw-connections-20261106/checks/fitted-prototypes/integrated-c200f41-summary.json) |

The integrated CLI run includes approximately 3.19 seconds of hardware work and
1.23 seconds of connection validation. Unsupported prototype keys still use
native construction. These measurements establish neither universal cold
latency nor a whole-application speed guarantee.

The [paired native query checks](../../artifacts/screw-connections-20261106/checks/api-parallel-common-01/summary.json)
timed the same fitted screw and nut at 8.16 seconds serial and 0.83 seconds
parallel, averaging two observations per method. Both found no interference.
A deliberately wrong phase returned the same positive overlap volume with each
method. These are operation timings for one fixture, with other work allowed on
the machine.

The [native integration control](../../crates/multicad-model/tests/native_intersection_proofs.rs)
checks unchanged through-nut and tapped connections, changed engagement and a
new foreign collider. Its warm compiles retain identical native connection
receipts and physical leaf identities. A moved collider must execute a fresh
native query and fail without publishing pending proofs.

## Canonical geometry and reuse boundaries

The shared [core compositor](../../crates/multicad-core/src/model/screw_connections.rs)
owns each connection's hardware, axis and receiving features in one transaction.
Captured entry and far faces derive current stations and grip from native
geometry. [Live placement bindings](../../crates/multicad-core/src/model/screw_connection_kinematics.rs)
follow actual source dimensions and overrides while preserving stable role IDs.
The [MCP and CLI contract](../architecture/mcp-server.md#screw-connection-control)
documents the same editable request.

The [native validator](../../crates/multicad-model/src/screw_connections.rs)
continues checking physical receiver membership, metric phase, complete thread
overlap, engagement, bearing material, blind clearance and foreign collisions.
[Seating checks](../../crates/multicad-model/src/screw_connections/seating.rs)
verify both sides of bearing contact and the remaining usable tapped interval.
Head seats require a supported front entry; ambiguous or incompatible seats
return diagnostics. Custom wood hardware describes authored modeled geometry
without purchased installation or strength certification.

The [intersection proof store](../../crates/multicad-model/src/native_intersection_proofs.rs)
retains at most 256 exact empty-intersection results per evaluation-cache owner.
Keys include bounded lossless binary geometry digests and every operand/request
policy field. Nonempty results, failures, cancelled compiles and source-scoped
evaluations publish no proofs. Material and bearing witnesses remain native
operations. The store retains no native geometry handles.

The [independent asset audit](../../artifacts/screw-connections-20261106/checks/fitted-prototypes/implementation-verification.json)
records 20 compiled thread prototypes. The original 15 BRep payloads and all 172
Library documents remain byte-identical. Focused tests compare fresh and restored
native geometry, topology provenance, rounded roots, STEP readback and separate
owners. A source, policy or exact-input mismatch reconstructs normally.

## Actual graphical evidence

Open one of these projects in the local application to try the saved connections:

| Connection | Project created in the graphical app |
| --- | --- |
| Through-nut with grip captured from the far face | [Through-nut](../../artifacts/screw-connections-20261106/ux-fixtures/native-through-nut-autogrip-e59b28a.tcd) |
| M3 screw in a blind tapped receiver | [Tapped receiver](../../artifacts/screw-connections-20261106/ux-fixtures/native-tapped-gui-e59b28a.tcd) |
| Wood screw in a blind pilot hole | [Direct wood](../../artifacts/screw-connections-20261106/ux-fixtures/native-direct-wood-gui-e59b28a.tcd) |
| Machine screw in a threaded wood insert | [Wood insert](../../artifacts/screw-connections-20261106/ux-fixtures/native-wood-insert-gui-e59b28a.tcd) |
| Hanger bolt with separate wood and metric ends | [Hanger bolt](../../artifacts/screw-connections-20261106/ux-fixtures/native-hanger-gui-5821582.tcd) |

In Model, select the connection hardware and open its connection editor.
Run Preview to check the receiving cuts, then turn on **Exploded stack** to
inspect the hardware order. Cancel leaves the saved connection unchanged.
Use Save As before changing an example you want to keep.

The [reopen receipt](../../artifacts/screw-connections-20261106/checks/ux-a2920b7-all-five-reopen-summary.json)
records separate native processes and complete unchanged Cancel flows.
The [hanger lifecycle](../../artifacts/screw-connections-20261106/checks/ux-5821582-hanger-summary.json)
also covers a saved edit, stable ownership, Undo and Redo.

The `08d1b67` window check picked a real receiving-hole rim, highlighted the clamped
plate and revealed its receiving-cut card. Measure picked a committed nut face
with the connection form suspended. Closing Measure restored the complete
exploded layout automatically. The
[window receipt](../../artifacts/screw-connections-20261106/checks/ux-08d1b67-final-summary.json)
and accompanying screenshots record these checks.

Exploded views separate complete displayed bounds in hardware order and keep
receiving stock stationary. The
[native layout controls](../../artifacts/screw-connections-20261106/checks/ui-component-bounds-01/summary.json)
cover a 12/12 mm hanger and a 24 mm machine screw. Actual window checks also
covered section rendering and a 1100 by 700 window with visible action buttons.
The original window size and preferences were restored before shutdown.

The [first workspace run](../../artifacts/screw-connections-20261106/checks/final-workspace/result.json)
found two seating tests that read the zero station field of associative drivers.
Review found the same assumption about roll in the hanger test. Commit `9d9e333`
checks the evaluated hardware positions and phase.
All nine tests across the two native targets passed. The application source did not change.
The [second workspace run](../../artifacts/screw-connections-20261106/checks/final-workspace-live-pose/result.json)
passed both corrected targets and found one placement footer regression and
two native thread preview timeouts under concurrent UI tests. Both thread tests
passed [individually on the same binary](../../artifacts/screw-connections-20261106/checks/ui-timeout-isolation-9d9e333/result.json)
with their original deadlines.
Commit `eddad64` restores repeat Preview in ordinary modeling tools while the
screw connection editor suppresses duplicate requests for the same pending form.
The original placement test, the progress footer test and five mode controls
passed with their assertions intact. The
[workspace run on this commit](../../artifacts/screw-connections-20261106/checks/final-workspace-serial-ui/result.json)
ran every target and scheduled only the UI unit tests one at a time. Both
previously timed-out thread tests and the placement test passed in that run.
Windows/Linux runtime and human acceptance are outside this macOS verification.
