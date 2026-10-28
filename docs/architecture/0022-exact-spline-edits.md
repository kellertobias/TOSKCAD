# ADR-0022: Segment exact splines and bound tangent continuation

| | |
|---|---|
| Status | Accepted engineering design; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

Sketch Trim and Extend must preserve rational and polynomial curve geometry.
Display segments provide useful picking feedback, but cannot determine exact
intersections or become canonical spline source. The native query can take long
enough to require cancellation and protection against concurrent document edits.

## Decision

Segment supported open clamped splines through the native kernel on a private
curve copy. Persist the returned exact basis and poles. Keep a single survivor's
curve identity; record explicit replacements when an interior cut splits it.
Preserve existing points and unaffected constraints. Reject constraints on
changed target geometry when their intent cannot be preserved.

Extend an endpoint with a finite straight span along its geometric tangent.
Preserve the original native interval, poles and rational weights. Insert a
degree-multiplicity join and endpoint-weight collinear poles. This guarantees
geometric tangent continuity (G1); it does not promise parametric C1 continuity.
Expose a maximum length and require a visible choice when several contacts exist.
Reject absent, tangent, overlapping and ill-conditioned contacts explicitly.

Run native queries on one bounded background worker with one latest queued
request. Give it a read-only preparation snapshot retaining the live session's
identity and source revision. Only the live session can commit the resulting
prepared transaction. Check the complete evaluation stamp, request, source,
session and query context before adopting a result.

Treat numeric coordinates as strict geometric input. Mark viewer picks separately
and project them onto the exact curve within the captured click camera's
eight-pixel radius and a finite world-space bound. A later zoom cannot enlarge
that original pick authority. Display segments identify candidates only.

Bound continuation to 1,000,000 mm, 256 total poles and the kernel's coordinate
policy. Bound selection to 128 other curves and intersection catalogues to 256
contacts. Closed spline seam edits and analytic extension against spline
boundaries remain explicit unsupported operations.

## Consequences

Exact source stays editable and supports native closed-region and solid building.
Cancel and stale results leave canonical source unchanged. UI and external callers
use the same typed solver preparation and transaction checks.

Retaining existing points can leave unused control points in the sketch. The
conservative constraint policy rejects some edits that a future explicit repair
workflow could support. Tangent continuation adds a straight segment rather than
predicting the designer's intended curvature. Destructive Trim does not create an
associative relationship to its boundary.

## Alternatives considered

Polynomial extrapolation can preserve higher derivative continuity near a join,
but may turn or diverge before reaching the selected boundary. Straight bounded
continuation gives the user a finite, inspectable rule.

Refitting sampled points simplifies integration with display geometry, but changes
the retained curve and loses rational intent. Native segmentation preserves its
exact mathematical representation within the declared numerical policy.

Synchronous native queries avoid worker ownership machinery, but block input
during geometry checks. The bounded worker preserves input responsiveness without
giving background code independent commit authority.

## References

- [Canonical drafting and diagnostics](../../crates/multicad-core/src/sketch_trim/exact.rs)
- [Native spline operations](../../crates/multicad-kernel/src/spline_edits.rs)
- [Worker ownership and projection](../../crates/multicad-ui/src/sketch_trim_worker.rs)
- [Host adoption guards](../../crates/multicad-ui/src/sketch_trim_host.rs)
- [Verification and supported limits](../verification/native-spline-editing-2026-10-28.md)
