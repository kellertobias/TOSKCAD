# Trapezoidal and Acme thread verification, 2026-11-03

The implementation at `e5d2c00` adds separate metric Trapezoidal and inch
Acme nominal lead-screw profiles through the canonical inspector and MCP API.
Native, saved-source, UI and manufacturing checks passed. All required
checks passed: 2,510 tests passed, zero failed and four ignored across
283 result groups. Actual OS interaction and human
acceptance remain unverified.

## Source and construction scope

The [lead-screw reference](../architecture/threads.md#trapezoidal-and-acme-lead-screw-profiles)
owns editions, equations, coverage and limits. Metric Tr10x2 and Tr12x3 use
ISO 2901:2016 design profiles and ISO 2904:2020 rows, with a 30-degree axial
angle. Acme 3/8-12 and 1/2-10 use NBS H28 (1957) Part III, issued in 1960,
with a 29-degree axial angle and source general-purpose crest/root clearance.

The basic profiles have zero theoretical flank allowance. They do not claim
ISO 2903 or Acme G-class fitted product dimensions. The internal cutter uses
its own source major diameter and narrower crest width while sharing the
external basic pitch diameter. Sharp limiting corners remain distinct from
manufactured rounded roots. Source crest/root clearance and additional
fabrication allowance are separate.

Existing authored fields capture the single pinned convention: family,
matched nominal diameter/pitch, NominalBasic, hand, starts and allowances.
The resolved receipt reports source row, construction dimensions and explicit
start count, lead and RH/LH. One to eight starts and either hand are bounded
engineering geometry, not certification of every standard lead/fit combination.
The profile uses adjacent pitch; the helix uses pitch times starts.

## Native geometry and measured lead

No kernel code or trusted metric assets changed. The existing parallel native
helix constructs the two distinct axial forms. Five exact native pairs cover
all four right-hand presets and a left-hand two-start Tr12x3 fixture. Each
uses explicitly saved 0.005 mm radial fabrication allowance per member.
This additional clearance is not a sourced fit-class allowance.

The [model tests](../../crates/multicad-model/tests/lead_screw_threads.rs)
passed two tests in 60.96 seconds. They check actual material on both sides
of each flank at two radial depths, empty mating intersection, start ancestry,
fractional end planes, real STEP readback, raw print sections, source reopen,
receiving-wall refusal and cosmetic print refusal.

The Rust test independently sections written STL triangles at one radius
and locates crest centers at angles zero and 0.43 radians. Separate native
material probes check both flanks at two radial depths at angle zero.
Measured adjacent-ridge spacing is 3.000083484 mm. A noncommensurate
0.43-radian angular step gives signed lead -6.000255836 mm/revolution, against
requested 3 mm pitch and 6 mm left-hand lead. This distinguishes pitch,
lead and hand rather than asserting a metadata multiplication.

A separate [raw-file parser](/private/tmp/cad216-measured-artifact-lead.json)
checks the retained review STL using its own binary reader. It measured
3.000006136 mm pitch and -5.999409864 mm/revolution signed lead. Bounds are
derived from the export chord deviation and axial flank angle. This measured
LH/multiple-start export evidence specifically covers Tr12x3 with two starts.

## Canonical workflows and production inspector

Five new core tests and five existing registry tests passed. They verify
source dimensions, mode-specific construction, supported and rejected rows,
actual radial/axial fabrication-allowance effects, atomic failure and source
history/save/reopen. The conditional reader gate covers direct, suppressed,
template-only, nested and retained-reference sources.

Three MCP tests passed in 30.09 seconds. They cover actual native owner
preview/reject/apply, source units and lead, cancellation, omitted-field edits,
Undo/Redo, independent saved-session reopen, STEP and nested nut/screw receipts.
Ten production thread inspector tests passed in 39.40 seconds, including
existing metric, Unified, rounded, BSPT and NPT controls. The new controls
preserve authored length, operate RH/LH and starts, and use the same native
Preview/Cancel/Apply and history gate. The explicit designation follow-up
passed in 4.35 seconds. These are offscreen production egui interactions.

## Review artifacts and independent raw audit

[Review artifacts](../../artifacts/lead-screw-review-20261103-verified/report.json)
contain five editable TCD projects, ten STEP files, five combined 3MF files,
ten per-Part STL files and a source/metrology report. The generator checks
native mating intersection and STEP/reopen volume for all five cases.

The [independent audit](/private/tmp/cad216-artifact-independent-audit.json)
passed all 30 payload hashes, 15 raw print files and 20 helical material
sections. It checks TCD manifest/source checksums and parses binary STL and
transformed millimetre 3MF triangles, volumes and bounds independently of
writer receipts. It compares against recorded native metrology under the
chord/area budget; it does not independently reevaluate native B-reps.
Maximum STEP volume delta was 0.000078876 mm3, within 36.43 percent of its
native tolerance/area budget. Reopen delta was zero. Raw volume discrepancy
used at most 9.223 percent of its conservative chord/area budget; maximum
raw/native bounds delta was 0.001496 mm. Export triangle counts ranged from
15,586 to 59,640.

## Packaged runtime and performance boundary

The actual `./build` passed in 28.25 seconds. The packaged app is
`artifacts/development/build-lzGeHT/MultiCAD.app`; executable SHA-256 is
`185cff75bd298ba3857e0d0924d6880fa285bbd1e6d65b54d459c9f8c500d911`.
Diagnostics and native self-test passed, including all four bundled examples
and STEP exports. The executable is unstamped; source attribution comes from
the frozen workspace/build record.

Five [separate packaged stdio sessions](/private/tmp/cad216-runtime-fixtures-result.json)
independently reopened all review projects, evaluated their native solids,
matched complete canonical source and native volume, and exported both
members with STEP readback. All processes exited zero. These checks ran
alongside workspace tests and are not isolated latency measurements.

The native prototype fingerprint remains
`c2f5df006b0620eadbf1c11b81ad3a31799d936bf4fd68275653ffac9b2f47ef`.
No kernel source or metric B-rep asset was changed.

After the build and workspace suite exited, an isolated packaged stdio MCP
check measured first default M3 L10/B10 creation at 512.675 ms, unchanged edit
at 2.532 ms, second identical new screw at 9.464 ms, and an underhead L11 edit
retaining B10 with both screws present at 379.992 ms. Canonical dimensions and
revisions were checked; the process exited zero and the executable stayed
unchanged. These are single observations, not OS-window timing or p95 results.

## Required checks and remaining acceptance

Formatting, strict locked workspace/all-target Clippy and the full locked
workspace suite passed on `e5d2c00`. Workspace tests took 2,082.70 seconds.
OS-window interaction, physical fit, calibrated gaging, fitted tolerance
classes, load/life, Windows/Linux runtime and human acceptance are unverified.
No installation, push or publication occurred.

## Evidence files

- [Focused core](/private/tmp/cad216-core-final.log)
- [Focused native model/export](/private/tmp/cad216-model.log)
- [MCP lifecycle](/private/tmp/cad216-mcp.log)
- [All thread inspector checks](/private/tmp/cad216-ui-all-thread-controls.log)
- [Explicit lead designation](/private/tmp/cad216-ui-designation.log)
- [Packaged runtime](/private/tmp/cad216-runtime-result.json)
- [Required checks](/private/tmp/cad216-required-checks-result.json)
- [Isolated packaged metric timing](/private/tmp/cad216-metric-perf-C9oVak/report.json)
