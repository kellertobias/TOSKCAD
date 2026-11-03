# Tapered pipe thread verification, 2026-11-03

The implementation at `cacfbd0` provides bounded sourced R/Rc pipe threads
through the canonical inspector and MCP recipe. Native geometry, saved-source
workflows and manufacturing exports passed focused checks. All required checks passed: 2,483 tests passed, zero failed and four ignored
across 278 result groups. Actual OS-window interaction and human acceptance
remain unverified.

## Supported construction

The [thread reference](../architecture/threads.md#tapered-pipe-construction)
owns editions, selected rows, gauge conventions and tolerance rules.
The implementation supports right-hand, single-start R external and Rc internal
forms in the two listed pipe sizes. Rp, other sizes, left-hand designation,
axial clearance and incomplete runout remain unavailable.

Saved intent retains the pipe form, nominal size and pinned gauge convention.
The base diameter keeps its established local-reference meaning. Resolved
metadata separates the nominal plane, allowance displacement and actual gauge
offset. Old-reader refusal covers direct, suppressed and template-only source,
nested hardware, and retained literal references after a family change.

## Native geometry and clipping regression

The tapered profile has actual tangent circles and axial 55-degree flanks.
A dedicated meridian frame preserves their shape along the conical helix.
The native adapter retains exact source phase and fractional end planes.

A 1/4-inch external recipe with length 9.86 mm exposed a native classification
failure with half-turn spline faces. Its surface volume was 1160.421082164 mm³,
against independent analytic integration of 1160.421081242 mm³. An independent
triangle section contained the disputed crest point. The native classifier
and a tiny-cube Boolean both reported empty material; those checks share
OCCT classification machinery and are not independent of each other.

Quarter-turn tapered domains preserve the same authored geometry and pass
classification with ordinary end trimming. Parallel and flat-profile paths
retain their existing representation. The exact failing internal acceleration
step was not isolated. Temporary diagnostics and alternative trims are absent
from the committed implementation.

The [native regression](../../crates/multicad-kernel/tests/tapered_rounded_threads.rs)
checks all four sourced form/size recipes, gauge-cone and circle tangencies,
flank material, exact end bounds and independent volume quadrature.
Both tests passed in 107.32 s. The actual location-law factory also passed
80 samples covering both hands, taper signs, nonzero phase and later segments.
Its legacy-frame control differs from the intended frame.

## Canonical workflows and raw exports

The painted production inspector test passed family re-entry, R/Rc mode
changes, authored-length preservation, native Preview/Cancel/Apply, history
and saved-feature reopening in 19.84 s. This is an offscreen egui/native-backend
check. MCP checks cover owner review/rejection/one-time acceptance, omitted
edits, cancellation, stale revisions, nested screw/nut inspection and save/reopen.
An unchanged nested edit correctly preserves its revision.

The final model target passed three tests in 280.03 s: sourced mating pairs,
strict STEP readback, raw print sections, independent-session rebuild,
cosmetic-output refusal and insufficient receiving-wall rejection.

The review directory is
[pipe-thread-review-20261103-verified](/Users/keller/repos/_apps/multicad/artifacts/pipe-thread-review-20261103-verified).
Its 13 files contain two editable projects, four STEP files, two 3MF packages,
four STL files and a report. Independent auditing checked every payload hash,
six print receipts and eight STL material/void section probes. Mesh bounds
and volumes stayed inside their declared budgets. Recorded STEP volume deltas
are at most 2.1123e-10 mm³; independent-session rebuild deltas are zero.
Both native mating intersections were empty. The raw-file audit did not rerun
native mating or STEP evaluation.

Evidence: [native tests](/private/tmp/cad214-native-final-domains.log),
[model tests](/private/tmp/cad214-model-verified.log),
[artifact report](/Users/keller/repos/_apps/multicad/artifacts/pipe-thread-review-20261103-verified/report.json),
and [independent raw-file audit](/private/tmp/cad214-artifact-independent-audit.json).

## Metric performance and required checks

The regenerated metric cache has fingerprint
`9b2ba788316e94ccfe9e814205097a19d4fe7205073b78ad3c6292197f5cb93e`.
All 15 BRep assets remain byte-identical to the prior verified assets and
cover 19 shipped catalog recipes plus default M3. The
[independent asset audit](/private/tmp/cad214-quarter-final-metric-asset-audit.json)
checks source identity, keys, policy, hashes and authored/template mappings.

The committed-source offscreen default-M3 controller check measured first and
second independent previews at 592.745 and 127.748 ms. Unchanged previews took
63.756 and 121.401 ms; Apply to authoritative readiness took 62.024 and
119.821 ms. These are single observations, not OS-window or percentile timings.

The native build passed in 28.67 s, producing
[build-C9GVP1](/Users/keller/repos/_apps/multicad/artifacts/development/build-C9GVP1).
Packaged diagnostics and the native STEP/bundled-example self-test passed,
with the owned process exiting zero. The
[runtime evidence](/private/tmp/cad214-final-runtime-result.json) records binary
SHA-256 `26a4e8b2816f2cc071e40bc1d7e36fedaa54fb26b9e40f7796742302922425fa`.
Diagnostics identify an unstamped development executable; source attribution
comes from the frozen workspace and build evidence, not executable self-report.
After the workspace suite ended, the same packaged executable passed the
[headless default-M3 check](/private/tmp/cad214-metric-perf-EMVo9w/report.json):

| Native MCP operation | Time, ms |
| --- | ---: |
| First default M3 L10/B10 | 513.291 |
| Unchanged edit | 2.466 |
| Second identical screw with new identities | 9.493 |
| Edit first to L11/B10 with both screws present | 377.364 |

The process exited zero and its executable hash stayed unchanged. These are
single observations of the native API, not OS-window or percentile timings.
Formatting and strict workspace Clippy passed. The complete locked workspace
suite passed on `cacfbd0` in 1,964.44 s. The
[check report](/private/tmp/cad214-required-checks-result.json) records exact
commands, exit codes and result counts.
No installation, push or publication was performed. Geometry and export checks
establish neither calibrated-gage acceptance nor physical sealing or strength.
