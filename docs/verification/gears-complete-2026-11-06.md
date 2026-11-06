# Complete bounded gear-generator verification

The native Gear command covers all eight requested family categories through
shared parametric UI and MCP transactions. The retained geometry, workflow and
exchange evidence below closes the original seven implementation criteria.
The final locked workspace suite passed 2732 tests with zero failures and four
intentional ignores. Human acceptance, Windows/Linux runtime checks and the
separate pair-performance follow-up remain outstanding.

The [gear reference](../architecture/gears.md) owns the current supported
matrix and source limits. This record consolidates evidence; it does not extend
those bounds. The tested suite source is
`44cef27d7c943753004f3129e2d8e998e7da21c3`. The actual expanded macOS package is
built from `d91ba92b9d72c707bc26b70f73aa30b973a7828e`; the intervening change
corrects only a test initializer, with identical product code. Its executable
SHA-256 is `b2844e0194b2e2803520d580e32f771da4ef8cb714e3bac64f9acc5989de5650`.
[Runtime attribution](../../artifacts/gear-aggregate-20261106/runtime.json).

## Original acceptance criteria

| Criterion | Verified evidence and boundary |
| --- | --- |
| 1. Mechanical Gear command, family fields and explicit coverage | The [coverage matrix](../architecture/gears.md#coverage) and canonical registry expose all eight families. Inspector and MCP retain explicit family-specific fields and refuse unsupported combinations. Support is bounded by each declared subset. |
| 2. Native external pair examples and distinct chevrons | The [external foundation](../../artifacts/gear-review-20261101/pair-foundation-review.json) records four-family native placement and STEP cases. The [expanded coverage record](gear-coverage-2026-11-06.md) adds shifted Helical, continuous Herringbone and gapped DoubleHelical native fixtures in both selected planes, independent dimensions/volumes and real Normal-plane macOS edits. |
| 3. Family-correct internal, rack, bevel and worm geometry | [Internal ring](internal-gears-2026-11-04.md), [finite rack](rack-pinion-2026-11-05.md), [spherical-involute bevel](bevel-gears-2026-11-05.md) and [generated ZA worm wheel](worm-gears-2026-11-05.md) retain native construction, compatibility, refusal and exchange evidence. No requested family remains omitted. |
| 4. Derived placement, centers, axes, phase and incompatibility diagnostics | Ordinary Assembly, Instance and Fixed relationship records carry derived family-specific anchors. Native current-pose and sampled-cycle checks accompany dimensional, undercut, bore/wall, overlap and separation controls. Rack travel is finite; bevel members share an apex; worm drive and crossed axes retain their declared roles. |
| 5. Explicit backlash/allowance and honest interference scope | Each member retains separate design backlash and fabrication thinning. Whole-solid overlap refusal remains distinct from engagement-proximity witnesses. Shifted chevrons require a witness in each band. Finite samples do not certify continuous contact, strength or lifetime. |
| 6. Parametric identities, edits/history/persistence and one UI/MCP path | The same canonical single/pair prepare APIs serve the inspector and MCP. Foundation and family lifecycle matrices cover atomic edits, cancellation, Undo/Redo and independent saved-session reopen. Source audits retain member/Part/assembly/relationship IDs and preserve complete recipes during drive-only edits. |
| 7. Native geometry, STEP/print, required checks and actual macOS | The linked family records combine native section/curve/volume checks, real STEP readback and independently parsed print payloads. Actual macOS interactions are separately attributed below. The final complete suite, formatting and strict Clippy pass; platform CI and physical printing are separate evidence. |

UI and MCP call `prepare_gear` and `prepare_gear_edit` for singles, and
`prepare_gear_pair`, `prepare_gear_pair_edit` and
`prepare_gear_pair_geometry_edit` for pairs. The
[UI command bridge](../../crates/multicad-ui/src/modeling/gear/pair.rs),
[single MCP tool](../../crates/multicad-mcp/src/tools/gears.rs) and
[pair MCP tool](../../crates/multicad-mcp/src/tools/gear_pairs.rs) use those
canonical transactions. Display tessellation is not the manufacturing source.

## Supported geometry and evidence provenance

All four external cylindrical families support module or diametral pitch with
an explicit Normal/Transverse convention, optional intrinsic coaxial hubs and
bounded selected-plane shifts. Internal rings retain the unshifted 20-degree
24-tooth pinion with a 48/60-tooth ring. Racks support 4-64 whole cells and a
matching unshifted spur pinion within conservative travel limits. Straight
bevels use the declared spherical-involute system, 20-degree pressure,
90-degree shafts and 24-64 teeth. Worm gearing is the declared right-hand,
single-start ZA subset: quotient 10, normal pressure 20 degrees, axial module
0.5-2 mm and 32-64 wheel teeth. These subsets do not imply unrestricted catalogs,
Gleason geometry, arbitrary worm systems or standards certification.

The original [foundation receipt](../../artifacts/gear-review-20261101/pair-foundation-review.json)
is preserved with its then-incomplete status and remaining-family list. Its
macOS binaries were not source-stamped; recorded source commits and executable
hashes supply historical attribution. Later family reports retain their own
runtime identities. No historical GUI or manufacturing run is relabeled as an
execution on the final suite source.

The actual rack, bevel, worm and expanded external macOS workflows are recorded
in their linked verification reports. Painted production-egui tests use real
native backends but remain offscreen evidence. The original internal-ring
record explicitly lacked an OS interaction check; the additional workflow below
supplies that evidence without rewriting the historical report.

## Additional actual macOS internal-ring workflow

The attributed package opened the Mechanical Gear inspector. A single
48-tooth ring with outside diameter 112 mm reached native Preview; Cancel
retained revision 0. Pair creation then used Spur A with 24 teeth and an 8 mm
bore, Ring B with 48 teeth and outside diameter 64 mm, module 1 mm, width 4 mm,
pressure 20 degrees and design thinning 0.05 mm per member. Native Preview and
Apply reached revision 1; Save As reached revision 2.

A drive preview at 12 degrees was cancelled at revision 2, preserving the saved
bytes and original zero-degree drive. After quitting and confirming an empty
running-app inventory, a fresh application session reopened the saved pair at
revision 0. Edit gear pair retained drive 0; Cancel left revision 0, followed by
quit. The [final source/process closure](../../artifacts/gear-aggregate-20261106/final-source-and-closure.json)
confirms the exact saved hash, no running QA process and no running CAD
application. The [interaction manifest](../../artifacts/gear-aggregate-20261106/screens/manifest.jsonl)
retains the actual captures. This workflow adds no claim of graphical Undo/Redo
for the ring; its native painted and canonical lifecycle tests remain separate.

The saved [native internal pair](../../artifacts/gear-aggregate-20261106/native-internal-pair.tcd)
has SHA-256 `32df37d3c8afb7a064c51cc1eeb7be7c3a21e76025756fb9e6092a238d17304a`.
The [independent source/native summary](../../artifacts/gear-aggregate-20261106/independent-internal-native-summary.json)
checks the container and a stamped, read-only public MCP rebuild. All 14 source
identities match the native producer/Part/member paths; two physical leaves
resolve to a 12 mm center distance. The current pose and nine sampled phases
report zero maximum overlap volume. The process exited normally with code 0,
without forced shutdown, and source bytes stayed unchanged. This check created
no STEP file; ring STEP/print geometry evidence remains in the earlier family
record. It also makes no timing or per-member native-mass claim.

## Required checks and remaining limits

The [final complete runner](../../artifacts/gear-coverage-20261106/checks/expanded-workspace/workspace-summary.json)
records `cargo test --workspace --locked --no-fail-fast -- --test-threads=4`
on the exact suite source: 2732 passed, zero failed, four ignored across 315
result groups, exit 0. Test optimization was 3 with debug assertions and
overflow checks enabled. The four ignores are two GPU tests, an
interrupted-writer subprocess helper exercised by its parent, and optional
Blender integration. The [formatting and strict all-target Clippy results](../../artifacts/gear-coverage-20261106/checks/expanded/gate-corrections.json)
pass separately. Earlier failed expectation tests and interrupted runs remain
attributed in the coverage report; focused reruns do not replace them.

Native B-reps, approximate involute/surface construction bounds and display or
print meshes have distinct tolerances. Independent print checks establish the
reported topology, finite coordinates, transforms and numerical budgets;
they do not certify geometric self-intersection freedom or physical print
success. The evidence establishes neither continuous collision-free engagement
nor load capacity, lifetime, accuracy grade or standardized fit.

[CAD-232](https://plans.tokenet.de/CAD/232) still tracks repeated pair work across
application lanes and operator-visible performance. Loaded historical preview
observations and test durations establish no acceptable-latency or p95 result.
No Windows/Linux interactive runtime, installation, publication or human
acceptance follows from this macOS and repository verification.
