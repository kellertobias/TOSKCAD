# ADR-0030: Keep explicit Split roles and semantic output pairs

| | |
| --- | --- |
| Status | Accepted engineering design; implementation and native UI acceptance pending |
| Date | 2026-11-02 |
| Decider | Codex within CAD-222 implementation scope |

## Context

CAD-222 requires simultaneous Targets and Splitting tool fields, multiple target
bodies, plane or model cutters and one atomic result. The current Split provider
accepts one target and one closed solid tool. Its persistent inside/outside
identities are specified by [ADR-0014](0014-semantic-solid-partitions.md).

## Decision

Keep the existing `model.split` schema 1 reader and its saved Parts unchanged.
Add a versioned Split source for explicit solid-tool and plane-tool partitions.
A solid tool has InsideTool and OutsideTool outputs. A plane has Positive and
Negative outputs relative to its authored normal. Allocate Part/Feature IDs once
for each target and semantic side. Never assign identities by fragment order,
relative volume or geometric proximity.

The plane is explicit: an authored datum plane with an offset, or a canonical
construction plane resolved against current native geometry. Its dependency and
dimension bindings belong in canonical input links. Plane clipping uses exact
OCCT half-spaces, with native topology history for retained target geometry.
It does not approximate an infinite plane with a display mesh or a cutting box.

Prepare all selected targets as one typed canonical request. Native preview must
validate every output before a single Session commit creates the result Parts.
Copy each target's material and organizational ownership. Preserve frozen input
producer versions and the existing keep-tool membership contract. A failure,
changed source, cancellation or unsupported result commits no target.

Keep the current connected-solid output boundary: every semantic side must be
one nonempty connected solid, and the pair must conserve target volume within
model tolerance. Supported model cutters are closed native solids, partitioned
along their actual boundary with both sides retained. Open shells and disconnected
membership sides require a future persistent fragment contract and receive a
clear diagnostic. This extends input roles and target count, not fragment naming.

The Split form owns both selection fields from entry. Compatible preselection
fills Targets; otherwise Targets is active. Enter advances to Splitting tool,
and the next valid Enter requests execution through the same native validation
gate. Clicking either field changes selection focus. Required fields never depend
on completing a preceding hidden prompt. Preview and error displays retain the
distinction between target and cutter geometry.

## Consequences

Old projects and their semantic references continue to use schema 1. New plane
partitions retain side identities when the cut moves and relative volumes reverse.
Multi-target operations have one Undo entry and cannot leave a partial result.
The existing generic target-acquisition gate must be bypassed for Split so it
cannot hide the second required field or require two selected bodies for a plane.

General multi-fragment and open-surface splitting remain unsupported. That limit
must be visible before mutation; arbitrary native fragment enumeration remains
unsuitable for persistent Part identity.
