# Internal ring gear verification, 2026-11-04

CAD-227 adds bounded internal spur rings and compatible external pinions through
the canonical Mechanical inspector and MCP API. Implementation 9930e82 is
committed on main. Focused source, native, API and painted UI checks passed.
Required formatting and strict workspace Clippy passed. Packaged diagnostics,
self-test and both independent saved-project sessions passed. The complete
workspace suite passed 2,564 tests with zero failures and four ignored. Actual macOS
interaction and human acceptance are unverified.

## Sourced subset and canonical intent

The [gear reference](../architecture/gears.md#internal-ring-subset) owns the
source editions, equations and exclusions. The initial subset is unshifted
20-degree spur geometry: a 24-tooth pinion with a 48- or 60-tooth ring. KHK's
internal involute/trimming inequalities and strict standard-20-degree tooth
difference policy are checked before preparation. The dated KHK-USA article
corroborates dimensions and resolves a contradictory external-root sign in the
live table. An ambiguous printed trochoid operator is not guessed.

These are implementation bounds. They do not imply a KHK catalog part, a
simulated pinion-cutter trochoid, manufacturing cutter access, current-standard
certification or a strength/physical-fit rating. Other counts, angles, shifts
and internal helices remain unavailable with explicit diagnostics.

The ring stores an optional `outer_diameter` alongside the existing typed
module, count, width and thinning inputs. External serialization omits it.
The inward toothed opening is derived, and the separate bore input must be
zero. Root-wall validation measures radial material beyond the outward roots.
Expressions retain parameter links. Backlash and fabrication allowance remain
independent material-tooth thinning rather than an invented fit class.

The [reader regression](../../crates/multicad-core/tests/model_internal_gears.rs)
checks `model.gear.internal.1` for direct, suppressed and template-only ring
source, refuses a reader lacking that capability and independently reopens the
saved project. The structural outside-field guard also rejects invalid
external/source-schema combinations before saving.

## Actual native profile and pair geometry

The [adapter](../../crates/multicad-kernel/src/gears.cpp) constructs a
complementary cavity whose flanks follow the internal material-width law.
Its circular fillets lie at the outward material roots. It preserves a
connected outside rim and checks actual pitch-circle material/space and opening.
An unchanged external cutter would have the wrong thinning direction and fillet
location; that construction is not used.

The [kernel matrix](../../crates/multicad-kernel/tests/internal_gears.rs)
checks both ring sizes against independent inward-flank sections, native root,
tip, fillet and outside circles, an independently integrated area/volume and
STEP readback. Both native and STEP volumes meet the unchanged 0.0001 mm3
analytic budget. Negative controls reject invalid radii, fillet domains,
missing rim, separate bore and nonfinite inputs.

Independent code review found an ancestry indexing mismatch between cavity
gaps and material teeth. The final adapter maps each face to its material
tooth, including wraparound. The regression checks actual face centroids:
every tip centers on its numbered tooth and each left/right involute and
fillet lies on the corresponding side. Every native face retains ancestry.
The final focused kernel run passed two tests in 5.43 seconds.

The [model matrix](../../crates/multicad-model/tests/internal_gears.rs)
checks both ring sizes in both member orders. Existing assembly relationships
derive the radius-difference center distance and positive tooth-count speed
ratio, preserving a material-tooth phase convention. Nine phases plus the
current pose have no native material overlap at the authored clearances.
A deliberate half-tooth misphase produces actual positive overlap, establishing
that the success cases do not pass by missing one another.

Geometry and drive edits retain pair/member identities and recompute placement.
Drive-only edits reuse a completed cycle proof while still checking the current
pose; geometry changes invalidate that proof. Preview preparation is source
neutral, and Undo/Redo plus independent TCD reopen retain the complete source.
Sampled static checks do not establish continuous collision-free motion.

## Inspector and shared API

The painted production-egui regression selects each ring/pinion role order
through visible controls, then exercises the real native Preview/Cancel/Apply,
Undo/Redo and saved-session reopen path. The outside/rim controls use explicit
labels; internal rings have no misleading separate bore field. Existing
external inputs retain their previous semantics.

The [MCP matrix](../../crates/multicad-mcp/tests/internal_gears.rs) covers real
creation, omitted outside-diameter retention, explicit clearing, conflicting
clear/replacement refusal, invalid dimensions and cancelled requests without
partial commits. Both pair orders produce the expected native placement and
real STEP output. The [API reference](../architecture/mcp-server.md#gear-control)
and generated schema define the shared field semantics.

Focused results: four core tests, two model tests, two MCP tests and one painted
UI test passed. The complete frozen-source workspace run below includes these
matrices. Painted UI evidence is offscreen; it does not establish OS interaction.

## Retained projects and independent raw-file audit

The reproducible [generator](../../crates/multicad-app/examples/internal_gear_review.rs)
produced [review artifacts](../../artifacts/internal-gear-review-20261104-final/report.json):
two editable TCD projects, four STEP files, two placed 3MF packages and four
per-member STL files. The examples cover both ring sizes and both member
orders, a 13-degree first-member drive and translated pair placement. Each
member saves 0.05 mm design backlash and 0.02 mm fabrication thinning.

The [independent audit](/private/tmp/cad227-artifact-audit.json) passed all
12 payload hashes/lengths, both actual TCD manifests/canonical sources and
reader flags, six raw mesh files and 412 material probes. It reconstructs
nominal dimensions and thinning from saved intent, applies/inverts occurrence
transforms, measures triangle volumes/bounds and checks actual flank material,
opening, rim, pinion bore and same-direction phase in both file formats.
All native chord/area budgets passed. A separate read-only review found no
false-positive defect in the verifier.

The audit checks reported STEP deltas against their declared budgets; it does
not independently reevaluate B-reps or establish fillet curvature from the
mesh. Kernel circle/area tests and native STEP reads supply separate evidence.
Raw files are manufacturing geometry exports, not printer-fit acceptance.

## Cache preservation and final required checks

The new native fingerprint is
30f6406c710eea2b355930db9efc114125f72c8f7f4a02d571457313ad01564f.
Eight-worker regeneration took 30.019 seconds. All 15 existing metric prototype
B-reps, totalling 11,294,004 bytes, remain byte-for-byte identical; only their
source registration fingerprints changed.
[Prototype audit](/private/tmp/cad227-metric-assets-audit.json).

Required `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets --locked -- -D warnings` passed on
frozen 9930e82. `cargo test --workspace --locked` passed 2,564 tests across
295 result groups with zero failures and four ignored in 4,286.807 seconds.
Formatting took 2.926 seconds; strict Clippy took 118.209 seconds.
[Required runner](/private/tmp/cad227-required-checks-result.json).
The native package built at artifacts/development/build-4PJdG4/MultiCAD.app.
Executable SHA256:
43d3bdb498c3deb2c2284ff0c8c44c4d595cd8acb9a99af9ecf62cec5b1d9928.
[Diagnostics and native self-test](/private/tmp/cad227-runtime-result.json) passed.
[Two independent packaged stdio sessions](/private/tmp/cad227-runtime-fixtures-result.json)
reopened the saved pairs, compared full canonical source and actual member
placements, checked native compound volume and all nine/current pair receipts,
and exported both members with STEP readback. Both processes exited normally
with code zero. The executable is unstamped; frozen-source build and binary
hash identify the tested package.

The full suite's late documentation-test delay sampled compiler file search
waiting in directory enumeration of the large developer dependency cache.
[Process sample](/private/tmp/cad227-rustdoc-sample.txt). A separate fresh-cache
run passed both core documentation examples; the original complete workspace
command subsequently finished with code zero, without source or test changes.
[Fresh-cache documentation check](/private/tmp/cad227-doc-recovery.log).
No latency result follows from these concurrent verification runs. Actual OS
interaction, Windows/Linux runtime, physical fit and human acceptance remain
unverified. No installation, push or publication occurred.
