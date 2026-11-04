# Metric fits and endpoint verification, 2026-11-04

CAD-219 adds a bounded historical metric 6g/6H subset and explicitly custom
entry/end geometry through the shared native inspector and MCP source.
Implementation e167110 and performance change 4d5847e are committed on main.
Focused source, native, UI, API and manufacturing checks passed. The required
workspace suite passed 2,549 tests with zero failures and four ignored.
Packaged self-test and fixture sessions also passed.
Actual OS interaction and human acceptance are unverified.

## Coverage and source basis

The [thread reference](../architecture/threads.md#metric-fit-classes) owns
source editions, exact table limits, target equations and unsupported coverage.
BIS IS 14962 Parts 1/2:2001 identically adopt ISO 965-1/2:1998. Part 2 printed
pages 2/3 supply M3x0.5 and M6x1 uncoated 6H/6g limits with normal engagement
strictly above the lower bound and at most the upper bound. Part 1 clause 11,
Figure 6 and Table 7 supply external root constraints.

External major/pitch targets are table midpoints; the true tangent root uses
R=P/(4sqrt(3)). Internal major starts at nominal and pitch/minor targets are
independent midpoints. These targets are engineering choices, not required
manufacturing targets or a measured gage result. The source tables do not
supply external minor or internal major limit pairs; neither is invented.
Nominal basic dimensions, unchanged class limits, target geometry and actual
allowance-adjusted diameters remain distinct in the resolved receipt.

Both hands and one start are supported. Other fitted metric sizes/classes
remain unavailable. Existing Nominal basic shapes and catalog pins are
unchanged. No current ISO edition, coating, property-class, load, gage or
physical-fit certification follows from this dimensional construction.

## Explicit custom entry and relief

The [endpoint reference](../architecture/threads.md#custom-metric-endpoint-geometry)
defines an optional 45-degree entry at local z=0 and square cylindrical relief
at z=authored length. They consume an explicit interval within unchanged total
length and retain at least one complete central pitch. Fitted engagement is
checked on that complete central interval. Dimensions preserve expressions.

These are custom engineering operations. Historical Schaeffler/Ettinger DIN 76
shoulder diagrams require shoulder context and do not prescribe a unique
natural-runout ramp; those standard treatments remain unavailable. Nested
screw junction and nut-mouth treatments are refused explicitly. Standalone
metric receiving cuts retain positive core, connectedness and minimum wall
checks. The internal wall envelope includes the deepest entry or relief,
including entries wider than the ordinary major cylinder.

## Native regression and exchange metrology

The [native model matrix](../../crates/multicad-model/tests/metric_fit_threads.rs)
checks both sizes/hands, actual circle material, empty mating intersection,
STEP readback and independent analytic radial-boundary volume. Additional
sections measure entry radius at multiple angles, terminal radius and a
receiving body whose ordinary wall passes but deep-entry wall fails.

The original symmetric-root PipeShell representation produced an M3 native/STEP
volume difference of 0.000013826 mm3 against a 0.000007414 mm3 budget.
Tighter quadrature and a detached SameParameter test did not remove it.
The new symmetric root uses the existing explicit meridian sweep with checked
surface approximation and quarter-turn domains; the old asymmetric buttress
path is unchanged. The retained exchange regression now passes the original
budget. Source dimensions, class limits and physical checks were not widened.
The underlying OCCT trimming/volume-reference discrepancy was not fully
isolated, and no general engine repair is claimed. Temporary diagnostics and
unsuccessful smaller-domain experiments were removed.

## Canonical history, UI and MCP

The [core matrix](../../crates/multicad-core/tests/model_metric_fits.rs)
checks exact class/allowance boundaries, wrong mode/size/start, root truncation,
engagement boundaries, invalid treatment atomicity, saved history and
independent reopen. Reader capabilities cover direct, suppressed, embedded
and template-only sources; metric fits also cover nested and retained-only
hardware references. Legacy absent endpoint fields remain absent in saved data.

The painted production-egui test selects each fit preset, left hand and both
custom treatments, preserves authored length, previews/rejects/applies once,
uses Undo/Redo and independently reopens the saved session. Reselecting the
current metric family retains fit and preview validity. Resolved metric
receipts show diameter/pitch in millimetres without rounded TPI substitution.
This is offscreen production UI evidence, not OS-window interaction.

The MCP matrix checks actual owner-reviewed native Preview/reject/Apply,
cancellation, atomic invalid edits, unchanged source limits after allowances,
omitted-field retention, explicit clearing, clear/replacement conflict,
history, independent saved-session reopen and STEP output. Nested fitted
hardware exposes the same metric receipt and refuses endpoint junction intent.

## Retained files and independent raw audit

[Review artifacts](../../artifacts/metric-fit-review-20261104-final/report.json)
retain four editable TCD projects, eight STEP files, four combined 3MF files
and eight per-Part STL files. Four pairs cover both sizes and hands, with
0.002 mm radial and axial fabrication allowances plus both end treatments.
The generator checks complete source identity, empty native mating,
independent reopen and STEP volume budgets.

The [independent audit](/private/tmp/cad219-artifact-independent-audit.json)
parses payload hashes, TCD manifest/source checksums, binary STL and transformed
millimetre 3MF geometry. It independently reconstructs fitted target and actual
geometry from the published table rows and saved allowances. Actual material
sections cover root circles or internal flats, both flanks, two axial turns,
signed angular phase, two entry sections and the terminal-relief start boundary.
Saved hand, total length and treatment dimensions are checked explicitly.

The audit passed 24 payloads, 12 raw meshes and 448 material sections. Maximum
STEP volume difference is 0.000015093 mm3, using 54.541 percent of its native
budget; independent reopen difference is zero. Raw mesh volume discrepancy
uses at most 8.800 percent of its chord/area budget. Maximum bounds difference
is 0.000000196 mm. Actual files contain 9,318-200,086 triangles. The audit checks
reported native facts against budgets without independently reevaluating B-reps.

## Metric cache and required verification

All 15 metric prototype B-reps remain byte-for-byte unchanged, totalling
11,294,004 bytes. Eight-worker native regeneration took 27.658 seconds and
updated only registry source fingerprints. Native source fingerprint:
eb53d3b46dcc65c3c14cb633e5d58f736032b9f05ae27c6cd043b53ed0f5dadb.
[Asset audit](/private/tmp/cad219-metric-assets-audit.json).

Final focused results include core 6 (0.04s), native model 2 (94.47s on the
parallel implementation), MCP 3 (43.10s before the parallel-only native change)
and painted UI 1 (37.27s before that native change). Final strict all-target
Clippy passed; the complete required runner covers all layers on frozen 4d5847e.

A separate stage profile found 17.375 seconds in three successive serial
joins, versus 0.132 seconds in sweeps. A combined union failed the analytic
volume regression and was discarded. The final change preserves the measured
union order/history and enables stock OCCT parallel work within each new
symmetric-root union. It does not change global pool limits or legacy paths.

The [final isolated packaged run](/private/tmp/cad219-metric-perf-lwKGBv/report.json)
measured default M3 L10/B10 first creation 537.102 ms, unchanged edit 2.654 ms,
second identical new feature 9.718 ms and an L11/B10 edit 378.628 ms. New 6g M3
L3.3/B3.3 first creation improved from 18.528 s to 6.232 s; unchanged edits take
7.515 ms and a second identical fitted screw 20.377 ms. The first fitted
construction remains noticeable and is explicitly not instant. These are
single native MCP observations with workload/build/load recorded, not OS
latency or p95 results. The helper checked actual default/fitted dimensions,
class receipts, unchanged binary identity and normal process exit.

Final package: artifacts/development/build-rzfE2Q/MultiCAD.app; ./build 22.05 s.
Executable SHA256 3abb4a055eb0d69fca9ab2d739f0b0b688fc9d8fc6395bb4b4bbb2c45718b04a.
The unstamped executable is attributed through frozen source/build and runtime
coverage. [Diagnostics and native self-test](/private/tmp/cad219-runtime-result.json) passed. Four independent packaged
stdio sessions reopened complete saved source, checked native volume and
exported both members with STEP readback. Every process exited normally with
code zero. These fixture runs overlapped workspace tests and are not latency
measurements. [Packaged fixtures](/private/tmp/cad219-runtime-fixtures-result.json).
Required formatting, strict locked all-target workspace Clippy and the complete
locked workspace suite passed on 4d5847e. The suite passed 2,549 tests across
291 result groups with zero failures and four ignored in 3,002.868 seconds.
The final rustdoc delay sampled directory enumeration in the large developer
cache; it completed without changing source, tests or acceptance criteria. Actual OS interaction, Windows/Linux runtime,
physical gaging/fit/load and human acceptance remain unverified. No installation,
push or publication occurred.

## Evidence files

- [Required checks](/private/tmp/cad219-required-checks-result.json)
- [Final core matrix](/private/tmp/cad219-core-final.log)
- [Final parallel native matrix](/private/tmp/cad219-model-parallel.log)
- [MCP focused matrix](/private/tmp/cad219-mcp-final.log)
- [Painted inspector](/private/tmp/cad219-ui-final.log)
- [Packaged self-test](/private/tmp/cad219-runtime-result.json)
- [Packaged independent fixture sessions](/private/tmp/cad219-runtime-fixtures-result.json)
- [Serial fitted baseline](/private/tmp/cad219-metric-perf-bhV9ah/report.json)
- [Final isolated native measurements](/private/tmp/cad219-metric-perf-lwKGBv/report.json)
