# Gear pitch, hub and shifted-profile verification

The expanded implementation has passed focused canonical, native geometry,
STEP/print, painted controller and MCP checks. Formatting and strict workspace
Clippy pass. Actual macOS edits, cold reopen, refusal/cancel and manufacturing
exports have independent source and readback audits. **The final workspace suite passed on `44cef27`: 2732 passed, zero failed
and four intentional ignores. The implementation is ready for Review / test;
human acceptance remains pending.**

The initial implementation commits are `4066775` and `36d9b13`. The expanded
backend is `87da1bd`, UI/MCP is `d91ba92`, and `44cef27` changes only a test
initializer for Clippy. The packaged expanded runtime is built from `d91ba92`;
its product sources are unchanged by the subsequent test correction.
[ADR-0035](../architecture/0035-retained-gear-pitch-hubs-and-shift.md) owns the
current dated expansion decision and limits. The
[gear reference](../architecture/gears.md) owns family coverage.

Historical evidence remains separate: the first full run on `36d9b13` completed
with 2718 passes, two MCP expectation failures and four intentional ignores in
313 result groups. Tests were corrected in `d3694f5`. Its rerun was interrupted
with exit130 when an acceptance audit found applicable helical shifts still
missing. Those runs are retained; neither proves final expanded coverage.

## Expanded shifted families

The acceptance audit found source-applicable helical shifts still excluded by
the initial Spur implementation. The expansion adds selected-plane shifts for
Helical, Herringbone and DoubleHelical, and extends sourced pressure coverage
subject to the existing geometric guards. The dated expansion in ADR-0035
owns the equations and engineering limits. Legacy zero-shift source keeps its
previous behavior; new shifted profiles require `model.gear.shifted_external.1`.

The [expanded focused checks](../../artifacts/gear-coverage-20261106/checks/expanded/summary.json)
pass 13 canonical tests, three native control tests, two native fixture/STEP/history
tests (eight cases), one print test (seven family/plane combinations), five UI
field tests, one painted controller test (three families), nine MCP unit tests
and two native MCP tests. The initial strict Clippy failure
was a test initializer style check; the test-only correction and successful
[follow-up gates](../../artifacts/gear-coverage-20261106/checks/expanded/gate-corrections.json)
are recorded separately. Each chevron band requires its own named native flank
witness. The controls found and corrected a sampled-source mismatch: native
placement used the sampled document drive, while candidate tooth selection
used the original recipe drive. Both now resolve the same sampled source.

The independent fixture lives alongside the core tests, so clean checkouts do
not depend on ignored artifacts. Both KHK 12-tooth worked examples passed real
native mating, independent complete-volume and STEP readback checks.

The [expanded runtime](../../artifacts/gear-coverage-20261106/expanded-runtime.json)
uses binary SHA-256
`b2844e0194b2e2803520d580e32f771da4ef8cb714e3bac64f9acc5989de5650`.
Its [screen manifest](../../artifacts/gear-coverage-20261106/expanded-screens/manifest.jsonl)
binds actual interaction captures to that build. The UI opened the original
saved Spur source and changed both recipes to Helical with a helix angle of 25 degrees,
then Herringbone, then DoubleHelical with a gap of 1 mm. Normal-system DP 25.4/3 teeth/in, 12/24 teeth,
shifts 0.6/0.36, width 4 mm, bore 4 mm and hubs 16 mm at Z=-2..6 mm were retained.
Native preview, Apply and Save As succeeded for each family. Their saved
revisions were 2, 4 and 6. A drive edit to 37 degrees saved at r8 retained both complete
recipes and all 14 identities. Independent source audits confirm exactly these
changes and the reciprocal typed mate contexts.

After quitting and proving the process closed, the UI cold-opened the driven
file at r0. An out-of-range shift of 1.01 disabled Preview and Apply. Cancel left r0
and the saved bytes unchanged. Saving the whole-project STEP set added only
one ExportSet entity; the other 14 entities stayed exact. Both Common and Print
wrote at revision 1. Independent STEP readback found two valid single solids:

| Member | Independent Green volume mm³ | STEP readback mm³ | Print triangles |
| --- | ---: | ---: | ---: |
| A | 5956.676114134108 | 5956.676100018313 | 12204 |
| B | 20445.902540477906 | 20445.902532980910 | 18158 |

The [expanded manufacturing audit](../../artifacts/gear-coverage-20261106/expanded-independent-manufacturing-final-summary.json)
and [ordered-facet binding](../../artifacts/gear-coverage-20261106/expanded-independent-actual-package-binding.json)
pass for the two STEP files, placed 3MF and two STLs. Both stored-index 3MF objects
and exact-coordinate STL graphs are closed, consistently oriented, have cyclic
vertex fans, one connected component and Euler characteristic 0. All 30362
ordered triangles correspond across formats within per-corner float32 half-ULP
budgets. Mesh volumes 5950.971330/20435.080713 mm³ remain within the declared
0.1 mm chord-volume budgets of the independent reference. Poses match source
rotations of 37/169 degrees and center 62.1558035023 mm. Z=-2..6 mm is preserved with expected
below-bed findings; no arrangement was performed.

The [final closure audit](../../artifacts/gear-coverage-20261106/expanded-final-source-and-closure.json)
confirms all four earlier sources and five expanded sources remain byte-exact,
and no CAD application is running. The full suite passed, and a fresh
[post-suite inventory](../../artifacts/gear-coverage-20261106/expanded-post-suite-closed-apps.json)
also records no running CAD applications. These
checks do not certify continuous motion, load, accuracy grade, full-sweep
analytic bounds, mesh self-intersection, physical fit or print readiness.

The following runtime and export receipts retain their original `36d9b13`
attribution and prove the initial Spur workflow only.

## Native runtime and actual workflow

The [stamped runtime](../../artifacts/gear-coverage-20261106/runtime.json) is the
packaged macOS application built from `36d9b13`, executable SHA-256
`93cfe95acf2e17d106138457054836cc06cdc22a73758c263c133ef76743d02e`.
The [screen manifest](../../artifacts/gear-coverage-20261106/screens/manifest.jsonl)
records the actual native interaction. Developer test timings and automation
waits below are not application latency or p95 measurements.

Actual creation used diametral pitch `8.466666666666667 teeth_per_in`, equivalent
to module 3 mm in the selected zero-helix plane; counts 12/24, shifts 0.6/0.36,
20-degree pressure, width 4 mm, bore diameter 4 mm, thinning 0.05 mm per member
and root-fillet ratio 0.2. Both hubs began with diameter 16 mm and local axial
extent -2 to 6 mm. Native Preview succeeded and Apply created the pair in one
transaction. The actual Save panel wrote
[the created project](../../artifacts/gear-coverage-20261106/native-shifted-pair-created.tcd).

Drive-only editing set 37 degrees, then native Preview, Apply and Save As wrote
[the driven project](../../artifacts/gear-coverage-20261106/native-shifted-pair-driven.tcd).
A hub preview was cancelled at revision 4 with unchanged saved bytes, recorded
in the [cancel receipt](../../artifacts/gear-coverage-20261106/native-preview-cancel-receipt.json).
A subsequent shared edit changed only the primary hub diameter from 16 to 18 mm.
Toolbar Undo restored the displayed diameter 16; toolbar Redo restored 18.
Immediate Command-Z was not observed to act, so keyboard Undo is not claimed.
Changing shifted pressure to unsupported 25 degrees disabled Preview and Apply;
Cancel retained the valid 20-degree recipe. The final edit was saved as
[the hub-edited project](../../artifacts/gear-coverage-20261106/native-shifted-pair-hub-edited.tcd).

The application was closed, the running-app inventory was empty, and the same
bundle cold-opened the hub-edited file at revision 0. Its source hash remained
unchanged, as recorded in
[the cold-reopen receipt](../../artifacts/gear-coverage-20261106/cold-reopen-source-immutability.json).
An actual Whole Project, separate STEP export set was added and saved in
[the export-config project](../../artifacts/gear-coverage-20261106/native-shifted-pair-export-config.tcd).
Native Common and Print completed their writes. Print reported two placed
objects and 40,614 triangles at chord 0.1 mm and angular deviation 15 degrees.
Coordinates remained at Z=-2..6 mm; below-bed findings were expected. No automatic
arrangement or physical print-ready result is claimed.

## Independent source and manufacturing readback

The checked-container
[created-source audit](../../artifacts/gear-coverage-20261106/independent-created-source-audit.json)
confirms the actual typed DP, reciprocal mate count/shift sources and all 14
canonical entities. Independent source equations give operating center
56.49986972030518 mm, working pressure angle 26.0885634421 degrees and contact
ratio 1.20210157005. These scalar results do not replace native interference
checks.

The [drive comparison](../../artifacts/gear-coverage-20261106/independent-driven-source-audit.json)
retains every entity ID, output and both complete gear recipes. Only drive
0 to 37 degrees and preview metadata changed. The
[hub comparison](../../artifacts/gear-coverage-20261106/independent-hub-edited-source-audit.json)
retains those identities and changes only primary hub diameter plus preview
metadata. Its analytic volume increases by 213.6283004441 mm³. The
[export-config binding](../../artifacts/gear-coverage-20261106/independent-export-config-source-binding.json)
retains all 14 old entities exactly and adds one ExportSet, producing 15 entities.

Actual file hashes are retained in
[the export inventory](../../artifacts/gear-coverage-20261106/actual-export-file-inventory.json).
The [independent manufacturing audit](../../artifacts/gear-coverage-20261106/independent-manufacturing-final-audit.json)
uses a standalone OCCT STEP reader, independent Green boundary integration and
raw standard-format parsers. It does not call the production adapter to obtain
its analytic volume targets. The
[summary](../../artifacts/gear-coverage-20261106/independent-manufacturing-final-summary.json)
and [package binding](../../artifacts/gear-coverage-20261106/independent-actual-package-binding.json)
retain producer IDs, actual resource/build ordering, transforms and input hashes.

| Final member | Analytic solid volume, mm³ | Independent STEP volume, mm³ | 3MF triangles |
| --- | ---: | ---: | ---: |
| A, hub diameter 18 mm | 5586.798671891806 | 5586.798668297240 | 21702 |
| B, hub diameter 16 mm | 17614.449415201630 | 17614.449428941887 | 18912 |

Each STEP is a valid single solid in millimeters. Volume differences are
3.595e-6 and 1.374e-5 mm³, within the unchanged area/linear-policy plus relative
volume budgets. Independently read world support bounds differ by about
1.000004e-7 mm from the
[analytic curve extrema](../../artifacts/gear-coverage-20261106/independent-final-analytic-support.json),
within an explicitly declared 2e-5 mm numerical consistency budget. That budget
is not a manufacturing accuracy grade.

Both 3MF stored-index objects and both STL exact-coordinate facet graphs pass
closed-edge incidence, opposite winding and connected vertex-fan checks. Each
has one connected component and Euler characteristic zero, consistent with its
through bore. Actual 3MF resource 1 is B and resource 2 is A. Source-derived
world transforms match the stored build transforms. Ordered STL corners match
the transformed 3MF facets within local float32 half-ULP plus 1e-12 mm rounding
allowance; maximum differences are 9.403e-7 mm for A and 3.809e-6 mm for B. No
welding, reordering or repair is performed by the independent comparison.

Print volumes are approximately 5583.694 and 17609.297 mm³. Their differences
from the exact-profile oracle remain within the requested 0.1 mm chord-based
area budgets. Bounds match the STEP support within approximately 1.32e-6 mm.
These are tessellated approximations, separate from native manufacturing solids.
All four saved source hashes remain unchanged after export and final quit in
[the closure receipt](../../artifacts/gear-coverage-20261106/final-source-and-closure.json);
[the final app inventory](../../artifacts/gear-coverage-20261106/final-closed-apps.json)
contains no running CAD application.

## Focused checks and remaining gate

The [native summary](../../artifacts/gear-coverage-20261106/checks/native-focused-summary.json),
[transformed/print summary](../../artifacts/gear-coverage-20261106/checks/transformed-and-print-summary.json)
and [UI/MCP summary](../../artifacts/gear-coverage-20261106/checks/ui-mcp-focused-summary.json)
retain exact commands, exits and elapsed time. Counts below come from each
linked target log, not a combined full-workspace claim.

| Focused target | Result |
| --- | --- |
| [Canonical coverage](../../artifacts/gear-coverage-20261106/checks/core-final-focused.log) | 8 passed |
| [Native gear matrix](../../artifacts/gear-coverage-20261106/checks/kernel-gears-final.log) | 8 passed |
| [Shifted engagement controls](../../artifacts/gear-coverage-20261106/checks/native-shifted-controls.log) | 1 passed |
| [Bevel regression](../../artifacts/gear-coverage-20261106/checks/native-bevel-regression.log) | 2 passed |
| [Transformed shifted pairs](../../artifacts/gear-coverage-20261106/checks/native-transformed-shifted-pairs.log) | 3 passed |
| [Four-family hub print](../../artifacts/gear-coverage-20261106/checks/four-family-hub-print.log) | 1 passed |
| [Retained UI fields](../../artifacts/gear-coverage-20261106/checks/ui-retained-fields.log) | 5 passed |
| [Painted native preview](../../artifacts/gear-coverage-20261106/checks/ui-native-painted.log) | 1 passed |
| [MCP native workflow](../../artifacts/gear-coverage-20261106/checks/mcp-native.log) | 1 passed |
| [Corrected MCP unit target](../../artifacts/gear-coverage-20261106/checks/mcp-all-unit-corrected.log) | 8 passed |
| [Corrected gear workflows](../../artifacts/gear-coverage-20261106/checks/mcp-gears-corrected.log) | 3 passed |
| [Prototype guards](../../artifacts/gear-coverage-20261106/checks/prototype-guards.log) | 5 passed |

The [prototype producer](../../artifacts/gear-coverage-20261106/checks/prototype-generation.log)
regenerated 15 unique assets for 19 canonical recipes. The
[independent audit](../../artifacts/gear-coverage-20261106/independent-prototype-audit.json)
confirms every BRep byte, mass, bound, face receipt and seed ancestry is unchanged.
Only the registry source fingerprint changes to
`4f64ed176d010b3468fe9406dddf24f1d44d919dc1afc63e14d833b1f3d8ea62`.

The first [full workspace run](../../artifacts/gear-coverage-20261106/checks/workspace/workspace.log)
and its metadata retain the two failed test expectations; they are not replaced
by focused results. Post-correction [formatting](../../artifacts/gear-coverage-20261106/checks/fmt-corrected.log)
and [strict Clippy](../../artifacts/gear-coverage-20261106/checks/clippy-corrected.log) pass.
The d3694f5 rerun was interrupted with exit130 after an acceptance audit found
applicable helical and chevron shift coverage still missing. The results above
remain evidence for the prior Spur subset. Expanded focused, native macOS, manufacturing and final workspace results
are recorded above.

The final [complete workspace run](../../artifacts/gear-coverage-20261106/checks/expanded-workspace/workspace-summary.json)
passed on `44cef27d7c943753004f3129e2d8e998e7da21c3`: 2732 passed, zero failed,
four intentionally ignored, across 315 result groups, exit 0. It ran from
04:24:42 to 05:39:13 UTC on 2026-11-06 (74 minutes 31 seconds). The exact command was
`cargo test --workspace --locked --no-fail-fast -- --test-threads=4`, with test
optimization 3, debug assertions and overflow checks enabled. The UI target
passed 1001 tests with two GPU ignores in 172.61 seconds. The other ignores are
an interrupted-writer subprocess helper exercised by its parent test and an
optional Blender integration test. No application performance percentile follows
from these checks. Finite poses do not prove continuous engagement, and this evidence does
not certify strength, standardized fit, geometric triangle self-intersection or
physical print success. Windows/Linux runtime results and human acceptance
remain separate.
