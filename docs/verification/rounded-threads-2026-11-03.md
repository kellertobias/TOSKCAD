# Rounded thread verification, 2026-11-03

CAD-213 has resumed on local `main` after CAD-252 entered Review/test. Its
rounded-thread implementation is committed at `09ef2e3`, with explicit
reviewed-MCP coverage added in `2fed272`.
The shared MCP owner-stamp correction is the separate commit
`737bd92`. Focused MCP/kernel, model and whole-UI checks passed, followed by
strict workspace Clippy. Artifact generation and an independent mesh audit
passed. The frozen `09ef2e3` workspace baseline completed with 2,415 tests
passed, zero failed and four ignored across 273 result groups. Final combined
source `491b359` also passed all required checks with 2,466 tests passed,
zero failed and four ignored. The final `2fed272` workspace run including the
reviewed-MCP regression passed 2,467 tests with zero failures and four ignored.
Actual OS-window interaction and human
acceptance remain unverified.

## Source and bounded coverage

The [registry](../../crates/multicad-core/src/model/threads/whitworth.rs)
contains six selected rows, each with external and internal limits:

| Series | Size | TPI | Basic physical diameter, mm | Classes |
| --- | --- | ---: | ---: | --- |
| BSW | 1/4 | 20 | 6.35 | Medium external/internal |
| BSW | 5/16 | 18 | 7.9375 | Medium external/internal |
| BSF | 1/4 | 26 | 6.35 | Medium external/internal |
| BSF | 5/16 | 22 | 7.9375 | Medium external/internal |
| BSPP | G1/8 | 28 | 9.728 | Class B external / sourced internal |
| BSPP | G1/4 | 19 | 13.157 | Class B external / sourced internal |

BSW/BSF source is BS 84:2007, Figure 1 and uncoated Medium Tables 3/4 and
9/10. The official BSI publication date is 30 November 2007. The inspected
BSI-authored full text is hosted by a manufacturer; its PDF is not
redistributed. BSPP uses Elesa+Ganter Technical Data copyright 2018,
section 10.6, and NBK's JIS B 0202:1999 basic-form excerpt. Exact primary links,
pages and limitations are in the [thread reference](../architecture/threads.md).
The full publisher standards are not asserted as verified manufacturing or
pressure-sealing compliance. G fractions remain nominal labels; the physical
diameters above are used by geometry. BSPP sealing occurs outside the thread.

For half-angle `a=27.5°`, depth is `P/(3*tan(a))` and circle radius is
`P*cos(a)/(12*(1-sin(a)))`. The analytic formulas retain tangent circles;
rounded printed coefficients remain source-table approximations. Fit targets intersect
all applicable major, pitch and minor ranges before selecting a midpoint;
all bounds are rechecked after radial allowance. The selected full form uses
one start and zero axial allowance. BSW/BSF engagement is at most 15 pitches;
the BSPP source supplies no engagement bound. Other rows/classes/combinations
remain unavailable. These choices are not calibrated-gage or physical-fit
acceptance.

## Implementation and independent review

Separate saved BSW/BSF/BSPP families and fit values use the same retained
`model.thread` recipe. Legacy ambiguous Whitworth remains unavailable. A
conditional `model.thread.rounded.1` reader gate covers saved direct,
suppressed, imported-template, nested mechanical and retained-reference
families. Existing non-rounded source retains its serialized fields and
required capability set.

The native rounded branch uses three narrow swept profiles: a circular crest
with tangent flanks and both circular root shoulders. Buried overlap joins
these to the core; the complete-period self-contact of one wide sweep is
avoided. Old flat-profile construction, source phase, dimensional guards,
volume convergence and face ancestry remain enforced. Independent reviews
found no actionable issue in the source fit calculation, circle/tangent
construction or independent section-integration math.

MCP candidates validate their prepared owner session/revision in the shared
commit path before trusted commit or review queueing. Deterministic thread
and washer tests cover owner edits,
same-revision session replacement, cancellation and draft state. Commit
`737bd92` records the shared owner-stamp correction in
[the MCP commit helper](../../crates/multicad-mcp/src/tools.rs). The focused
owner-reconciliation check and six generic-symbol tests passed, seven tests
in total; final rounded workflows remain a separate verification step.

## Trim-aware bounds defect and correction

The first rounded solid passed native crest/root circle and ancestry probes,
but `AddOptimal` reported a lower axial bound near -0.1547 mm for an intended
zero base. Other runs under-reported a radial extent. Whole exterior-slab
intersections were empty in both native and STEP geometry; vertices and caps
remained within the intended interval. The solid was physically clipped; its
reported bounds were incorrect. No bounding assertion was relaxed.

The correction uses six native minimum-distance queries to exterior planes
covering a conservative non-triangulated enclosure. Finite, converged native
witnesses and projection residuals are checked. It is scoped to doubly curved
BSpline/Bezier faces with degree greater than one in both parameters. Actual
rounded native/STEP faces have degrees 6x9/6x11; legacy flat M6 faces have
only 1x10. Analytic and ruled fast paths remain unchanged.

Each immutable shape owns a shared mutex-protected bounds cache. New geometry
receives a fresh cache; unchanged sharing reuses it. Native and independent
raw STEP support queries recover the 3.175 mm test radius and 0..4.699 mm
extent within approximately `4.04e-8 mm`. Initial queries took about 0.128 s
native and 0.544 s after STEP reload; measured cached queries were 61-62 ns
(legacy flat: 59 ns). Nonconvex-ring, disconnected-box and multi-witness
diagnostics also passed. Temporary diagnostics were removed.

An audit found an old eager cut operation allowed input subshape mutation.
It now configures non-destructive mode before Build. A regression primes
original/shared caches and checks bounds, volume, validity and an independent
BRep reread after the cut, preventing a stale cache from hiding mutation.

## Proper rigid receiver representation

A blind BSW receiver exposed a second defect during the final MCP workflow.
The identity deep-copy rigid transform produced a raw STEP with 5,301 entities
and an out-of-bounds support witness near `z=-0.3247 mm`, although the actual
lower extent was zero. Its projection residual was approximately `0.04054 mm`
with topology tolerance `1e-7 mm`; the unchanged support guard refused it.
The untransformed cutter's 5,250-entity STEP passed. This isolated a
representation change introduced by rigid geometry copying. The correction
preserves the bounds criterion.

The [proper-rigid adapter](../../crates/multicad-kernel/src/bridge.cpp) uses
`BRepBuilderAPI_Transform` with `copyGeom=false`. The pinned OCCT contract for a
direct isometry applies a location while preserving surfaces and pcurves;
Rust validates finite, orthonormal, determinant-positive rigid transforms.
Located modified subshapes retain native ancestry and each transformed shape
receives a fresh bounds cache. Affine and mirror operations retain their
existing paths. Independent review found no actionable issue in this change
or its cache and ownership handling.

The permanent
[receiver regression](../../crates/multicad-kernel/tests/rounded_threads.rs)
checks through and blind receivers with identity and a nontrivial 3D rotation
and translation, four configurations. Native and independently reloaded raw
STEP geometry passed strict world XYZ bounds, analytic volume, crest/root
circle, material and mating-clearance checks in 61.60 s. Original and shared
source geometry, ancestry and primed caches remained unchanged. No bounds,
volume or source-input tolerance was relaxed.

The retained bad 5,301-entity STEP still fails the same support criterion. A
separate retained-input diagnostic passed in 0.16 s; that temporary diagnostic
was removed. The diagnostic establishes refusal of that retained
representation. Arbitrary STEP repair remains outside this evidence. Local logs
are `/private/tmp/cad213-rigid-receiver-proof.log` and
`/private/tmp/cad213-old-bad-support-refusal.log`; the retained input is
`/var/folders/sq/pjy6q88574xgvljn35734kdw0000gn/T/cad213-blind-receiver-3db223f2-49e3-4659-becc-9ef47e54777f/receiver.step`.

## Recorded checks and remaining work

Rows marked prior retain earlier focused evidence. The corrected MCP/kernel,
model, whole-UI and artifact checks passed, followed by the frozen `09ef2e3`
full workspace baseline. These results do not validate later CAD-252 changes.

| Check | Result |
| --- | --- |
| Prior canonical rounded target | Seven passed; 0.17 s |
| Prior canonical metric/Unified targets plus rounded target | 15 passed after updating the supported-family inventory |
| Shared MCP stale-owner correction, commit `737bd92` | Seven passed: one owner-reconciliation check in 2.43 s and six generic-symbol checks in 13.44 s |
| Prior kernel rounded geometry/STEP subset | Five passed; 26.78 s |
| Prior canonical fit/unit/hand/allowance native matrix | 24 configurations passed; 185.25 s |
| Corrected existing kernel thread target | Seven passed; 18.27 s |
| Corrected unchanged mechanical-part target including strict M6 volume test | Four passed; 4.34 s |
| Corrected concurrent cache and non-destructive legacy cut regressions | Two passed; 0.09 s |
| Proper-rigid blind/through receiver regression | Four configurations passed in one test; 61.60 s |
| Retained bad STEP support refusal diagnostic | Passed; 0.16 s; temporary diagnostic removed |
| Corrected rounded MCP native-pair workflow | One test covering BSW, BSF and BSPP pairs passed; 258.15 s |
| Corrected rounded kernel subset | Six passed; 65.85 s; permanent 24-configuration matrix excluded from this rerun |
| Corrected metric screw target | Six passed; 38.90 s |
| Rounded model target | Four passed; 251.09 s |
| Whole UI | 973 passed, zero failed, two ignored: 927 unit tests in 29.68 s plus 46 integration tests; compilation took 82 s |
| Prior Node release contracts | 32 passed; 4,414.49 ms; the current native changes do not alter these contracts |
| Required formatting and diff checks | Passed |
| Strict workspace Clippy, all targets | Passed; 5 min 38 s |
| All-six-row artifact example and independent mesh audit | Passed; six TCD, 12 STEP, 12 STL, 12 3MF and one JSON report |
| Complete three-test rounded MCP target in frozen baseline | Three passed; 261.42 s |
| Complete seven-test rounded kernel target in frozen baseline | Seven passed; 192.41 s, including the fitted matrix |
| Full workspace tests on frozen `09ef2e3` | 2,415 passed, zero failed, four ignored across 273 result groups; compilation 21 min 45 s |
| Subsequent development build | Passed in 57.94 s, but included later CAD-252 working-tree changes; not frozen `09ef2e3` build evidence |

The rounded path uses bounded half-turn edges as its primary representation,
with an additional valid-one-solid pretrim check. Legacy flat-profile
continuous/per-turn behavior stays unchanged. A complete-period rounded sweep
had exposed invalid or contradictory clipping/classification cases; those
results were refused. Smaller surface parameter domains resolve the recorded
cases without accepting zero solids, invalid unions or contradictory material
queries. No pitch, length or physical phase is normalized. Final count,
circle, volume and ancestry guards remain enforced.

The permanent fitted matrix covers each of the six rows with external and
internal fits, right-hand exact 3P and left-hand fractional 3.25P configurations;
the latter use 0.0001 mm radial allowance. The 24-configuration pass used the
same cutter construction before the proper-rigid adapter correction. Exact
BSF, BSW and BSPP regression inputs remain retained. Only diagnostic timing
prints were removed; the matrix remains one of seven permanent tests in
`rounded_threads.rs`. The corrected focused kernel rerun passed the other six
in 65.85 s. The frozen `09ef2e3` workspace baseline subsequently executed all
seven successfully in 192.41 s. Recorded constructor timings
were 5.82-7.03 s for BSW/BSF and 9.25-10.86 s for BSPP, with 38 or 41 faces.
These are native constructor measurements. No OS-window timing was measured.

The corrected MCP native-pair workflow passed after the rigid representation
correction, including all six external/internal STEP exports and readbacks,
source/history checks and saved reopen. Its local log is
`/private/tmp/cad213-rigid-final-mcp-pairs.log`. The other two rounded MCP tests
passed in an earlier run. The complete three-test target subsequently passed
in 261.42 s within the frozen full workspace baseline.

The corrected MCP/kernel pipeline completed with exit status zero. Its
remaining focused logs are `/private/tmp/cad213-rigid-final-rounded.log`,
`/private/tmp/cad213-rigid-final-legacy-kernel.log`,
`/private/tmp/cad213-rigid-final-legacy-mechanical.log`,
`/private/tmp/cad213-rigid-final-machine-screws.log` and
`/private/tmp/cad213-rigid-final-bounds-cache.log`.

The model target passed all four tests in 251.09 s, recorded in
`/private/tmp/multicad-cad213-model-final.log`. Model and artifact checks now
compare independently parsed STL/3MF bounds with native bounds: each separate
mesh uses its selected solid, and the combined 3MF uses the union bounds of
both native solids. Coordinates must be finite. The coordinate budget is
`chord_deviation_mm + 2*f32::EPSILON*scale + 16*linear_tolerance_mm`, where
`scale` is at least one and covers the absolute native bound coordinates.
This accounts for meshing deviation, binary STL coordinate serialization and
native numerical precision. The artifact example records parsed bounds and
the budget; its final execution passed. Independent review found no
actionable issue in these checks.

A test-only engagement assertion now requires a present BSW/BSF limit within
`1e-12 mm` of 15 pitches, replacing exact equality that differed by about
`2e-15 mm` after unit conversion. BSPP still requires an absent limit exactly.
This changes assertion roundoff handling; geometry acceptance is unchanged.

The whole UI passed 927 unit and 46 integration tests, with zero failures and
two ignored tests. Both new painted rounded-thread tests passed. The unit
target took 29.68 s after 82 s compilation; its log is
`/private/tmp/multicad-cad213-ui-final.log`. The initial sandboxed attempt could
not acquire a cache lock. The authorized rerun passed.

Required formatting and diff checks passed. Strict
`cargo clippy --workspace --all-targets --locked -- -D warnings` passed in
5 min 38 s, recorded in `/private/tmp/multicad-cad213-clippy-final.log`. One
mesh-helper range loop was expressed with an iterator; its numerical behavior
is unchanged.

The first artifact run stopped because its checker incorrectly required
object/build counts of one for STL. The STL reader returns facets with zero
object/build records. The checker now requires `(1,1)` for each single-part
3MF and `(0,0)` for STL, retaining unit, triangle, finite-coordinate, bounds
and volume checks. Independent review confirmed the format-specific counts.
After 9.09 s compilation, the corrected example completed successfully in
`artifacts/rounded-thread-review-20261103-final`. The 43 files comprise six
TCD documents, 12 STEP, 12 STL, 12 3MF and one JSON report. Reported STEP/native
volume differences were at most `2.8925816423e-5 mm³`, using at most 22.96% of
the combined comparison budget; saved reopen volume differences were zero.

The independent audit at `/private/tmp/cad213-artifact-independent-audit.json`
passed inventory and all 24 mesh byte/hash comparisons, then parsed raw STL
and 3MF coordinates and transforms to check finite values, units, counts,
volumes and bounds. Each file has 179,506 to 382,302 triangles. Maximum mesh
bound difference was `3.0204433e-5 mm`; maximum mesh volume difference used
6.16% of its budget. This audit checks actual exported meshes and reported
budget arithmetic. It does not independently reevaluate TCD or STEP B-reps
or establish OS interaction.

The full workspace baseline on frozen `09ef2e3` completed successfully after
21 min 45 s compilation: 2,415 passed, zero failed and four ignored across
273 result groups. The log is
`/private/tmp/multicad-cad213-workspace-final.log`. It includes the complete
three-test rounded MCP target and seven-test rounded kernel target.
The queued development build completed in 57.94 s and produced
`artifacts/development/build-6UlHsi/MultiCAD.app`; it read the later CAD-252
working tree and is not frozen `09ef2e3` build evidence. Final required checks
and application verification must cover the completed combined source.

The user rejected the measured interactive fastener delay. CAD-252 now takes
priority: a default M3 L10/B10 headless check took about 6.1 s cold, 6.1 s
unchanged and 12.4 s for the second operation while loaded. These are headless
measurements, not OS interaction evidence. By `491b359`, actual native default
creation measured 499 ms, unchanged reevaluation 2.4 ms and a second new screw
9 ms. Representative repeated Library placements measured 117–171 ms. Current
scope and remaining verification are in the separate fastener performance
report. CAD-253's build-cache fix is committed in
`54dedf3`, with verification report `c330b85`, and submitted for Review/test.
CAD-213 has resumed for the reviewed-MCP regression and final verification;
the item has not advanced to Review.

Actual macOS OS-window interaction, Windows/Linux
CI/runtime and human acceptance remain unverified. This report records no
rounded-thread GUI interaction, installation, push or publication.

Commit `2fed272` adds explicit reviewed-MCP Preview/Cancel/Apply coverage in
`crates/multicad-mcp/tests/rounded_threads.rs`. Each of BSW, BSF and BSPP uses
the actual native thread API before a pending review is returned. The API
exposes no native model receipt, so the test observes the native request
counter and inspects the queued canonical candidate; an accepted native
cache hit is permitted. The owner rejects the first preview without changing
the document, undo/redo or activity history. A second preview is accepted
exactly once, preserving the family, fit and source units, with one MCP-client
Apply activity and one Applied audit entry. A rejected feature cannot be inspected, and duplicate
acceptance cannot mutate the applied source.

The new three-family regression passed in 21.79 s. The entire rounded MCP
target passed all four tests in 104.27 s. Required formatting and strict
workspace/all-target Clippy passed; the latter took 0.59 s incrementally.
Independent review found no remaining acceptance-evidence gap in this path.
This is an in-process reviewed host using real native geometry, not an actual
application socket or OS-window interaction. Only test source changed.
The final `2fed272` workspace run exited zero: 2,467 passed, zero failed and
four ignored across 275 result groups. Log:
`/private/tmp/cad213-final-workspace-reviewed.log`; summary:
`/private/tmp/cad213-final-workspace-reviewed-result.json`.

The final `./build` completed successfully (incremental compilation 0.36 s),
producing `artifacts/development/build-6EuaiB/MultiCAD.app`. Its packaged
`--diagnostics` and `--self-test` with a fresh output directory exited zero;
the smoke test performed a native STEP round-trip and opened all four bundled
examples. Logs: `/private/tmp/cad213-final-reviewed-build.log` and
`/private/tmp/cad213-final-reviewed-runtime.log`. The new regression changes
only tests; production geometry and the validated metric asset fingerprint
remain unchanged from the preceding combined-source verification.
