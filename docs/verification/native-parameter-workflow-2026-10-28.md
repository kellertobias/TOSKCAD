# Native Parameters workflow verification, 2026-10-28

The native macOS application edited a parameter-driven constrained panel, retained
its geometry bindings through renames, and reopened the saved result. This records
engineering verification. Human workflow acceptance remains separate.

## Source and executable evidence

The parser correction is `5dee649`. Table and draft controls are `8893873` and
`9a7eaab`. The native fixture generator and lifecycle regressions are `cd29db3`
and `8f73d0c`.

| Native check | Local bundle | Executable SHA-256 |
| --- | --- | --- |
| Authoring, failure diagnostics, history and Save As | `artifacts/development/build-7rc46p/MultiCAD.app` | `f36863092e7e1c7f49526f0a553ac1a0371c644bc69c86bfbe6414715ccf5cd3` |
| Fresh reopen and filtered draft editing | `artifacts/development/build-X9eTv2/MultiCAD.app` | `975084f319b87b47df2ee65d0e6d18d6c1168906454f8be94bc9719aa3e9d675` |

The final bundle contains production source through `9a7eaab`. Earlier review
instances were closed through native controls before launching it. Nothing was
installed, pushed or published.

## Coverage and concrete corrections

The implementation lives in
[ParameterEditor](../../crates/multicad-ui/src/parameter_editor.rs),
[application composition](../../crates/multicad-ui/src/application.rs) and
[parameter evaluation](../../crates/multicad-core/src/parameters/mod.rs).

| Behavior | Evidence |
| --- | --- |
| A completed row adds one blank row and retains typing focus | Actual native entry plus pointer/keyboard editor regression |
| Filtered rows remain reachable and actions retain the correct identity | A 200-row fixture filters an initially off-screen row, then clicks reorder/delete and checks persisted IDs and Undo |
| New rows and a row being renamed remain editable under an active filter | Editor regressions and final native filtered new-row check |
| Unit-only input remains an unapplied draft | Editor and application tests preserve it across commands and document switching; Save remains blocked |
| Cleared existing rows require completion or explicit deletion | Source-neutral editor regression |
| Invalid, stale or foreign drafts cannot apply | Session-scoped editor guards, actual stale-revision tests and native disabled Apply controls |
| Invalid or stale drafts do not display a previous number as evaluated | Editor painting tests and native failure checks |
| Error labels identify the affected parameter | Missing-reference/unit regressions and native `depth` errors |
| Renames preserve existing bindings despite new parameters using old names | Core tests, native lifecycle tests, GUI save and independent source/geometry report |
| Values, units, descriptions, exposure and ordering survive history and reopen | Native controls, saved container and native lifecycle tests |

A failing regression reproduced an evaluation panic from `=1 < = 2` before the parser
fix. Whitespace-separated comparison characters produce a Syntax error. Valid
`<=`, `>=`, `==`, `!=`, `<` and `>` comparisons remain covered. These expressions
evaluate ratio values; they do not solve geometric inequalities.

The full suite found a shortcut compatibility regression after Apply began
requiring a current checked preview. Initial rendering prepares a valid populated
draft once. The existing keyboard Apply regression passes with that correction.

## Native authoring and failure checks

Editable inputs and results are under
`artifacts/parameter-review-20261028-2119/`. The generator creates three native
projects and a manifest, refuses an existing output directory, and reopens each
archive through the production source reader and native compiler.

The first project contains width 120 mm, height 80 mm and stock 6 mm. Its depth is
`=$stock + 2 mm`, producing one 120 x 80 x 8 mm solid with volume 76,800 mm³.
The canonical sketch deliberately retains smaller authored coordinate guesses;
the current constrained solve determines the actual profile.

In the native table, `=1 < = 2` produced an explicit comparison error. Apply and
Save were disabled, evaluated cells showed `-`, and Discard retained revision 0.
Editing width to 150, height to 100 and stock to 8 committed one transaction.
Toolbar Undo restored all three original values; Redo restored the edited set.

Typing `temporary` and `42` added exactly one blank row. Continued typing produced
`420` in the same field. Apply added that parameter; its trash control and another
Apply removed it explicitly.

The same session renamed width to `span`, changed its value/unit to 15 cm, edited
its description, and cleared exposure. Stock became `material_thickness`.
New `width` and `stock` parameters each contained 999 mm. The original sketch and
depth bindings retained their original parameter identities. Filtering `span`
and clicking its down arrow reordered the intended parameter.

Drafts containing `=$depth + 2 mm`, `=$missing + 2 mm` and `=1 deg + 2 mm` showed
cycle, missing-reference and incompatible-dimension errors, respectively. None
enabled Apply or changed revision 7. Discard restored the committed formula.

The actual model displayed the regenerated panel. Save As wrote
`04-parameters-native-gui.multicad` at revision 7. A fresh final-build session
reopened it in Model. Parameters retained the row order, 15 cm span, descriptions,
exposure flags, both 999 mm replacements and the original depth formula at 10 mm.

With a `height` filter active, entering a new nonmatching `unmatched` parameter
with value 42 cm retained the visible row and added a blank row. Discard removed
that entire draft and left the reopened source at revision 0.

## Independent saved-source and geometry verification

The production `model_file_review` example read the GUI-saved archive and wrote
`04-parameters-native-gui-report.json`. It rebuilt one solid bounded by
(0, 0, 0) and (150, 100, 10) mm, with volume 149,999.99999999997 mm³.
Every recorded geometry reference resolved.

Comparison with `01-parameter-panel-report.json` retains the document, sketch,
Part and extrusion identities. The complete canonical sketch and producer source
remain unchanged while the native volume changes from 76,800 to 150,000 mm³.

| Retained identity | UUID |
| --- | --- |
| Document | `cc5d1cc6-20e0-4d7c-b15a-04ff0cc10b97` |
| Sketch | `57cf1ff9-83bf-446a-be8e-deeeeb286d25` |
| Part | `261897e7-08ed-4da7-bd32-e4a67e9e6bf3` |
| Extrusion feature | `06cb0c2e-85e9-4648-891a-380e78e5bb8b` |
| Original width, renamed span | `c53b0a43-53f3-4c95-ab88-644ae509aab0` |
| Original stock, renamed material_thickness | `3d64bc1e-1778-4264-8faa-b37980c4d6d5` |

The saved depth expression's `stock` alias still targets the original stock UUID.
The width dimension's `width` alias still targets the original width UUID.
The replacement parameters have separate identities.

## Automated checks and boundaries

Final production/test source `9a7eaab` passed the pinned formatting check, strict
workspace/all-target Clippy and locked offline workspace tests: **1,187 passed,
zero failed and two existing ignored tests**. `npm test` passed all 25 release
contract tests. Those tests do not publish or establish hosted CI execution.

Logs are `/private/tmp/multicad-cad133-fmt-final-complete.log`,
`/private/tmp/multicad-cad133-clippy-final.log`,
`/private/tmp/multicad-cad133-tests-final-complete.log` and
`/private/tmp/multicad-cad133-release-contracts.log`. The known upstream OCCT
`sprintf` deprecation remains outside the Rust lint scope.

Focused coverage includes nine core parameter tests, seven editor workflow tests,
two application regressions and five native lifecycle tests. The existing sketch,
formula and shortcut suites also remain in the workspace gate.

Source-valid parameter changes may intentionally leave dependent geometry in a
repairable failed state. Parameter Apply validates source and expressions; it does
not promise a successful native solve for every consumer. Native lifecycle tests
commit negative, incompatible-dimension or removed driving inputs, verify native
failure and old-result stamps, then restore exact identities and geometry with
Undo. Such source remains saveable. Invalid table expressions remain unapplied.

Dynamic symbol overrides, remaining sketch extensions and the combined drawing,
import, export and connected MCP workflow are separate work. These results supply
no Windows/Linux interactive, low-power hardware or fabrication acceptance claim.
