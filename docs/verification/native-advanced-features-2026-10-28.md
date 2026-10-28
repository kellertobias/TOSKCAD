# Advanced solid features: native verification

CAD-65 implements the [advanced modeling contract](../architecture/advanced-modeling.md)
and [Fillet/Chamfer controls](../architecture/edge-treatments.md). Engineering
verification is complete for the supported operations below. Human workflow
acceptance remains open.

## Frozen source and automated evidence

Final production and test source: `564720d`. Earlier native checks used
`60518ed`; the final source additionally fixes dropdown dismissal and vector
reorder controls and registers the solved-context host regression.

The pinned toolchain and locked dependencies passed:

- `cargo fmt --all -- --check`.
- Strict workspace/all-target Clippy, with `-D warnings`.
- Locked offline workspace tests: **1,076 passed, zero failed, two existing
  ignored tests**.
- Release-contract tests: **25 passed, zero failed**. These run on the same
  unchanged release tooling; they do not publish anything.

Final Rust logs are `/private/tmp/multicad-cad65-final-{clippy,tests}.log`.
Release-contract evidence is `/private/tmp/multicad-cad65-release-tests.log`.
The known upstream OCCT `sprintf` deprecation warning remains visible.

Native tests check actual B-rep volume, validity, solid count, topology history,
STEP readback and independent owners running concurrently. Compiler and
controller tests cover upstream expressions, linked Instances, persistent source
identities, removed-member repair, one-transaction Apply, exact Undo/Redo,
save/reopen and source-neutral failure. A construction-line change invalidates a
saved Sweep with the same curve identities.

The actual editor-host regression supplies authored circle radius guesses of
1 mm that solve to 3 and 4 mm. The Sweep controls receive current solved profiles
and only the nonconstruction path segment. After an external source edit, the
old preview and Apply are rejected; reevaluation refreshes the catalogs and
excludes the newly classified construction path. The Loft controls use the new
solved sections. Cancel preserves the changed source revision.

## macOS window checks

Final build:
`artifacts/development/build-YbTu8X/MultiCAD.app`, executable SHA-256
`c6f468ef5862a5c9863866fba97176ea597191a895309135345006e54198bcdf`.
Intermediate geometry checks used `build-OKNhTh` from `60518ed`.
Both builds were launched locally; neither was installed.

The native Model/Common ribbon exposes all named tools with distinct vector
icons and a Pattern menu. Tool-first Shell, Mirror, Draft and patterns acquire a
Part before showing controls. Sweep and Loft open directly without requiring an
unrelated Part selection. Actual solved section/path menus populate correctly.

Shell captures an opening face in the viewer, previews a hollow box and applies
one transaction. Editing its thickness to 1,000 mm fails native material-side
validation and keeps Apply disabled. Correcting to `=$wall` produces a valid
preview; Cancel preserves the saved revision.

The native window creates Sweep from the section and +Z path, Loft from two
ordered circle sections, Mirror through XY, a linear pattern with two members
and a circular pattern with four members. Draft selects an explicit side face,
previews a 5° modification and applies it. Each result is saved to a fresh file
through the operating-system picker.

Sided Chamfer captures one upstream edge, then its adjacent face without
replacing the edge. First distance 2 mm and second distance `=$second` (3 mm)
apply successfully. Editing that same feature to distance 2 mm and angle 30°
retains its identities. Toolbar Undo and Redo restore the two-distance and
angle versions at revisions 3 and 4. Changing upstream width from 40 to 60 mm
rebuilds the dependent native feature and retains both captures.

The final build freshly reopens the GUI-saved angle project in Model at revision
zero. Selecting a Chamfer profile closes its popup; Cancel leaves the source
unchanged. The saved Loft reopens with correct ordered sections and vector
reorder controls, previews successfully and cancels without mutation. Draft is
created and saved in this final build. All older review instances are closed.

## Independent saved-file regeneration

Review files and production `model_file_review` reports are under
`artifacts/advanced-review-20261028-1648/`. They are local artifacts, not fixtures
committed to the repository. The generator also supplies 18 editable input/result
projects and a manifest, and refuses an existing output directory.

| GUI-saved result | Native solids | Native volume, mm³ |
|---|---:|---:|
| Shell | 1 | 7,152 |
| Draft | 1 | 23,300.090691792608 |
| Sweep | 1 | 2,513.2741228718287 |
| Loft | 1 | 3,518.58377202056 |
| Mirror | 1 | 24,000 |
| Linear pattern | 2 | 48,000 |
| Circular pattern | 4 | 96,000 |
| Two-distance Chamfer | 1 | 23,940 |
| 30° Chamfer | 1 | 23,976.905989232408 |
| Chamfer after width edit | 1 | 35,976.9059892324 |

Rounded integers agree within native numerical precision. Mirror bounds reach
Z = -20 mm with maximum Z = 0. The upstream Chamfer bounds reach X = 60 mm.
Every saved reference resolves. The Chamfer feature, edge capture and side-face
capture identities remain equal across profile and upstream dimension edits.

## Limits of the evidence

The [operation boundaries](../architecture/advanced-modeling.md) remain explicit:
open-face Shell, supported analytic Draft faces, straight open Sweep paths,
matching ordered Loft sections and compound Part patterns. This work does not
implement variable-radius fillets, arbitrary spline sweeps or independent
assembly motion for pattern members.

These checks establish native macOS behavior and exact source regeneration.
They do not establish Windows/Linux interactive behavior, low-power performance,
physical trackpad behavior, fabrication suitability or human acceptance.
No remote creation, push, publication, release or installation occurred.
