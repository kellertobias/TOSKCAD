# Metric buttress thread verification, 2026-11-04

The implementation at `f6ce5f6` adds nominal ST20x4 buttress threads through
the canonical inspector and MCP API. Native geometry, manufacturing artifacts,
focused source/UI/MCP tests, packaged reopen and all required checks passed.
The locked workspace suite passed 2,523 tests with zero failures and four
ignored across 286 result groups. Actual OS interaction and human acceptance
remain unverified.

## Source and construction

The [buttress reference](../architecture/threads.md#metric-buttress-profile)
owns editions, dimensions, equations and source limits. Coverage is BIS
IS 4696 Parts 1/2/3:2004 nominal ST20x4, with a 3-degree pressure flank,
30-degree opposing flank and mandatory tangent circular external root.
The internal cutter is flat and carries the source relief clearance and
signed crest phase. The axial side of the external tooth's steep flank is
explicit and independent of handedness. No 7e/7H fit class, current DIN
compliance, load capacity or physical fit is claimed.

The exact tangency construction retains source-rounded dimensions separately.
At P4 the circle radius is 0.49708 mm, external depth is 3.471073533 mm and
the internal crest phase is plus or minus 0.1 mm. NominalBasic, one start,
zero axial fabrication allowance and the listed diameter/pitch are required.
The test pairs save an additional 0.005 mm radial fabrication allowance per
member, separate from source clearance.

## Native geometry and display mesh

The [kernel matrix](../../crates/multicad-kernel/tests/asymmetric_root_threads.rs)
passed two tests in 67.06 seconds in the committed workspace run. Four recipes cover both hands and steep-flank
sides. Independent native material probes check the circle and both actual
flank angles; independent profile integration checks native volume. The tests
also check valid single solids, root ancestry, fractional ends and STEP readback.

The [model matrix](../../crates/multicad-model/tests/buttress_threads.rs)
passed two tests in 151.06 seconds in the committed workspace run. Four mating pairs check empty material intersection,
actual external/internal flanks, phase, root circle, internal flat treatment,
end bounds, independent saved-session rebuild, STEP and written print sections.
Receiving-wall and cosmetic manufacturing refusals remain explicit.

Initial default display tessellation failed on an otherwise valid native
external solid. Its fine print-quality mesh succeeded, while a coarse
B-spline face had no triangulation. Bounded finer tessellation now operates
only on the detached copy. Every exact face and the original triangle/index
budget remain required. Manufacturing geometry and its tolerance are unchanged.
The committed tests additionally check complete coarse face coverage,
repeat-mesh equality, low-budget refusal and unchanged native B-rep bytes.

## Canonical workflows

Five core tests passed, covering source dimensions, phase and hand independence,
unsupported inputs without partial commit, history/reopen, suppressed/template
sources, nested/retained reader gates and load-side-only reference mismatch.

Three MCP tests passed in 91.82 seconds. They exercise actual owner preview,
reject/apply, cancellation, atomic invalid edits, omitted-intent retention,
Undo/Redo, independent reopen, STEP and nested hardware receipts. The painted
production-egui test passed preset, authored-length preservation, left hand,
load-side selection, family return, Preview/Cancel/Apply and history. These
checks do not establish actual OS-window interaction.

## Retained artifacts

[Review artifacts](../../artifacts/buttress-review-20261103-verified/report.json)
contain four editable TCD projects, eight STEP files, four combined 3MF files,
eight per-Part STL files and a metrology report. They cover both hands and
both steep-flank sides. Generator checks include native mating intersection,
source identity and STEP/reopen volume budgets.

The [independent audit](/private/tmp/cad217-artifact-independent-audit.json)
passed 24 payload hashes, 12 raw print meshes and 96 material-section probes.
It parses TCD source/checksums, binary STL and transformed millimetre 3MF
triangles, volumes and bounds. Independent source equations locate actual
STL flank/phase and circle probes. It compares recorded native metrology
against budgets without independently reevaluating B-reps.

Maximum STEP volume delta is 0.000153065 mm3, within 29.830 percent of its
native tolerance/area budget. Reopen delta is zero. Raw volume discrepancy
uses at most 5.865 percent of its chord/area budget; maximum bounds delta
is 0.001496 mm. Print files contain 34,872 to 180,116 triangles.

## Package and metric performance

The actual `./build` passed in 28.82 seconds. The packaged application is
`artifacts/development/build-aVIijO/MultiCAD.app`, with executable SHA-256
`373b3edb402085a2f7177369e5d5e6d482ce5b8ed08510fbecd8c463b97435d3`.
Diagnostics and native self-test passed. The unstamped executable is attributed
through the frozen source/build record and checked runtime family coverage.

Four [independent packaged stdio sessions](/private/tmp/cad217-runtime-fixtures-result.json)
reopened the saved review projects, compared complete canonical source and
native volumes, and exported each pair through STEP readback. All processes
exited zero. These checks ran alongside workspace tests and are not isolated
latency measurements.

The source fingerprint is
`85baf7fef632189fa973e8d3c3932ef31f841b94a04016d17892e74f3685a6b0`.
Metric prototype refresh used four workers in 37.522 seconds. All 15 existing
metric B-reps, totalling 11,294,004 bytes, are byte-for-byte unchanged.

The painted default M3 L10/B10 test measured first/second preview-ready times
607.182/127.560 ms, unchanged previews 63.387/121.254 ms and Apply-to-canonical
ready 62.788/119.272 ms. These offscreen single observations are not OS-window
latency or p95 measurements.

After all owned builds, generators, fixture processes and workspace tests
exited, an [isolated packaged stdio check](/private/tmp/cad217-metric-perf-7TUtY8/report.json)
measured first M3 L10/B10 creation at 515.018 ms, unchanged edit at 2.367 ms,
second identical new screw at 9.353 ms and an underhead L11 edit retaining
B10 at 377.643 ms. Complete canonical dimensions and revisions were checked;
the process exited zero and the executable stayed unchanged. These are
single native-MCP observations, not OS-window latency or p95 results.

## Required checks and acceptance

Formatting and strict locked workspace/all-target Clippy passed on `f6ce5f6`.
The complete locked workspace test run passed in 2,342.61 seconds. OS interaction,
physical fit, calibrated gaging, load/life, Windows/Linux runtime and human
acceptance are unverified. No installation, push or publication occurred.

## Evidence files

- [Focused source checks](/private/tmp/cad217-core-final.log)
- [Native root matrix](/private/tmp/cad217-kernel-root.log)
- [Committed workspace native/UI matrix](/private/tmp/cad217-required-workspace.log)
- [MCP lifecycle](/private/tmp/cad217-mcp-final.log)
- [Painted inspector](/private/tmp/cad217-ui.log)
- [Metric prototype audit](/private/tmp/cad217-metric-assets-final-audit.json)
- [Packaged runtime](/private/tmp/cad217-runtime-result.json)
- [Packaged reopen checks](/private/tmp/cad217-runtime-fixtures-result.json)
- [Required checks](/private/tmp/cad217-required-checks-result.json)
- [Isolated packaged metric timing](/private/tmp/cad217-metric-perf-7TUtY8/report.json)
