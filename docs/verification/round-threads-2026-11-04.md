# Round thread verification, 2026-11-04

The implementation at `61bc653` adds separate nominal Rd10x1/10 and Rd20x1/8
profiles through the canonical inspector and MCP API. Focused source, native,
UI, manufacturing and packaged runtime checks passed. The full required suite
passed 2,537 tests with zero failures and four ignored. Actual OS interaction
and human acceptance are unverified.

## Source and nominal construction

The [Round reference](../architecture/threads.md#round-profiles) owns source
versions, equations and coverage. Mitsubishi Materials C010B, printed G009,
supplies the 30-degree contour and source radial crest clearance. BAER 2022,
printed page 134, supplies the two named metric-diameter/reciprocal-inch-pitch
sizes. Walter Special Edition 2026, imprint 11/2025, independently corroborates
the distinct internal material radii.

The internal positive cutter reverses material crest/root terminology. Its
crest and root radii differ; the external member has equal circle radii.
Exact tangent construction retains source-rounded coefficients separately.
Shared basic pitch diameter uses the source dimension arrows; the internal
construction diameters do not place pitch diameter at their midpoint.

Only one start, NominalBasic, zero taper and zero axial fabrication allowance
are supported. Both hands are checked engineering geometry. No complete
DIN 405-2 class, calibrated gaging, full current DIN conformity, physical fit
or load capacity follows. The test pairs additionally save 0.005 mm radial
fabrication allowance per member, separate from source clearance.

## Native circles and retained classification regression

The [native matrix](../../crates/multicad-kernel/tests/knuckle_threads.rs)
passed three tests in 48.33 seconds. It checks actual crest/root circles and
30-degree flanks for both hands and mode-specific profiles, independent
analytic volume, single-solid validity, root ancestry, fractional ends,
STEP readback and default mesh. A separate Rd10 fixture checks actual
native material against independent fine-mesh parity at two meridians and
proves mesh generation leaves native B-rep bytes unchanged.

An earlier 150-degree crest face produced contradictory native and mesh
classification at an Rd10 circle. Tiny-cube Boolean Common treated both
points 0.001 mm inside/outside the expected circle as material, while an
independent triangle section bracketed the circle correctly. The current
knuckle representation uses two exact 75-degree crest arcs on that same
circle, plus separate exact root shoulders and aligned quarter-turn spine
domains. Phase, radii, depth, end planes, tolerances and physical/volume
checks remain unchanged. Both native regression fixtures pass. This is a
checked representation change; the underlying OCCT classification path was
not fully isolated and no general OCCT engine fix is claimed.

The [model/export matrix](../../crates/multicad-model/tests/round_threads.rs)
passed two tests in 211.60 seconds. Four pairs cover both sizes and hands.
They verify empty mating intersection, native and STEP material on both
flanks and both circles, ancestry, exact ends, independent source reopen,
written print circles and signed phase at meridians 0/0.43 radians. Receiving
wall and cosmetic manufacturing refusals remain explicit.

## Canonical and inspector checks

Five core tests passed, checking source/constructed dimensions, unequal
internal radii, actual versus basic pitch diameter, radial fabrication effects,
atomic unsupported requests, history/reopen and direct/suppressed/template/
nested/retained reader gates. The source uses existing family, nominal size,
fit, hand and allowance fields, with conditional model.thread.round.1 capability.

Three MCP tests passed in 48.01 seconds: actual native owner preview/reject/
apply, mixed metric/inch literals, cancellation, atomic failure, omitted edits,
Undo/Redo, saved independent reopen, STEP and nested hardware receipts.
The painted production-egui test passed both presets, authored length,
left hand, family return, Preview/Cancel/Apply/history and independent saved
session reopen. These are offscreen checks rather than OS-window interaction.

## Retained manufacturing files and measured pitch

[Review artifacts](../../artifacts/round-thread-review-20261104-verified/report.json)
contain four editable TCD projects, eight STEP files, four combined 3MF files,
eight per-Part STL files and a metrology report. Generator checks include
native mating intersection, source identity and STEP/reopen volume budgets.

The [independent audit](/private/tmp/cad218-artifact-independent-audit.json)
passed 24 payload hashes, 12 raw print meshes and 208 material-section probes.
It parses actual TCD checksums/source, binary STL and transformed millimetre
3MF triangles/volumes/bounds. Source equations independently locate distinct
crest/root, flank and handed-phase probes. It checks recorded native facts
against budgets without independently reevaluating B-reps.

Maximum STEP volume delta is 0.000046874 mm3, within 11.257 percent of its native
budget. Reopen delta is zero. Raw mesh volume discrepancy uses at most
11.025 percent of its chord/area budget; maximum bounds delta is 0.001498 mm.
Written mesh files contain 130,776-431,676 triangles.

A separate [actual-STL measurement](/private/tmp/cad218-measured-artifact-pitch.json)
sections each left-hand external STL and bisects real crest boundaries at
two axial turns and a noncommensurate 0.43-radian angular step. Measured
adjacent pitch and signed lead are:

| Left-hand fixture | Measured pitch (mm) | Measured signed lead (mm/revolution) | Requested signed lead (mm/revolution) |
| --- | ---: | ---: | ---: |
| Rd10x1/10 | 2.539999818 | -2.543588549 | -2.54 |
| Rd20x1/8 | 3.175000000 | -3.176226446 | -3.175 |

The chord-deviation/3D-helical-normal/float32-derived bounds pass. These
measure actual triangle geometry, not metadata multiplication.

## Native package and metric boundary

./build passed in 28.98 seconds. Package:
artifacts/development/build-jaaCBx/MultiCAD.app. Executable SHA-256:
0f1c250643f8fda79c8b407c1ec5cc7cf3a123cf1c7c083b508a7361a8c2ee0f.
Diagnostics and native self-test passed. The unstamped executable is attributed
through the frozen source/build record and checked runtime family coverage.

Four [separate packaged stdio sessions](/private/tmp/cad218-runtime-fixtures-result.json)
reopened all saved projects, compared complete canonical source and native
volume, and exported both members with STEP readback. Every process exited
normally with code zero. These checks ran alongside workspace tests and are
not isolated latency measurements.

All 15 existing metric prototype B-reps, totalling 11,294,004 bytes, remain
byte-for-byte unchanged. Five prototype tests passed, including exact lookup,
new-input refusal and cold loading all 19 catalog recipes from 15 prototypes.
Four-worker regeneration took 34.746 seconds. Source fingerprint:
c02940b1237e44fa33a1c5b4a19666624679b5452d12f290f43582b78c1a8ee7.

Painted M3 L10/B10 measurements were 591.906/125.340 ms first/second preview,
63.430/118.893 ms unchanged preview and 60.752/118.306 ms Apply-to-canonical ready.
These single offscreen observations are not OS latency or p95 results.
An [isolated packaged M3 run](/private/tmp/cad218-metric-perf-8wS1Fd/report.json)
measured 518.644 ms for the first default L10/B10 screw, 2.626 ms for an
unchanged edit, 9.895 ms for a second identical new screw and 382.146 ms to
change the first screw to L11 while retaining B10. The helper checked actual
canonical dimensions, latest round coverage, unchanged binary identity and
normal process exit. All owned builds, tests and generators had exited.
These are single native MCP observations, not OS-window latency or p95 results.

## Required checks and acceptance

Formatting and strict locked workspace/all-target Clippy passed on `61bc653`.
The full locked workspace suite passed 2,537 tests across 289 result groups
with zero failures and four ignored in 2,617.294 seconds. Actual OS interaction,
Windows/Linux runtime, physical fit/gaging/load and human acceptance remain
unverified. No installation, push or publication occurred.

## Evidence files

- [Core source/reader matrix](/private/tmp/cad218-core-final.log)
- [Native circle matrix](/private/tmp/cad218-kernel-split-crest.log)
- [Native pair/export matrix](/private/tmp/cad218-model-final-split.log)
- [MCP lifecycle](/private/tmp/cad218-mcp-final.log)
- [Painted inspector/reopen](/private/tmp/cad218-ui-final.log)
- [Metric prototype audit](/private/tmp/cad218-metric-assets-audit.json)
- [Packaged runtime](/private/tmp/cad218-runtime-result.json)
- [Required checks](/private/tmp/cad218-required-checks-result.json)
