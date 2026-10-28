# Native assemblies and inline Inspector checkpoint, 2026-10-28

The native macOS application placed reusable nested components, edited a
member's geometric joint anchor, regenerated that reference after a parameter
change, and moved/copied a whole subassembly through its viewport gizmo. The Part
Inspector also passed comma-to-badge entry, inline confirmation/discard and
immediately applied appearance choices with independent Undo/Redo.

This is engineering verification. Human acceptance, Windows/Linux interactive
behavior, physical trackpad gestures, low-power performance and manufacturing
suitability remain unverified by this checkpoint.

## Executable and automated evidence

Final source is pinned at `9a7a242`. The local bundle is
`artifacts/development/build-WjxwAP/MultiCAD.app`; its executable SHA-256 is
`1b97a2c100b1430b2b574b515f3588fd77c4964879cb64160b21d9b7b20ac452`.
The application was built with `CARGO_INCREMENTAL=0 ./build` and opened directly
as a review bundle. It was not installed, pushed or published. Older review
instances were closed through their native application controls.

The pinned toolchain and lockfile passed formatting, strict workspace Clippy and
the full workspace suite: 1,137 passed, zero failed, two existing ignored tests.
The release-contract suite passed 25 tests. Logs are local files:

- `/private/tmp/multicad-cad67-clippy-final-complete.log`
- `/private/tmp/multicad-cad67-tests-final-complete.log`
- `/private/tmp/multicad-cad67-release-contracts.log`
- `/private/tmp/multicad-cad67-native-final-build.log`

The [host regressions](../../crates/multicad-ui/src/sketch_assembly_host_tests.rs)
exercise actual Inspector buttons, scoped face picks, native candidate proof,
source-neutral cancellation, definition editing and object-coordinate bases.
The [assembly controller tests](../../crates/multicad-ui/src/assembly/native_tests.rs)
cover all five joint types, saved pose editing, limits, Undo/Redo, rereading and
reusable member identity. [Inspection tests](../../crates/multicad-ui/src/assembly/inspection_native_tests.rs)
use native query workers and reject stale or malformed replies.

## Editable drawer, hinge and repeated cases

The [review generator](../../crates/multicad-app/examples/assembly_review.rs)
created six editable projects and a native-checked manifest in the local folder
`artifacts/assembly-review-20261028-1805/`. The cases contain a base, sliding drawer
and hinged display. Closed, Half and Open store explicit slider and hinge values.
The nested example has one physical outer root and six canonical Part leaves.

In the native UI, Joint motion accepted `=$max_travel/2`, Preview produced exact
geometry and Apply committed the motion. Save As created
`07-slider-native-gui.multicad`. Application Undo/Redo restored the transaction,
then saved it at revision 3. Selecting the stored Open pose, previewing and
applying it updated both coordinates atomically; Save As created
`08-open-pose-native-gui.multicad`.

Independent rereading with the
[model report example](../../crates/multicad-app/examples/model_file_review.rs)
found two physical cases, six leaves and resolved references. Each case contained
three solids with total volume 55,000 mm³. Drawer travel was 60 mm and the hinge
rotation was 90 degrees. These values came from regenerated native solids and
poses, not the display mesh.

Selecting an internal Drawer member and Edit relationship opened its saved joint.
Pick face switched to a labeled component source view, fitted that exact native
mesh and captured an authored component-local frame. The physical camera was
restored when picking ended. Preview/Apply succeeded, and the same single member
remained selected with Edit relationship available. Save As produced
`09-nested-anchor-native-gui.multicad`.

Changing `drawer_width` from 40 to 50 mm through Parameters and rebuilding produced
`10-upstream-anchor-native-gui.multicad`. Independent regeneration retained one
physical root, six leaves, resolved references and the same joint/member IDs.
Both drawer solids changed from 12,000 to 15,000 mm³. The full native reports are
`09-nested-native-leaves.json` and `10-upstream-native-leaves.json`.

Place component reused the complete two-case module without flattening its source
definitions. `11-placed-nested-native-gui.multicad` regenerated two roots and twelve
leaves, each root containing 116,000 mm³. The final build's Edit assembly definition
quick action reopened the Assembly entity itself and canceled without changing it.

The final build moved the placed module with an actual X-axis gizmo drag. Opening
the same Move/Copy tool, ticking Create copy and dragging its Y arrow created one
linked Instance. `13-gizmo-move-copy-native-gui.multicad` regenerated three roots
and eighteen leaves. Native root translations were approximately
`[71.517279, 0, 0]`, `[71.517279, 87.906101, 0]` and `[1000, 0, 0]` mm; each root
retained 116,000 mm³. The source definitions remained shared.

## Exact inspection and exploded display

Interference / Clearance selected the physical Left moving case / Base member
and Left moving case / Drawer member paths. The native report showed contact,
0 mm minimum clearance, 0 mm³ overlap and four consistent native witnesses.

Sample motion range evaluated the selected pair at 0, 20, 40 and 60 mm. The UI
listed four explicit samples and actual intervals `[20, 20, 20]` mm. Expanding the
20 mm sample showed its native contact result. Inspection left the source revision
unchanged. The report explicitly states that samples do not establish continuous
collision-free motion, dynamics or safety between poses.

Exploded view separated individual Part leaves in the repeated nested example,
including when its physical display had only one aggregate root. The viewport
labeled the illustration as display-only and stated that inspection uses assembled
geometry. Cancel restored the assembled display without a source edit. The
[nested export regression](../../crates/multicad-model/tests/assembly_exports.rs)
separately writes/rereads four exact STEP leaves and planar DXFs at distinct world
positions; exploding a display does not alter those outputs.

## Inline tags and immediate Part assignments

The input `artifacts/model-review/materials-edited-native.multicad` contains genuine
Shelf and Cabinet side solids with material, stock, appearance and hatch links.
Selecting Shelf showed existing tags as removable badges inside an outlined field.
Typing `native-review,` created the badge, removed the comma and retained focus for
`discard-me`. Clicking the inline X discarded both pending values at revision 0.

Typing `native-review,cam,` and clicking the inline checkmark committed both tags
in one transaction. Choosing Warm plywood in Appearance override immediately
committed the assignment without a separate properties Apply action. Undo restored
Cool preview finish while retaining the tags; Redo restored Warm plywood.
Save As created `12-inspector-tags-native-gui.multicad` at revision 4.

Independent native rereading retained Shelf and Cabinet side volumes of 3,654,000
and 4,320,000 mm³, respectively, with resolved references. The local report is
`12-inspector-native-report.json`. The eight focused
[Part Inspector tests](../../crates/multicad-ui/src/materials/part.rs) additionally
cover continued typing, pasted delimiters, inline discard, mixed-selection field
preservation and rejection of foreign or stale drafts.

The production container reader also wrote `12-inspector-source-report.json`.
Its canonical Shelf tags contain `native-review` and `cam`, its Part properties
reference Warm plywood, and Cabinet side retains its original tags. These values
were checked independently of the open UI session.

## Limits

The [assembly decision](../architecture/0020-directed-assemblies-and-physical-leaves.md)
owns supported relationship and physical-leaf semantics. Subassembly copies share
internal joint values; flexible per-copy overrides and closed kinematic loops are
unsupported. Free transforms reject joint-controlled instances. Numeric and
geometric anchors are explicit, and unresolved references require repair.

These checks exercise the typed shared transaction API, including attributed
external-control channels. They do not demonstrate a connected MCP client or
claim completion of the separate MCP transport work.
