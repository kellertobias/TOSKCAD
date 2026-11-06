# Configurable metric hex-nut catalog verification

The shared Library contains nineteen configurable metric hex-nut definitions in
separate DIN 934:1987 and ISO 4032:2012 reference families. Canonical, native,
UI/MCP, actual macOS, STEP and Print evidence passes. The complete catalog
baseline suite passed 2770 tests with zero failures and four intentional ignores;
the later UI-only correction passed its own complete UI-package checks and
native repeat. Engineering verification is ready for Review. Human acceptance
remains separate.

Source `503a835` adds parameter-controlled representation; `2a5717e` adds the
catalog. The [tested native package](../../artifacts/nut-catalog-20261106/runtime.json)
is release source `2a5717ea5f58fd8b94f2b91eabd1a1b2d0b68bf5`, executable SHA-256
`7d347cb08141844d20a668109436bfdc4f02c7e412096eaf5eeb1cafc8a3a4df`.
The UI-only identity correction is
`3676058abbf156bf704ebcf007944d7c1a89f298`; it changes the update inspector and
its regression tests. The corrected package has separate attribution below. The
[source-scope and binary proof](../../artifacts/nut-catalog-20261106/checks/verification-source-scope.json)
confirms the workspace command used the original catalog UI binary and the
corrected UI-package run used a distinct binary containing the new selector
regression. An earlier assumption that one binary had replaced the other was
incorrect. This is a complete catalog-baseline suite plus a complete corrected
UI-package run, not a full-workspace run on the newer UI commit.

## Source families and requested nominal coverage

The [curation file](../../library/nuts-curation.json) owns nineteen selected rows
and twenty-four per-family statuses across all twelve requested nominals.
The [source and adaptation notices](../../library/sources/bolts/NUT-NOTICES.md)
retain original Johannes Reinhardt attribution, the unmodified BOLTS source,
LGPL-2.1-or-later terms, exact upstream commit/hash and separate primary
cross-checks. No manufacturer CAD, branded diagram or standards PDF is shipped.

| Nominal mm | DIN 934:1987 reference | ISO 4032:2012 reference |
| --- | --- | --- |
| 2 | Supported | Supported |
| 2.5 | Supported | Supported |
| 3 | Supported | Supported |
| 3.5 | Supported | Supported, nonpreferred row |
| 4 | Supported | Supported |
| 4.5 | No selected row | No selected row |
| 5 | Supported | Supported |
| 6 | Supported | Supported |
| 7 | Supported coarse-pitch row | No row in selected edition |
| 8 | Supported | Supported |
| 9 | No selected row | No selected row |
| 10 | Supported | Supported |

The families remain distinct. M10 uses AF 17/height 8 mm for DIN and
AF 16/height 8.4 mm for ISO 2012; heights also differ at M5/M6/M8. The BOLTS
ISO-labeled M7 is excluded because the selected ISO 2012 edition has no such
row. No ISO 2023 replacement is silently inserted. Missing curated rows do not
assert universal commercial unavailability.

Across flats uses source nominal/max and height uses source maximum as disclosed
modeling references. The native blank is a sharp regular hexagon; its corner
diameter is derived from flats, while retained source minimum corner dimensions
remain provenance. The internal thread uses the shared JIS B 0205-2/4:2001
nominal-basic convention. There is no ISO 965 fit class. Tolerances, external/end
chamfers, washer faces and thread-entry mouth details are omitted. Material,
grade, coating and stock are unspecified; source product properties do not
become model properties or current-standard certification.

The [integration audit](../../artifacts/nut-catalog-20261106/catalog-integration.json)
retains all 57 earlier Library entries byte-exact and adds nineteen, for 76 total,
manifest revision 6 to 7. Its source/header checks are distinct from the subsequent
native geometry tests; its historical pending notes remain attributed to that
checkpoint rather than being rewritten as test results.

## One pinned definition, independent representation choices

[ADR-0037](../architecture/0037-parameterized-thread-representation.md) owns the
Count binding: exact 0 means Cosmetic envelope and exact 1 means Modeled helix.
When bound, the evaluated parameter is the sole authority even if the dormant
literal says the opposite. Missing, invalid or wrong-unit values refuse rather
than fall back. Omitted UI/MCP edits retain the binding; switching to fixed mode
is explicit. Conditional reader capabilities, dependencies and private-ID
remapping protect retained and template-only sources.

Library inputs use named choices identified by the typed thread role, not by a
parameter's name or a binary numeric value. Defaults remain distinct from
instance overrides. The same canonical symbol transactions configure each
instance without replacing the immutable pin or changing its siblings. Physical
reference matching remains independent of representation; changing flats or
height instead reports a configured dimensional deviation while retaining the
original source facts.

The [focused binding summary](../../artifacts/nut-catalog-20261106/checks/binding-focused-summary.json)
records thirteen core, two native, four UI-filter and two MCP-filter passes.
The four UI-filter cases include one older quaternion test, not four new nut
tests. Native M3/M6 controls use both opposite dormant-literal cases and check
actual cache invalidation, minor-bore cosmetic volume, multi-turn helical
material/void sections, flats/height, STEP and manufacturing refusal. The initial
M3 oracle wrongly assumed a major-radius cosmetic bore; correcting it to the
existing minor-radius bore retained the original numerical tolerance.

The [catalog-focused summary](../../artifacts/nut-catalog-20261106/catalog-focused-summary.json)
records four shipped-catalog tests passing in 48.06 seconds, including native
checks of all nineteen actual assets, two representative M10 STEP readbacks,
independent configurations on one pin and the 19/24 coverage matrix. Eleven
runtime-Library tests and 34 package-contract tests pass. Source checks alone
are not the native evidence; the target logs retain the actual runs.

## Actual macOS and independent saved-source readback

The [native capture folder](../../artifacts/nut-catalog-20261106/macos) retains
Library M3 discovery and named-choice controls. Initial Preview/Cancel leaves
revision 0. Cosmetic placement is saved, and Print reports that a cosmetic thread
must be switched to Modeled. A modeled preview can be cancelled while preserving
the cosmetic instance. Applying conversion uses the same pin; toolbar Undo
shows Cosmetic and Redo shows Modeled. These are actual operator observations,
separate from offscreen painted tests and independent protocol reads.

The [saved cosmetic readback](../../artifacts/nut-catalog-20261106/readback/actual-m3-cosmetic-02/summary.json)
confirms one native solid/physical leaf, effective Cosmetic with a dormant
Modeled literal, and refused direct STEP export. Native volume
51.47818375601704 mm³ agrees with the independent minor-bore volume within
2.85e-14 mm³. The input bytes remain unchanged and the process exits normally.

The [modeled readback](../../artifacts/nut-catalog-20261106/readback/actual-m3-modeled-01/independent-modeled-final-summary.json)
retains the same fourteen IDs and pin. Native volume 49.09912188569393 mm³ and
independent STEP volume 49.09912188979854 mm³ agree with the independent basic
profile integral 49.099121885902626 mm³ inside the unchanged area/linear plus
relative budget 1.6895980208346992e-5 mm³. A separate direct OCCT reader finds one
valid STEP solid and four interior phase probes alternate OUT/IN over two turns.
That STEP is a public-tool export of the actual GUI-saved source, not a claimed
native GUI manufacturing write. No latency result follows from concurrent tests.

A second cosmetic instance is placed at X 10 mm with flats 6 mm. The
[mixed-source audit](../../artifacts/nut-catalog-20261106/qa-update/saved-mixed-audit.json)
finds both instances on the same revision 1 pin: the first remains modeled with
AF 5.5 and reference dimensions matching; the second is cosmetic with AF 6 and a
reported physical deviation. Each retains its own parameter/private-feature
identity. The [incoming revision fixture](../../artifacts/nut-catalog-20261106/qa-update/receipt.json)
changes only authored height 2.4 to 2.5 mm for revision 2 while retaining the source
reference height 2.4, symbol identity and representation role. It is a QA update
fixture, not an implicit catalog revision or a new dimensional standard row.

Actual update review reached Apply revision 9 and Save revision 10, producing
`native-m3-updated-r2.tcd`. The [revision-2 readback](../../artifacts/nut-catalog-20261106/readback/actual-m3-updated-r2-01/summary.json)
and [canonical comparison](../../artifacts/nut-catalog-20261106/readback/actual-m3-updated-r2-01/canonical-source-comparison.json)
retain all 26 IDs and the independent modeled/cosmetic overrides. Both effective
heights become 2.5 mm on the new pin; the original reference height remains
2.4 mm, so both report dimensional deviation. Flats remain 5.5/6 mm and
placement stays independent. Native volumes agree with the separate analytic
oracles within their unchanged policy budgets; cosmetic STEP remains refused.
The audit leaves source bytes unchanged and exits normally with code zero.
It uses the older catalog package and does not prove accessibility of the later
corrected controls.

Review also found duplicate widget identities for the two instance
representation controls. The [counterfactual painted regression](../../artifacts/nut-catalog-20261106/checks/ui-update-id-counterfactual.log)
exposes only one independently accessible control where two are required. The
[final focused regression](../../artifacts/nut-catalog-20261106/checks/ui-update-id-final.log)
passes after instance-scoping those identities and exercises both selectors
independently. The [UI-fix strict Clippy check](../../artifacts/nut-catalog-20261106/checks/ui-update-id-clippy.log)
passes; independent source review found no actionable blocker. Complete controlled UI-package tests and the rebuilt native repeat are
recorded below.

## Corrected package, native repeat and Print

The [corrected release runtime](../../artifacts/nut-catalog-20261106/runtime-ui-fixed.json)
is source `3676058abbf156bf704ebcf007944d7c1a89f298`, binary SHA-256
`eb9ec8d86623d313a3c8ad3884a417946082c91a5fb2d87df61c23cee7dc252a`.
Actual captures 14/15 separately show the first instance retaining Modeled and
the second being changed to Modeled during update review. Preview/Apply reaches
revision 1; Save As reaches 2 in `native-m3-ui-fixed-two-modeled.tcd`.
The [independent source comparison](../../artifacts/nut-catalog-20261106/qa-update/fixed-vs-mixed-independent.json)
retains all 26 entity IDs, private maps and placements. The revision 2 pin and
height 2.5 apply to both; first AF 5.5 and second AF 6 remain independently authored.
The second mode change does not rewrite the first's recipe or retained reference.

The [corrected-package public native readback](../../artifacts/nut-catalog-20261106/qa-update/fixed-native-mcp.json)
confirms two modeled physical leaves and exits normally without source mutation.
The actual GUI Print preview then shows two objects and 4424 triangles, and the
native write produces one 3MF at revision 2. Its [independent raw print audit](../../artifacts/nut-catalog-20261106/qa-update/fixed-print-independent-audit.json)
finds two millimeter objects/build items, each 2212 triangles, closed opposite
edge uses and cyclic vertex fans. World bounds differ from native bounds by
about 1e-7 mm; volume differences are at most 0.033786 mm³ within the requested
0.1 mm chord budgets. The file embeds names/Application but no canonical IDs;
source occurrence association uses independently checked pose and flats bounds,
not an invented embedded-ID receipt.

After normal quit, cold reopen shows the second instance Modeled with AF 6 and
height 2.5, and the first Modeled with default AF 5.5 and height 2.5. Cancel leaves
revision 0. The [window closure](../../artifacts/nut-catalog-20261106/macos/final-window-closure.json)
records both corrected-package sessions exiting 0 and no running CAD apps.
The [preservation receipt](../../artifacts/nut-catalog-20261106/macos/final-preservation.json)
retains all five saved TCD hashes and byte-exact preferences; GUI MCP settings
were unchanged. The [corrected packaged self-test](../../artifacts/nut-catalog-20261106/checks/self-test-ui-fixed.log)
passes separately, including four bundled examples and actual STEP writes.

## Required checks and evidence scope

[Final workspace formatting](../../artifacts/nut-catalog-20261106/checks/final-format.log)
passes. Binding/catalog strict workspace all-target Clippy and Node contracts
passed at their recorded checkpoints. The
[catalog formatting](../../artifacts/nut-catalog-20261106/checks/catalog-final-format.log),
[catalog Clippy](../../artifacts/nut-catalog-20261106/checks/catalog-final-clippy.log)
and [initial package self-test](../../artifacts/nut-catalog-20261106/checks/self-test.log)
retain their checkpoint attribution. The [UI correction evidence](../../artifacts/nut-catalog-20261106/checks/ui-update-id-evidence.json)
records the counterfactual failure, corrected two-selector test, formatting and
strict Clippy. Its initial default-parallel package run had 1005 passes and one
existing buttress preview exceeding the unchanged 45-second timeout. That failure
remains retained; the isolated test passed in 13.54 seconds without changing its
deadline. The controlled four-thread complete UI package passed 1006 unit tests
and 46 integration tests with zero failures and two intentional GPU ignores.

The [complete catalog-baseline workspace run](../../artifacts/nut-catalog-20261106/checks/final-workspace/workspace-summary.json)
passed on `2a5717ea5f58fd8b94f2b91eabd1a1b2d0b68bf5`: 2770 passed,
zero failed and four intentional ignores across 321 result groups, exit 0.
The [runner](../../artifacts/nut-catalog-20261106/checks/final-workspace/workspace-run.json)
records 11:19:59-12:37:50 UTC and
`cargo test --workspace --locked --no-fail-fast -- --test-threads=4`, with test
optimization 3, debug assertions and overflow checks enabled. The ignores are
the interrupted-writer child helper exercised by its parent, optional Blender
integration and two physical-GPU tests. They are not reported as executed passes.

The full run's `multicad_ui-f444b9f67d421677` binary has 1007 listed tests and
passed 1005 with two ignores; it lacks the new selector test. The corrected
`multicad_ui-f0b757524a5cc17b` binary has 1008 listed tests and passed 1006 with
two ignores, followed by all 46 integration cases. Their hashes and direct test
listings are retained in the source-scope receipt. Only the update inspector
and its tests differ between these commits; native/core/catalog code is
unchanged. Counts overlap and are not added as unique passes. No full-workspace
run on the UI-fix commit is claimed. Required execution evidence, corrected
native workflow and source preservation are recorded with these separate scopes.
These checks establish no physical
nut/bolt fit, gage result, material grade, load rating or print-success
certificate. macOS evidence does not establish Windows/Linux runtime or CI
results. Human acceptance remains separate.
