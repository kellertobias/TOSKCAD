# Native measurements

Measure evaluates persistent A/B references against the current native model.
The [canonical inspection types](../../crates/multicad-core/src/inspection/mod.rs)
share the document transaction system with other tools. The
[native service](../../crates/multicad-model/src/inspection/mod.rs) owns numeric
evaluation; the UI receives source-stamped values without native CAD handles.

## Source intent and derived results

A temporary measurement contains two source identities and their geometry
intents. Pin stores those intents in the Extension's canonical links, together
with label settings. It stores no measured numbers and adds no sketch constraint.
The typed definition requires `inspection.measurement.1` and limits retained
pins to 128, as defined in
[`inspection/source.rs`](../../crates/multicad-core/src/inspection/source.rs).
Pin, Update and Remove each prepare one checked, undoable transaction.

A result identifies its document, revision and complete source hash. Missing,
ambiguous, unsupported and stale references remain explicit failures. A pin
whose source disappears retains repairable intent without displaying its old
value. Valid typed measurement diagnostics do not block unrelated solid display;
the full diagnostic report remains available for repair.

## Geometry and units

The [distance adapter](../../crates/multicad-kernel/src/distance.rs) calls native
B-rep extrema for finite faces, edges or vertices. Current placed occurrences
use their world-space solids. Display meshes identify selected topology and
render highlights; they do not supply measurement values.

Minimum distance is in millimeters. Supporting-plane separation is an additional
metric for parallel planar faces, independent of their finite boundaries.
Straight edges and planar faces also support an unoriented acute angle,
displayed in degrees. Other analytic angle combinations report Unsupported.

Signed axis components describe B minus A only when the service can establish
a unique direction. A finite list of closest-point witnesses alone cannot
establish uniqueness for a curved continuum. Ambiguous directions or a reached
witness budget produce an explanation instead of asserted components. The
adapter returns at most 64 witness pairs, with total-solution and completeness
metadata. Zero distance does not distinguish contact from penetration.

## Worker and interaction ownership

The [measurement worker](../../crates/multicad-model/src/inspection/worker.rs)
constructs, caches and destroys its native model on one owner thread. It accepts
latest-only queries and checks the full source stamp before publication.
Incomplete repair models cannot authorize inspection results. Cancellation
supersedes publication between native calls; it does not interrupt an OCCT call.

The [Inspector feed](../../crates/multicad-ui/src/inspection/feed.rs) starts no
native evaluation until an A/B query or retained pin requires a value.
Foreign request identities, mismatched queries, missing pins and invalid result
units or witnesses fail publication. Hidden documents suspend their workers;
visible pins regenerate when their document returns to a model viewport.

The global Measure command opens a read-only Inspector over the canonical model.
It preserves pending source drafts and returns to the originating mode and
workspace tab when closed. Pin actions require completed source edits.
Changing files closes temporary measurement input. A and B retain distinct
highlight colors, including different faces on the same Part. Replacing one
side preserves the other reference. Closing Measure removes its temporary
overlay while visible saved pins remain.

## Clearance viewport evidence

Interference/Clearance results select into the viewport. The first known
current-pose pair is selected when its report arrives; clicking a pair toggles
it. The [marker](../../crates/multicad-ui/src/assembly/clearance_view.rs)
highlights both occurrence roots and draws every native witness pair with A/B
endpoint colors and a labeled distance. Contact and overlap add a ring, and
overlap also adds a cross with its COMMON volume. Fit result frames both
occurrence boxes and all witnesses by moving only the camera. Reveal targets
outlines both occurrences over occluding geometry. An occurrence hidden from the
draw list is always outlined from its native leaf placement.

Witness endpoints and numbers come only from the exact report. Occurrence boxes
come from display meshes and only locate the pair. A marker is built only when
the viewed Scene is the complete model of the report's document, revision and
source hash. Otherwise the Inspector says why markers are hidden. Sampled motion
poses are not displayed, so witnesses cover the current pose only.

## Verification

The [Inspector tests](../../crates/multicad-ui/src/inspection/tests.rs) use real
compiled solids for face, edge and point picking, native distances, replacements,
source changes, missing sources and pin save/reopen. Actual egui actions check
Pin, Update, Remove and narrow-panel layout. The
[application tests](../../crates/multicad-ui/src/application/measurement_tests.rs)
check the global button, workspace restoration and pending-draft guards.

The [review generator](../../crates/multicad-app/examples/measurement_review.rs)
writes editable native equipment-envelope projects with a parameter-driven gap,
a retained measurement and a missing-source case. It refuses an existing output
directory and verifies native project round-trips. Interactive macOS evidence,
Windows/Linux runtime checks and low-power timing require separate verification.
