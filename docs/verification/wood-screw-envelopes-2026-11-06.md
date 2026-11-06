# Wood screw dimensional envelopes - 2026-11-06

The canonical mechanical-part recipe, Library, native UI and MCP now retain
44 sourced external-hex wood screw envelopes. Focused checks, the full
workspace suite, source-stamped package, public MCP and actual macOS workflows
pass. Engineering evidence is ready for review;
human acceptance remains separate.

Evidence source: core commit `d8e2a0d` and integration commit
`1af33187cb216cb93448b2e9cd2746fbe85304a0`. The
[first-run scope receipt](../../artifacts/wood-screws-20261106/checks/full-workspace-inventory-first/full-workspace-scope.json)
records a clean tracked tree at its start; the final test-only correction is
identified below. Final verification source is
`c31205cdd4e0a98486cf3abca3d2993573c1fde6`.

## Source coverage and geometry limits

The [curation](../../library/wood-screws-curation.json) selects every populated
length cell for five diameters in the Bitork manufacturer sheet. The complete
twelve-diameter matrix also records unavailable choices within this selected
family. Unavailable means no verified preset here, without a claim about
commercial availability elsewhere.

| Nominal wood diameter, mm | Selected underhead lengths, mm | Presets |
| --- | --- | ---: |
| 4 | 16, 20, 25, 30, 35, 40 | 6 |
| 5 | 16, 20, 25, 30, 35, 40, 45, 50 | 8 |
| 6 | 20, 25, 30, 35, 40, 45, 50, 55, 60 | 9 |
| 8 | 25, 30, 35, 40, 45, 50, 55, 60, 70, 80, 100 | 11 |
| 10 | 30, 35, 40, 45, 50, 55, 60, 70, 80, 100 | 10 |

The other requested diameters are 2, 2.5, 3, 3.5, 4.5, 7 and 9 mm. Diameter
7 has head and V-reference dimensions but no populated discrete length cell.
The [independent curation audit](../../artifacts/wood-screws-20261106/sources/curation-audit.json)
checks all 44 rows and all twelve coverage cells.

Source facts have separate attribution:

- The undated Bitork DIN 571 sheet, PDF page 1, supplies the discrete
  diameter/length cells, underhead length drawing and head dimensions.
- The undated Placencia technical handbook, PDF page 18, section 1.3.2,
  supplies the separate DIN 7998 V-reference pitch, core diameter and
  60° ±6° included angle. These are wood screw references, without metric
  pitch diameter or receiving-fit semantics.
- The undated Westfield supplier specification, Figure 1, supplies the
  minimum tip-inclusive threaded extent `b >= 0.6l`. That lower bound includes
  the unspecified point and thread-start transition. It does not locate the
  actual thread start or recover a full-depth helical span.

URLs, research-copy hashes, source limitations and rights are retained in the
[source notice](../../library/sources/wood-screws/WOOD-SCREW-NOTICES.md) and
curation. The DIN 571:2016-12 identity does not make an undated manufacturer
sheet a verified reproduction of that edition. Fischer TX and Phillips H
presets are excluded because the selected evidence does not recover their
complete geometry. No Pozidriv substitution is made.

Each native body is a connected nominal stock cylinder and ideal sharp
regular hex head. The cylinder ends at a flat plane enclosing the unknown
manufacturer point. Head geometry uses the selected maximum across-flats
dimension and nominal height; the separately retained source diagonal is not
invented as a minimum constraint. The body has no recovered wood helix,
point cone, run-in, chamfer or manufacturing edge treatment.

The sole representation is `SimplifiedEnvelope`, with point
`ManufacturerUnspecified`. Nominal wood diameter is the stock diameter.
For example, the generic 6 ×40 mm recipe retains reference pitch 2.6 mm,
core diameter 4.2 mm, angle 60°, AF 10 mm and head height 4 mm. Its minimum
tip-inclusive extent is 24 mm, leaving at most 16 mm of plain length; neither
number asserts an exact physical thread transition.

No source PDF, illustration, logo, supplier CAD or implementation code is
bundled for these additions. No general redistribution license for the
manufacturer publications was established. New numeric curation does not
adapt FreeCAD or BOLTS code or data; existing BOLTS-derived families and their
notices remain separate. Current stock, certification, material, grade,
coating, structural capacity and physical print success are unverified.

## Retained source and native behavior

Wood screws use a distinct boxed canonical recipe, with required typed length,
reference profile and minimum extent. They have no `ThreadParameters`, metric
fit or modeled-thread selector. Bound expressions survive edits and reopen.
The Library exposes seven numeric inputs; the minimum extent remains a bound
expression using the authored underhead length. Immutable source references
retain literal source facts separately from edited values.

Native tests check five diameter envelopes, analytic volume and bounds,
hex sections, flat distal limits and two raw STEP readbacks. Raw kernel STEP
checks establish the declared envelope only. They do not authorize application
manufacturing export.

The native cache stores a completed solid under a private neutral owner and
rebinds each returned owner. Exact authored and derived side-role namespaces
are validated. Cold and cached construction under distinct owners must have
equal complete topology and no foreign ancestry. This does not broaden the
existing machine-screw occurrence-cache whitelist or change the native kernel
or trusted metric prototypes. Evidence:
[model/native summary](../../artifacts/wood-screws-20261106/checks/native-model-mcp-focused-summary.json).

The neutral manufacturing-history guard refuses simplified wood envelopes
through Common STEP, STL, OBJ, GLB, 3MF and manufacturing-outline DXF, Print
3MF/STL, and direct MCP STEP. Tests reach the wood-specific guard using valid
format settings, including meter units for GLB, and check that no file is
published. Downstream and pinned private occurrences remain limited after
cache reuse and reopen; an unrelated modeled part remains exportable.
Navigator parenting is tested through the actual placed instance and its
private member leaf path. It does not establish export of an aggregate native
assembly root.

## Library preservation

The Library contains 172 projects: all 128 prior project bytes, identities,
hashes and manifest entries remain unchanged, with 44 additions. The live
manifest moves from revision 8 to 9. A separate regeneration reproduces all
172 project bytes, identities, source pins and required resources exactly;
only that regenerated manifest increments from 9 to 10. It is not copied back
over the live manifest. Evidence:
[integration](../../artifacts/wood-screws-20261106/checks/library-integration.json),
[regeneration](../../artifacts/wood-screws-20261106/checks/library-regeneration.json)
and [package validation](../../artifacts/wood-screws-20261106/checks/package-source-agent-summary.json).

## Focused checks

Rust checks use the pinned toolchain and lockfile. Optimized test builds keep
debug assertions and overflow checks enabled. The rows below describe separate
targets, without treating repeated runs as additional unique tests.

| Check | Result | Retained evidence |
| --- | --- | --- |
| Core wood contract and legacy length | 5 wood +4 legacy pass | [log](../../artifacts/wood-screws-20261106/checks/core-focused.log) |
| Model native integration | 4 pass | [log](../../artifacts/wood-screws-20261106/checks/model-wood-fixed2.log) |
| Native owner/cache controls | 2 pass | [log](../../artifacts/wood-screws-20261106/checks/model-wood-cache.log) |
| Public in-process MCP and legacy mechanical parts | 2 new +6 legacy pass | [log](../../artifacts/wood-screws-20261106/checks/mcp-wood-and-legacy-fixed.log) |
| App catalog, native samples and regeneration controls | 4 pass | [log](../../artifacts/wood-screws-20261106/checks/app-focused-corrected.log) |
| UI form and search | 8 form +4 search pass | [form](../../artifacts/wood-screws-20261106/checks/ui-form-focused-corrected.log), [search](../../artifacts/wood-screws-20261106/checks/ui-search-focused.log) |
| Node release/package contracts | 34 pass | [summary](../../artifacts/wood-screws-20261106/checks/package-source-agent-summary.json) |
| Complete app binary unit target | 73 pass, 1 existing ignore | [native-permitted run](../../artifacts/wood-screws-20261106/checks/app-bin-focused-native.log) |
| Final workspace format and strict all-target Clippy | Pass | [format](../../artifacts/wood-screws-20261106/checks/fmt-final.log), [Clippy](../../artifacts/wood-screws-20261106/checks/clippy-final-after-card-coverage.log) |

Earlier failures remain available. The authored-only native ancestry guard
rejected real derived side roles; the neutral-owner implementation corrects
that product issue, and the app generator refused those invalid candidates
rather than accepting incomplete ancestry. Other corrections address test
fixtures: explicit body producer, GLB units, navigator-parent scope, egui
texture-delta lifecycle, the supported-geometry inventory and Clippy test
syntax. These failures are not hidden by the final focused results:
[model run history](../../artifacts/wood-screws-20261106/checks/native-model-mcp-focused-summary.json),
[app refusal](../../artifacts/wood-screws-20261106/checks/app-first.log),
[UI harness failure](../../artifacts/wood-screws-20261106/checks/ui-form-focused.log)
and [initial Clippy](../../artifacts/wood-screws-20261106/checks/clippy-first.log).

## Packaged public MCP and macOS workflows

The [release runtime receipt](../../artifacts/wood-screws-20261106/runtime.json)
identifies final verification source `c31205cdd4e0a98486cf3abca3d2993573c1fde6`,
172 bundled projects and 191 files. The bundled executable SHA256 is
`9ca0147fc85d1ddb64e9d50ef6073286f8c30980e6b6237317b5624ae75795d2`.
The [packaged self-test](../../artifacts/wood-screws-20261106/checks/native-self-test.log)
exits zero. The runtime receipt's original pending-workflow note predates the
completed workflow receipts below. No installation or publication is implied.

The [public stdio MCP receipt](../../artifacts/wood-screws-20261106/public-runtime-01/receipt.json)
checks five generic wood envelopes configured from curated scalar rows,
without attaching catalog provenance or Library pins. It verifies analytic
native volumes, retained bound lengths/minimum extents, edit/Undo/Redo/save,
and an independent read-only process reopen. Nine invalid, stale or STEP
requests refuse; an unrelated exact washer remains exportable. Recorded
creation calls of approximately 11-15 ms are functional observations under
concurrent test load, not GUI timing or a metric-thread latency certificate.

A separate [public MCP pin check](../../artifacts/wood-screws-20261106/public-pin-02/receipt.json)
imports the actual bundled 6 ×40 mm definition using the same package and
pin identified below. Instances A and B start at 40 and 55 mm, at world X0
and X30; editing only A to 45 mm leaves B at 55 mm. Both literal references
remain 40 mm with dimensional matching false. Native evaluation checks two
instances, no root Parts, analytic volumes and translated bounds. An invalid
B length of -1 returns `model_failed` without changing revision or source.
Undo/Redo/save and independent read-only cold reopen pass; both STEP requests
refuse without files, the original Library bytes remain unchanged, and both
processes exit zero. Final revision is 6 and project SHA256 is
`530152c65ef8c2889968d97594ff284dc7ef46b16d7fe6bba154e4d16a5396c7`.
The first harness expected a hexadecimal pin string; the API returns a
32-byte `ContentHash` array. Normalizing that array for comparison corrects
the harness, without a production change; the first script and log remain
retained.

The [actual macOS lifecycle](../../artifacts/wood-screws-20261106/native-ui-lifecycle.json)
records eighteen steps. Invalid nominal diameter disables Preview. Valid
native Preview followed by Cancel leaves revision zero unchanged. Overall
length expression `=44` with head height 4 mm applies at revision 1; Undo,
Redo and Save reach revision 4. At the observed compact size of 1100 ×700
logical points, changing head height to 5 mm retains the overall expression;
Apply, Undo, Redo and Save reach revision 8.

The actual bundled 6 ×40 mm definition is then placed using source pin
`8a74d8ce7f8820e7000de95e323c9c0900699bc9c6f0147c83886d2e96b9ffa2`,
with underhead override 55 mm and world X translation 30 mm. Apply reaches
revision 9, Save 10 and saving the whole-model camera preview 11. Print
previews for the private instance and All both refuse the unspecified
manufacturing detail; Write 3MF stays disabled. After normal quit, a fresh GUI
process opens the saved file at revision zero with both native solids visible.
Inspection retains `=44` and head height 5 mm; Cancel leaves it unchanged.

An [independent read-only packaged MCP process](../../artifacts/wood-screws-20261106/qa-ui/readback-02/receipt.json)
verifies the three original entity IDs within eighteen final entities, generic
overall length 44 mm, physical underhead span 39 mm and head height 5 mm.
The placed occurrence retains underhead 55 mm and bound minimum extent
33 mm, while its literal reference remains 40 mm and dimensional matching is
false. Actual shifted bounds and native volumes are checked: respectively
1535.7117233022343 and 1901.4985250407199 mm³ for the generic and placed bodies.
Both direct STEP requests refuse without creating files. The first readback
harness incorrectly looked for a consumed private definition body in the
final Part list; the corrected receipt checks the actual occurrence. The
original script and failing log remain retained.

The [cleanup receipt](../../artifacts/wood-screws-20261106/qa-ui/cleanup.json)
records both GUI process exits zero, no remaining CAD windows, unchanged saved
file SHA256 `8eb50b78cf67e8ec037bd656285c7705af1664e7fdb23cd660a553af8149a496`
and byte-exact preferences without a restore write. The preference SHA256 is
`49f7596bb838b8f71664a313bf91e9ba8a760d3393bd59179f99a41a0c056bb7`.

Two observed presentation issues remain scoped UI follow-ups: the Library
angle displays `59.99999999999999` degrees, and global Print status remains
“Resolving the print package” after the correct refusal and disabled Write
control. Neither observation establishes successful manufacturing output.

## Workspace verification

The first workspace attempt on integration source
`1af33187cb216cb93448b2e9cd2746fbe85304a0` found two stale app test
expectations of 128 Library projects instead of 172. It was interrupted with
SIGINT after 76 passes, two failures and one ignore across two completed
groups. The [retained summary](../../artifacts/wood-screws-20261106/checks/full-workspace-inventory-first/full-workspace-summary.json)
and [log](../../artifacts/wood-screws-20261106/checks/full-workspace-inventory-first/full-workspace.log)
record that attempt. Test-only commit
`45a78b3191baf232798551227025555834b4a284` corrected those inventory counts.

The second attempt exposed another branch in the same bundled-card test
that treated unclassified hardware as a bearing. It was interrupted after
72 passes, one failure and one ignore across one completed group. Its
[summary](../../artifacts/wood-screws-20261106/checks/full-workspace-family-first/full-workspace-summary.json)
and [log](../../artifacts/wood-screws-20261106/checks/full-workspace-family-first/full-workspace.log)
remain available. Final test-only source adds an explicit wood branch checking
all 44 unique nominal/length combinations, source head/minimum/representation,
absence of M-thread fields and absent material grade. The complete app unit
target then passes 73 tests with one existing ignore. Its earlier
[restricted run](../../artifacts/wood-screws-20261106/checks/app-bin-focused.log)
had three local-socket permission failures; the
[native-permitted rerun](../../artifacts/wood-screws-20261106/checks/app-bin-focused-native.log)
passes. Neither interrupted workspace attempt is a full-suite result.

The final workspace run passes on exact final verification source
`c31205cdd4e0a98486cf3abca3d2993573c1fde6`, with a clean tracked tree at both
start and finish: 2823 passed, zero failed and four existing intentional
ignores across 331 completed test groups, exit zero. It ran from
19:10:05.476 to 20:24:31.020 UTC on 2026-11-06, lasting 74m25.544s.
The command was `cargo test --workspace --locked --no-fail-fast` with test
threads set to four, test optimization level 3, debug info disabled, and debug
assertions and overflow checks enabled. Evidence:
[final summary](../../artifacts/wood-screws-20261106/checks/full-workspace-summary.json),
[scope](../../artifacts/wood-screws-20261106/checks/full-workspace-scope.json)
and [complete log](../../artifacts/wood-screws-20261106/checks/full-workspace.log).
Focused and full-suite totals describe overlapping checks and are not added
as unique tests.

The exact UI test binary SHA256 is
`ab97a470188963464c58573e18bf05ecc723f811a1522881363af89348521125`;
its [retained test list](../../artifacts/wood-screws-20261106/checks/full-workspace-ui-test-list.txt)
binds UI coverage to that completed run. The four ignores are the
`files::native::tests::interrupted_writer_child` subprocess helper, the Blender
handoff test requiring a local executable, and two GPU tests requiring
available physical GPU hardware. They are not reported as passes.

This evidence does not establish a performance target, Windows/Linux runtime
result, physical manufacturing success or human acceptance. The representation
and source limitations above continue to apply.
