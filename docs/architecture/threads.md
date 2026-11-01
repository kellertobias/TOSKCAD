# Parametric threads

The [`model.thread` source](../../crates/multicad-core/src/model/threads/mod.rs)
stores an editable thread feature and its manufacturing representation.
The native [evaluator](../../crates/multicad-model/src/threads.rs) creates a
threaded shaft or cuts a thread into an existing Part.
The Mechanical inspector and [MCP tools](mcp-server.md#thread-control)
prepare the same canonical transactions.

## Family coverage and sources

The [profile registry](../../crates/multicad-core/src/model/threads/profiles.rs)
owns supported families and diameter/pitch presets. Unavailable families are
disabled in the inspector and refused by the canonical API.

| Family | Implemented geometry | Dimensional basis and limits |
| --- | --- | --- |
| ISO metric basic profile | Straight, truncated 60-degree profile; listed coarse and fine pitches | NBK basic-profile table citing JIS B 0205-2/4:2001; nominal dimensions only |
| Custom | Straight or tapered axial profile with explicit depth, crest width and two flank angles | User-defined dimensions; no standard designation or tolerance class |
| Unified UNC / UNF | Unavailable | No sourced geometry or tolerance coverage implemented |
| Whitworth BSW / BSF | Unavailable | Rounded profile and standard size coverage unimplemented |
| BSPP | Unavailable | Parallel pipe profile and standard size coverage unimplemented |
| BSPT | Unavailable | Tapered pipe profile, gauge planes and standard size coverage unimplemented |
| NPT | Unavailable | Tapered pipe profile, gauge planes and standard size coverage unimplemented |
| Trapezoidal | Unavailable | Standard profile and size coverage unimplemented |
| Acme | Unavailable | Standard profile and size coverage unimplemented |
| Buttress | Unavailable | Standard asymmetric profile and size coverage unimplemented |
| Round | Unavailable | Rounded profile and standard size coverage unimplemented |

The metric data comes from the
[NBK metric-thread technical reference](https://www.nbk1560.com/images/en-US/product/technical_data/meternezi_NBK/meternezi_NBK_1.pdf),
consulted on 2026-11-01. For pitch `P`, the implemented radial depth is
`5*sqrt(3)*P/16`, the crest width is `P/8`, and each flank angle is 30 degrees.
The registry contains only explicitly listed diameter/pitch pairs.
It includes nominal diameters 2, 2.5, 3, 3.5, 4, 4.5, 5, 6, 7, 8, 9 and 10 mm,
plus selected larger sizes through 30 mm.

This basic profile does not implement ISO 965 tolerance classes or certify
compliance with the current ISO 724 edition. A Custom tapered profile does
not establish NPT or BSPT compliance. Those families require their own sourced
profile, gauge-plane and tolerance rules before they can become available.

## Parameters and receiving geometry

The definition separates nominal diameter and pitch from radial and axial
fabrication allowances. Positive radial allowance shrinks an external thread
and enlarges an internal cut. Axial allowance reduces external tooth width
or enlarges the internal cutting profile. Nominal pitch diameter remains a
basic-profile measurement independent of these allowances.

Pitch and lead are separate: `lead = pitch * starts`.
The source retains handedness, representation and a local orthonormal frame
with a unit-bearing origin. The Custom radius taper is radius change per
axial millimeter; diameter taper is twice that value.

`new_external` creates a Part and feature. Receiving modes append a feature
to the selected Part and retain its exact predecessor in the source.
Historical edits preserve that predecessor, Part identity and frame unless
the caller explicitly edits frame parameters.

The native evaluator requires a complete nominal shaft band for an external
cut and retains material outside that band. An internal cut requires a
complete annulus through the requested length, including the minimum wall.
Wrong shaft radius, incomplete stock and insufficient walls fail before Apply.
The result must remain one connected solid.

## Native construction and output

The [native adapter](../../crates/multicad-kernel/src/threads.cpp) sweeps a
profile along a cylindrical or conical helix, joins each ridge to the core,
and clips the joined result to flat ends. Both hands and multiple starts use
the same construction. Flat cut ends are the only supported runout;
automatic relief grooves and chamfers are unimplemented.

The [kernel input policy](../../crates/multicad-kernel/src/threads.rs) bounds
construction to 1-8 starts, pitch of at least 0.001 mm, and length of at least
one pitch. It limits `length/pitch + 2*starts` to 128.
Profiles also need a positive root, separated adjacent starts and resolvable
crest width. Flank angles must lie within 1-80 degrees and absolute radius
taper within 0.1.

Modeled threads contain helical native faces. Cosmetic envelopes retain
thread intent but produce only a major external envelope or minor internal
bore. A cosmetic thread in a selected producer's dependency history makes
STEP, mesh, planar manufacturing-outline and Print output unsupported.
Switching the source to Modeled rebuilds manufacturing geometry.

The native tests cover
[kernel pitch, handedness, starts, taper and STEP readback](../../crates/multicad-kernel/tests/threads.rs),
[canonical transactions and persistence](../../crates/multicad-core/tests/model_threads.rs),
and [receiving solids, mating clearance and export refusal](../../crates/multicad-model/tests/threads.rs).
These checks establish the tested geometry cases, not physical fit on a
particular printer or standardized tolerance compliance.
