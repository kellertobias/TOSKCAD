# ZA worm and generated-wheel verification

Commits `9eea007` and `30d9d36` implement the bounded native ZA worm and generated
wheel, canonical paired recipes, native editor and MCP controls. Commit
`8fb6677` corrects indexed 3MF seam connectivity. Focused native tests and actual
macOS creation, history, cold reopen and export workflows have retained evidence.
The independent audit of corrected GUI print files passes on `8fb6677`. Its
full locked workspace suite was interrupted after a front-panel 3MF export
regression. Commit `c30ab235` adds topology-proven native manufacturing seams
and the narrow native-pole exception. Focused native, model, front-panel, strict
Clippy and refreshed prototype checks pass. Its stamped release build, packaged
self-test, actual GUI print repeat and independent audit also pass. The
full suite completes with one buttress export failure. Commit `931cf7a` adds the
guarded internal-constraint correction. Its focused checks, refreshed prototype
audit, formatting and strict Clippy pass. Its stamped release, packaged self-test,
actual GUI print repeat and independent audit also pass. The final locked
workspace suite passes on `931cf7a`: 2695 passed, zero failed and four
intentionally ignored across 310 groups. Engineering verification is complete;
human acceptance, installation and publication are separate.

## Declared geometry and source boundary

[ADR-0034](../architecture/0034-generated-za-worm-wheel.md) owns the equations,
primary sources and approximation proof. The selected system is an axial-straight
ZA screw with a single-enveloping wheel generated from its hob. Houser and Su's
1999 translating-motion construction supplies the wheel envelope. KHK's guide
supplies axial-module/normal-pressure conventions; its Type III stock worm is
not relabeled as ZA. The cylindrical wheel blank and sharp generating-hob tip
are declared construction choices, without a manufacturer stock-blank claim.

The subset has axial module 0.5-2 mm, normal pressure angle 20 degrees, diameter
quotient 10, one right-hand start and 32-64 wheel teeth. Worm threaded length is
16-32 times module; wheel face width is independently 0.5-2 times module. Each
member's design backlash plus fabrication allowance is 0-0.1 times module.
Native bore/wall checks use the minimum root, including the wheel's torus root.
The [gear reference](../architecture/gears.md#za-worm-and-generated-wheel-subset)
and [MCP contract](../architecture/mcp-server.md#gear-control) define frames,
typed mate/quotient coupling, omitted edits and explicit conflict refusal.
Reader capability `model.gear.worm.1` covers direct, suppressed and template-only
recipes.

The source-position certificate and native validity checks have different
scopes. Six degree-16 polynomial patches are compared with their analytic source
at the true degree-25 Lobatto nodes using outward interval arithmetic. A source
tail and tensor interpolation bound, plus actual native half-knot coefficient
error, must fit half the requested `1e-7 mm` linear tolerance. The retained
[generic proof](../../artifacts/worm-review-20261105/verification-generic-proof-note.md)
and [actual-pole results](../../artifacts/worm-review-20261105/verify-degree16.json)
cover all six maps at modules 1/2 and counts 32/64, with corrupt-pole controls.
They do not certify normals or OCCT Boolean intersection curves. OCCT's
nonanalytic intersection machinery has its own edge tolerances, including a
`5e-6 mm` floor; tolerance metadata is not measured error. The native tests,
actual surface witnesses and exchange readbacks supply separate evidence.

## Native and protocol evidence

The [kernel tests](../../crates/multicad-kernel/tests/worm_gears.rs) measure actual
worm trapezoidal sections, generated wheel flanks/root transitions and material,
with native STEP readbacks and independent volume calculations. Boundary tests
include zero/positive bores, module/count/width limits and invalid dimensions.
The strict intersection control requires an empty native intersection for the
zero-thinning pair at the sampled phases; nearby positive controls retain real
intersection solids. These tests do not weaken the intersection predicate to
accept a small positive volume.

The retained [native intersection comparison](../../artifacts/worm-review-20261105/kernel-worm-parallel-comparison.log)
records the scoped parallel path at the implementation checkpoint. Correct-phase
queries take 2436-2564 ms sequentially and 422-436 ms in parallel; deliberately
interfering controls take 2914-3048 ms and 933-935 ms respectively. Both paths
retain the same empty/positive-solid checks and source bounds, volume and ancestry;
separate native owners also pass concurrent-thread controls. These measurements
cover the Boolean stage, not complete application latency or a p95 result.

The [model tests](../../crates/multicad-model/tests/worm_gears.rs) pass three tests
in 94.14 seconds in the retained
[model log](../../artifacts/worm-review-20261105/model-parallel-final.log).
They cover both member orders, actual world axes and STEP, full-turn sampling,
cache/current-pose behavior, atomic module-2 edits and saved reopen. Whole-solid
no-overlap is retained. Named native flank faces may establish an existence
witness within total authored thinning plus ten linear tolerances; a candidate
miss falls back to whole-solid distance. The reported witness is not a global
minimum or a loaded-contact certificate. Nine sampled phases over a complete
worm turn do not prove continuous contact or collision-free motion.

Focused editor and MCP checks pass under the optimized test profile with debug
assertions and overflow checks enabled:

| Retained check | Result |
| --- | --- |
| [UI worm controls and painted native lifecycle](../../artifacts/worm-review-20261105/checks/ui-worm-focused.log) | 2 passed, 112.11 seconds; both member orders, Preview/Cancel/Apply, history and independent saved reopen |
| [MCP parser and typed source conflicts](../../artifacts/worm-review-20261105/checks/mcp-worm-parser-focused.log) | 2 passed; equal numeric values do not authorize different literal/expression bindings |
| [MCP native worm workflows and legacy gears](../../artifacts/worm-review-20261105/checks/mcp-worm-and-legacy-gears-focused.log) | Worm 2 passed in 176.34 seconds; legacy gears 3 passed in 28.00 seconds |
| [Indexed 3MF export regressions](../../artifacts/worm-review-20261105/checks/3mf-index-fix-focused-final.log) | 4 passed in 6.05 seconds; native box, actual worm/wheel, exact build transforms and duplicate-vertex exceptions |

These test durations include multiple operations and are not interactive
latency measurements. Painted controller tests run offscreen and do not replace
the actual macOS workflow below.

## Actual macOS source workflow

The creation/history workflow uses source `30d9d36`, packaged executable SHA256
`41a359f969ed92ec02437ac7ccc9d4195e46e7a2bbe411419339639e0134244d`,
identified by [runtime.json](../../artifacts/worm-review-20261105/runtime.json).
The actual captures and accessibility receipts are retained under
`screens/` in the artifact root. The separate indexed-export runtime is identified
in the following section; its observations are not attributed to this binary.

| Actual operation | Observed result |
| --- | --- |
| Pair creation | Module 1, one worm start, 32 wheel teeth, worm length 16 mm, wheel width 2 mm and 2/4 mm bore diameters reach native Preview. Cancel leaves revision 0. Repeat Preview/Apply reaches 1; Save reaches 2. |
| Drive edit | Reopening the pair and setting worm drive to 37 degrees reaches native Preview/Apply at revision 3. Toolbar Undo/Redo reach 4/5; Save reaches 6. |
| Geometry edit | Unsupported normal pressure is refused. Restoring 20 degrees and doubling module, both axial dimensions, bore diameters, minimum walls and design backlash reaches native Preview/Apply at 7. Toolbar history reaches 8/9 and Save reaches 10. |
| Cold reopen | A fresh application session reopens the saved edited pair at revision 0 and rebuilds its native model. The retained source hash remains unchanged. |

[Independent saved-source comparison](../../artifacts/worm-review-20261105/independent-saved-worm-history.json)
proves that all 14 entity IDs, pair outputs, member relationships and reciprocal
typed count/quotient sources survive the three saved states. The drive save
changes only the drive and saved preview metadata. The geometry save changes
exactly five physical fields per member, plus preview metadata; wheel count
remains 32. No intermediate Undo TCD snapshot is invented from the toolbar
observation. These literal GUI edits do not establish GUI expression editing;
that has separate typed-source tests.

The [created](../../artifacts/worm-review-20261105/independent-created-source-native.json)
and [edited](../../artifacts/worm-review-20261105/independent-geometry-source-native.json)
readbacks bind raw checked TCD containers to actual stamped, read-only MCP native
processes. They independently derive member poses and compare the aggregate
native volume with analytic worm integration and a separately converged wheel
Green-integral calculation. The edited pair has center distance 42 mm, drive
37 degrees and two physical solids. Its analytic sum is 22113.36572094882 mm3;
reported native volume differs by about `5.60e-5 mm3`, below even the relative
policy term alone. The numerical wheel quadrature is a consistency oracle,
not a rigorous interval integration proof. Read-only processes exit normally.
[Cold-reopen binding](../../artifacts/worm-review-20261105/cold-reopen-source-immutability.json)
records byte-identical source before and after reopening.

## Actual exports and the indexed 3MF correction

The GUI adds one export set to the edited source. An
[independent source comparison](../../artifacts/worm-review-20261105/independent-export-config-source-binding.json)
finds all 14 preceding entities unchanged and exactly one new ExportSet.
The saved export source is `native-worm-pair-export-config.tcd`, SHA256
`a0efdf31982ffd69151141105acca0f4d64f29ed5a20fe4042a476c258d43b67`.
Actual GUI STEP export writes two placed solids, with UI actor/revision, source
hash and occurrence paths in its managed manifest.

The [independent STEP audit](../../artifacts/worm-review-20261105/independent-gui-step-audit.json)
uses a direct OCCT reader that does not link the MultiCAD adapter. Both files
contain one valid solid in millimeters. Analytic-volume differences are
`3.77e-6 mm3` for the worm and `1.73e-5 mm3` for the wheel, within their declared
area/volume consistency budgets. Maximum six-direction support-bound difference
is `4.52e-8 mm`, below the `2e-5 mm` readback budget. These are actual file
readbacks, separate from the exporter receipt.

The original GUI Print run writes a 3MF and two STL files from the posed pair.
The [strict indexed-topology audit](../../artifacts/worm-review-20261105/independent-3mf-indexed-topology-finding.json)
finds a real 3MF defect: per-face duplicate vertex indices leave 2682 worm and
7566 wheel boundary edges, despite exactly coincident coordinates forming closed
geometric surfaces. Coordinate closure and positive volume did not establish
indexed closure. Original payloads remain under `exports-before-index-fix/`.
The [negative control](../../artifacts/worm-review-20261105/independent-original-3mf-negative-control.json)
rejects those original bytes and distinguishes valid separately indexed
coincident closed components.

Commit `8fb6677` shares exact coordinate indices when fragmented faces require
it, verifies closed oppositely oriented indexed edges and splits disconnected
vertex fans. It preserves already closed independent index graphs, including
valid coincident components. Ambiguous fragmented edges fail rather than
inventing adjacency. No coordinate tolerance, moved vertex, deleted triangle
or consumer repair is used. The independent regression file
[three_mf_indexed_manifold.rs](../../crates/multicad-model/tests/three_mf_indexed_manifold.rs)
checks actual native box and worm/wheel exports, exact triangle positions,
winding and multiplicity, build transforms and the duplicate-vertex exception.

The corrected GUI Print run uses `8fb6677`, executable SHA256
`cb0d56516434fe8db74c7a5c7668104fed6114c0a53e2678e3930c218783cb65`,
identified by [runtime-index-fixed.json](../../artifacts/worm-review-20261105/runtime-index-fixed.json)
and the separate `screens-index-fixed/manifest.jsonl`. It cold-opens the owned
export source at revision 0, previews two placed objects, then reports three
written files under `exports-index-fixed/`. This proves execution of the fixed
GUI path. The [corrected independent audit](../../artifacts/worm-review-20261105/independent-index-fixed-final-summary.json)
passes two STEP and four mesh-object checks. Both stored 3MF resources have
closed oppositely oriented edges, cyclic vertex fans, one connected component
and Euler characteristic zero. Worm vertex count changes from 3420 to 2053 and
wheel count from 18840 to 14768; triangle counts remain 4106 and 29536. The
[package comparison](../../artifacts/worm-review-20261105/independent-index-fixed-package-binding.json)
proves that all expanded world triangle positions, order, winding and build
transforms are exactly preserved. Both STLs are byte-identical to their retained
originals. The original bad 3MF and its negative control remain separate evidence.
The audit does not test geometric self-intersections. Print warnings retain the
posed parts below the build plane; no automatic build-plate placement or
physical print success is claimed.
[Final window inventory](../../artifacts/worm-review-20261105/final-index-fixed-closed-apps.json)
records no running QA applications.

## Native seam transport and pole exception

Commit `c30ab235` addresses the front-panel refusal, reproduced independently
with a fused 43 by 86 by
4 mm plate, an offset 16 mm plug and two transverse blind pilot holes, followed
by a nontrivial 3D rotation and translation. Its raw face coordinates differ at
shared native seams, so exact coordinate identity alone cannot establish the
required index graph. The manufacturing path reconciles only nodes proven to
correspond on the same native TopoDS edge and edge parameter, or the same
native endpoint vertex. It does not pair unrelated nearby coordinates. Every
node displacement is bounded by `min(chord_deviation/16, linear_tolerance)`.
This is transport of a detached manufacturing mesh; the source BRep, persistent
topology, pose and ordinary display-mesh path remain unchanged.

A separate native pole exception handles zero-length degenerated edges. A
collapsed triangle can be removed only when its collapsed pairs belong to the
proved native degenerated-edge vertex group. Non-pole collapse is refused, each
original face must remain represented, and the final indexed edge/fan guards
still apply. The receipt records maximum displacement, native edge/vertex
associations and removed pole-triangle count. This is not a general sliver or
small-triangle removal rule.

The [native seam tests](../../artifacts/worm-review-20261105/checks/manufacturing-native-seams-final.log)
pass 3 tests in 0.04 seconds. The transformed blind-hole fixture moves mesh
nodes by at most about `1.281e-14 mm`, with no removed triangles. A sphere
requires removal of two native pole triangles; the doubly periodic torus
requires none. The [model integration tests](../../artifacts/worm-review-20261105/checks/manufacturing-model-indexed-final.log)
pass 5 tests in 6.77 seconds, including actual ZA manufacturing export. They
independently parse stored ZIP/XML, check indexed edge orientation and vertex
links, positive volume, build transforms, bounded node changes and unchanged
native topology/owner/revision/bounds/mass. Valid independently indexed coincident
components remain distinct; an ambiguous fragmented shared edge is refused.
The [front-panel regression target](../../artifacts/worm-review-20261105/checks/manufacturing-front-panel-final.log)
passes all 3 tests in 28.98 seconds, including rebuilding the shipped project
and its deliverables. The [independent source review](../../artifacts/worm-review-20261105/independent-native-seam-source-review.json)
passes the bounded topology transport, including native identity/parameter
correspondence, complete input index validation, deterministic representatives,
receipt bounds and the pole-only exception. It is source review, separate from
native execution. The packaged GUI repeat and its independent payload audit
are recorded below. The completed `c30ab235` full-suite result and subsequent
buttress correction are recorded separately.

## Packaged manufacturing repeat on c30ab235

The source-stamped release build on `c30ab235` passes in 3 minutes 22 seconds in
[release-build-native-seams.log](../../artifacts/worm-review-20261105/checks/release-build-native-seams.log).
The packaged [self-test](../../artifacts/worm-review-20261105/checks/self-test-native-seams.log)
passes diagnostics, actual native STEP output and opening all four bundled
examples. [runtime-native-seams.json](../../artifacts/worm-review-20261105/runtime-native-seams.json)
identifies executable SHA256
`9c2fab1748bc125872bcb403bf94e2cf36246193d922ec7e8dc20b29fcb54c44`
and exact source `c30ab2354dc8727aeb413c706cb74bac4e09e556`.

Eight actual captures in the separate
[screens-native-seams manifest](../../artifacts/worm-review-20261105/screens-native-seams/manifest.jsonl)
record cold-opening the owned export source at revision 0, selecting All and
per-Part STLs, and native Preview of two objects with 33642 triangles. The
native Save dialog targets `exports-native-seams/`. Completion reports three
written files at revision 0. The saved source remains byte-identical. This
repeats the Print path in the final native-seam runtime; it does not reattribute
the earlier creation/history workflow to that executable.

The [independent final audit](../../artifacts/worm-review-20261105/independent-native-seams-final-summary.json)
passes with zero findings. Its two STEP readbacks are explicitly the retained
`30d9d36` GUI files, read independently again; its four mesh-object checks cover
the new `c30ab235` GUI 3MF and STLs. Both 3MF resources are closed and oppositely
oriented by stored indices, with cyclic vertex fans, one component and Euler
characteristic zero. The [package comparison](../../artifacts/worm-review-20261105/independent-native-seams-package-binding.json)
finds maximum ordered world-triangle delta of exactly zero against the corrected
`8fb6677` baseline, including winding and transforms. Both STLs remain
byte-identical to the originals. The source hash, producer identities and poses
are bound by the audit metadata. Geometric self-intersections, physical fit and
print success remain outside this audit. The [final cleanup inventory](../../artifacts/worm-review-20261105/final-native-seams-closed-apps.json)
records no running QA applications.

## Buttress internal constraints and exact zero-area facets

The complete `c30ab235` suite exposes a buttress export regression, addressed by
commit `931cf7a`. The
positive-load/right-hand body has native triangles with two exactly equal XYZ
vertices. The manufacturing conversion removes only those exact zero-area
triangles, without an area threshold or epsilon, and requires every observed
native face to remain represented. Surviving positions, triangle order and
winding remain exact. The arbitrary `MeshObject` writer remains strict. The ordinary display mesh
remains unchanged. An independent actual right-hand fixture
checks that distinction; near-equal vertices do not qualify for this rule.

The positive-load/left-hand fixture instead has real open quadrilateral regions
inside three native face meshes. Node welding or adding guessed facets would
not establish the intended source surface. Only a typed final indexed-closure
failure permits one retry on another deep detached BRep copy. The retry calls
pinned OCCT `BRepTools::RemoveInternals(..., false)` and remeshes with the same
Watson algorithm, requested quality, tolerance and triangle budget. It protects
face/shell/solid occurrences, surface support and locations, and true boundary
coedge, endpoint and occurrence multiplicity. A wire container may disappear
only when all its coedges are INTERNAL constraints. Other correspondence,
validity, settings or budget failures do not receive this retry. The
[independent source review](../../artifacts/worm-review-20261105/independent-internal-cleanup-source-review.json)
passes these guards; it is separate from actual native execution.

The actual left fixture removes 15 internal meshing edges and produces 143872
triangles, with maximum node transport zero. This is a fresh native
tessellation, so its density legitimately differs from the original 139870
triangles. The model test compares exported positions, order and winding
exactly with the direct native manufacturing receipt, not with the original
holey mesh. Indexed closure and cyclic vertex fans, actual source bounds,
positive volume and native Boolean material probes are independently checked.
The right-hand exact-survivor proof remains unchanged.

One intermediate left regression fails its final source-byte assertion after
performing Boolean reference probes on the source itself. The retained
[before](../../artifacts/worm-review-20261105/debug-left-source-right.brep) and
[after](../../artifacts/worm-review-20261105/debug-left-source-left.brep) files
differ only in the root Compound's Free flag. The corrected test obtains its
Boolean oracle by independently reading the original BRep. It keeps immediate
byte-equality assertions after direct manufacturing and the model helper, plus
the final BRep/topology/session/raw-mesh assertions. No flag normalization or
ignored source-byte difference is introduced.

The [kernel target](../../artifacts/worm-review-20261105/checks/internal-cleanup-kernel-first.log)
passes 4 tests, including both buttress hands, in 38.46 seconds. The full
[buttress target](../../artifacts/worm-review-20261105/checks/internal-cleanup-model-final.log)
passes its 2 tests in 173.83 seconds, covering all four variants and real
STEP/Print outputs. That command subsequently fails the intermediate left
source-flag test, so it is not presented as a successful command overall.
After isolating the oracle, the [left regression](../../artifacts/worm-review-20261105/checks/internal-cleanup-left-isolated.log)
passes in 22.39 seconds and all 7
[independent 3MF regressions](../../artifacts/worm-review-20261105/checks/internal-cleanup-model-indexed-final.log)
pass in 47.77 seconds. The [front-panel rerun](../../artifacts/worm-review-20261105/checks/internal-cleanup-front-panel-final.log)
passes all 3 tests in 28.99 seconds. These results cover the correction committed
as `931cf7a96202fa71b798b490ace81b8a944f4f43`. The packaged GUI and independent
payload repeat below pass. The completed full-suite result is recorded below.

## Final packaged repeat on 931cf7a

The [source-stamped release build](../../artifacts/worm-review-20261105/checks/release-build-internal-cleanup.log)
passes in 2 minutes 44 seconds. Packaged diagnostics and the
[native self-test](../../artifacts/worm-review-20261105/checks/self-test-internal-cleanup.log)
pass; no self-test elapsed-time measurement is claimed.
[runtime-internal-cleanup.json](../../artifacts/worm-review-20261105/runtime-internal-cleanup.json)
identifies source `931cf7a96202fa71b798b490ace81b8a944f4f43` and executable SHA256
`3598daaa11d7fd5e5a1a3ade86491c8609a30cc585a88ce1225d382567591fbb`.
The [running-process receipt](../../artifacts/worm-review-20261105/running-final-app-process.txt)
identifies the actual application process used for this check.

Seven captures in the [final manifest](../../artifacts/worm-review-20261105/screens-internal-cleanup/manifest.jsonl)
record cold-opening the same owned source at revision 0, selecting per-Part
STLs and All, native Preview of two objects with 33642 triangles, and the native
Save dialog targeting `exports-internal-cleanup/`. Completion reports three
written files at revision 0. The [final source/process receipt](../../artifacts/worm-review-20261105/final-internal-cleanup-source-and-process.json)
records unchanged source bytes and that the application process is gone after
Quit. The [application inventory](../../artifacts/worm-review-20261105/final-internal-cleanup-closed-apps.json)
is empty.

The [final independent audit](../../artifacts/worm-review-20261105/independent-internal-cleanup-final-summary.json)
passes with zero findings. It checks two retained `30d9d36` STEP files separately
from four new `931cf7a` print mesh objects. The two stored-index 3MF resources
are closed and oppositely oriented with cyclic vertex fans, one component each
and Euler characteristic zero. Their vertex/triangle counts match the preceding
corrected package. The [binding comparison](../../artifacts/worm-review-20261105/independent-internal-cleanup-package-binding.json)
finds ordered world-triangle delta exactly zero against `c30ab235`, including
winding, producer identities and poses; both STLs remain byte-identical to the
originals. The original bad-index negative control remains retained. Neither
this audit nor the GUI write proves geometric self-intersection freedom,
physical fit, loaded behavior or successful printing.

## Existing thread prototypes and required checks

The `30d9d36` native source closure has a regenerated trusted metric prototype
fingerprint. Its [independent prototype audit](../../artifacts/worm-review-20261105/independent-prototype-audit.json)
binds source fingerprint
`c4a4772785ffa0728c7e3eeac216823ecfd49fd4376880cf50576efcb93a0951`
to pinned OCCT source identity, all 19 authored catalog rows and 15 unique native
assets totaling 11294004 bytes. All 15 BReps and recorded masses remain identical;
only the source fingerprint changes. This inventory audit does not itself run
OCCT. Five [native prototype guards](../../artifacts/worm-review-20261105/checks/prototype-native-guards.log)
pass separately, including restoration, actual STEP and native intent checks.
No new metric-fastener latency measurement is inferred from those passes.
For `c30ab235`, the [fresh prototype audit](../../artifacts/worm-review-20261105/independent-prototype-audit-native-seams.json)
binds the updated native closure to fingerprint
`feefa0e0d49c0646db3ffab39cce989531390425dcf6c9018ae0cbc782cf7dba`.
All 15 BReps, masses and ancestry remain exact; registry metadata changes only
the source fingerprint. Fresh generation uses eight workers and takes
26.621974375 seconds in the retained
[generation log](../../artifacts/worm-review-20261105/checks/prototype-generation-native-seams.log).
Five [native restoration/intent guards](../../artifacts/worm-review-20261105/checks/prototype-native-seams-guards.log)
pass in 39.50 seconds. These supersede the earlier source-closure checkpoint;
they do not constitute an interactive fastener latency measurement.
For `931cf7a`, the [independent prototype audit](../../artifacts/worm-review-20261105/independent-prototype-audit-internal-cleanup.json)
passes source fingerprint
`9db4950707194cc40b8b96d58cd9db7984a79d34ec16cbecc5b6286cccccd46f`.
Eight-worker [fresh generation](../../artifacts/worm-review-20261105/checks/prototype-generation-internal-cleanup.log)
finishes in 23.409926125 seconds. All 15 BReps, masses, bounds and ancestry remain
exact; registry metadata changes only the source fingerprint. The audited
registry is installed, and all 5 [native prototype guards](../../artifacts/worm-review-20261105/checks/prototype-internal-cleanup-guards.log)
pass in 40.30 seconds. Earlier fingerprints remain scoped to their historical
source checkpoints. No interactive latency guarantee follows from this audit.

The initial full locked suite on `30d9d36` ends with exit 101: 2682 passed,
two failed and four intentionally ignored across 308 groups. Its
[summary](../../artifacts/worm-review-20261105/checks/final-workspace/workspace-summary.json)
is retained unchanged. The two failures are existing painted BSPT and buttress asynchronous wait
timeouts. An [isolated rerun](../../artifacts/worm-review-20261105/checks/thread-timeouts-isolated.log)
passes all seven selected tests in 69.98 seconds with the original 45-second
per-wait deadlines unchanged. That rerun does not turn the initial full command
into a pass.

The superseded command on `8fb6677` was:

```sh
cargo test --workspace --locked --no-fail-fast -- --test-threads=4
```

Its [run receipt](../../artifacts/worm-review-20261105/checks/index-fixed-workspace/workspace-run.json)
uses the pinned toolchain, committed lockfile, optimized test profile, debug
assertions and overflow checks. It was intentionally stopped after 125 completed
groups: 884 passed, one failed and one ignored, with process exit `-2` (SIGINT).
The [interruption receipt](../../artifacts/worm-review-20261105/checks/index-fixed-workspace/interruption.json)
preserves the reason; these partial totals are not a complete workspace result.
`front_panel::generator_rebuilds_the_shipped_project_and_its_deliverables`
fails when exporting the Left end part because exact coordinate identity cannot
close its indexed boundary. The stricter writer refuses that indexed
boundary. The focused native/model checks and complete front-panel target
subsequently pass with the topology-based correction described above. The
interrupted full command remains a failed partial record. The subsequent
`c30ab235` full command completes as recorded below; focused passes do not
establish a full workspace pass.
On `c30ab235`, [formatting](../../artifacts/worm-review-20261105/checks/fmt-native-seams-final.log)
and [strict Clippy with the refreshed registry](../../artifacts/worm-review-20261105/checks/clippy-native-seams-with-registry.log)
pass. The replacement full command uses the same optimized test profile,
assertions/overflow checks and four test workers. Its
[run receipt](../../artifacts/worm-review-20261105/checks/native-seams-workspace/workspace-run.json)
identifies exact source `c30ab2354dc8727aeb413c706cb74bac4e09e556`.
The [completed summary](../../artifacts/worm-review-20261105/checks/native-seams-workspace/workspace-summary.json)
records exit 101: 2691 passed, one buttress failure and four intentional ignores
across 310 groups. This run is complete, not interrupted. The stamped release,
packaged self-test and GUI/audit passes above remain valid for their scoped
operations; they do not erase the failed workspace gate.

On `931cf7a`, [formatting](../../artifacts/worm-review-20261105/checks/fmt-internal-cleanup-final.log)
and [strict workspace Clippy](../../artifacts/worm-review-20261105/checks/clippy-internal-cleanup-final.log)
pass. The [new full-suite run receipt](../../artifacts/worm-review-20261105/checks/internal-cleanup-workspace/workspace-run.json)
identifies exact source `931cf7a96202fa71b798b490ace81b8a944f4f43`, the same
optimized test profile with assertions/overflow checks and four test workers.
The [final summary](../../artifacts/worm-review-20261105/checks/internal-cleanup-workspace/workspace-summary.json)
records exit zero: **2695 passed, zero failed and four intentionally ignored
across 310 groups**. The run starts at `2026-11-06 00:44:19 UTC` and finishes at
`01:43:43 UTC`. The ignored cases are one subprocess helper exercised by its
parent, the optional external Blender integration, and two explicit GPU tests.
The UI group passes 994 tests with two GPU ignores in 133.45 seconds; its
original 45-second preview deadlines are unchanged. The stamped release,
packaged self-test, final GUI repeat and independent payload audit also pass.
Older release, self-test and export evidence retains its recorded source scope.

Windows/Linux runtime, CI, physical printing or loaded gearing, manufacturing
fit and explicit human acceptance remain unverified. Native generation and
repeated sampled Boolean work remain substantial; no near-instant worm-pair
interaction or universal performance guarantee is claimed.
