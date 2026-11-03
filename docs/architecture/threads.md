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
| Unified UNC | Single-start, uncoated UN flat-root profile; 1/4-20 and 5/16-18 | Historical H28 (1969) limits; 2A external and 2B internal; threaded length at most 1.5 nominal diameters |
| Unified UNF | Single-start, uncoated UN flat-root profile; 1/4-28 and 5/16-24 | Historical H28 (1969) limits; 2A external and 2B internal; threaded length at most 1.5 nominal diameters |
| Legacy ambiguous Unified | Unavailable | Select UNC or UNF explicitly; `unified` remains unsupported |
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

The [Unified registry](../../crates/multicad-core/src/model/threads/unified.rs)
uses NBS Handbook H28 (1969), issued March 1970, with USA B1.1-1960 as its
historical dimensional basis. The [profile scan](https://nvlpubs.nist.gov/nistpubs/hb/1970/hb28-scan1.pdf)
contains Table 2.1 and Figure 2.3 on PDF pages 33-36, printed pages 2.01-2.04.
The [limits scan](https://nvlpubs.nist.gov/nistpubs/hb/1970/hb28-scan2.pdf)
contains the four supported rows in Table 2.21 on PDF page 9, printed page 2.26.
Nominal diameters are 0.25 and 0.3125 inch; the series names retain their TPI.
The resolver converts inches with 25.4 mm/in and computes pitch as `25.4/TPI`.

The 2024 [OSG Tap Drill Sizes and Pitch Diameter Limits, Vol 3](https://res.cloudinary.com/osg-usa-inc/image/upload/v1717093846/Literature/03%20-%20Charts%20and%20Guides/OSG_-_Chart_-_Tap_Drill_Size_and_Pitch_Diameter_Limit_-_Vol_3_-_2024_-_IA.pdf),
EDP [800274CA-V3](https://osgtool.com/800274ca-v3/), cross-checks the listed
2B minor and pitch diameters on pages 1-2 only.
It does not establish coverage of a newer full standard edition. The supported
subset excludes UNR rounded roots, coatings, other sizes, other fit classes
and multiple starts. These sources and analytic limits do not certify current
ASME compliance, gage acceptance or physical fit.

For Unified pitch `P`, let `H = sqrt(3)*P/2`. Each flank is 30 degrees from
the radial direction, giving a 60-degree included angle. The external profile
has radial depth `17H/24`; the internal cutting profile has depth `5H/8`.
Both helical solids have axial crest width `P/8`, so the internal material
crest is `P/4` before fabrication allowances. The flat-root construction
does not implement the UNR rounded-root variant.

## Parameters and receiving geometry

The definition separates nominal diameter and pitch from radial and axial
fabrication allowances. Positive radial allowance shrinks an external thread
and enlarges an internal cut. Axial allowance reduces external tooth width
or enlarges the internal cutting profile. Nominal pitch diameter remains a
basic-profile measurement independent of fit targets and these allowances.
Resolved actual pitch diameter describes the resulting profile after both.

For the supported Unified fits, the resolver chooses the midpoint of the
published pitch-diameter interval as an engineering target. This choice is
not a prescribed gage result. It derives the major diameter from that target
and the profile flanks, then applies fabrication allowances and checks every
applicable published diameter bound. External major and pitch diameters have
two-sided limits; the external minor column provides reference information
only. Internal minor and pitch diameters have two-sided limits;
internal major diameter has only its published minimum. Unsupported series,
size, class, mode, start count, length or allowance fails canonical preparation
without a partial commit.

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
The internal annulus and nested hex-nut wall checks use the fitted cutter
radius, which can exceed the nominal radius. The result must remain one
connected solid.

The inspector and MCP retain the same saved family and fit through previews,
edits, history and project persistence. Metric and Custom definitions keep
schema 1 and their existing nominal-basic fit behavior. Saved UNC/UNF source
additionally requires `model.thread.unified.1`, including nested hex-nut
recipes and suppressed features. A reader missing that capability refuses
the file before treating it as an older metric/custom-only document.

## Native construction and output

The [native adapter](../../crates/multicad-kernel/src/threads.cpp) sweeps a
profile along a cylindrical or conical helix, joins each ridge to the core,
and clips the joined result to flat ends. Both hands use the same construction;
the kernel's multiple-start support remains restricted to the families that
allow it. Flat cut ends are the only supported runout;
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

The test sources cover
[kernel pitch, handedness, starts, taper and STEP readback](../../crates/multicad-kernel/tests/threads.rs),
[canonical transactions and persistence](../../crates/multicad-core/tests/model_threads.rs),
[Unified limits, fit resolution and reader capabilities](../../crates/multicad-core/tests/model_unified_threads.rs),
[receiving solids, Unified pitch sections, mating clearance and manufacturing output](../../crates/multicad-model/tests/threads.rs),
[native inspector preview/history and independent-session reopen](../../crates/multicad-ui/src/modeling/controller/thread_tests.rs),
and [MCP family/fit control and output](../../crates/multicad-mcp/tests/threads.rs).
Verification results belong in the corresponding verification report. Test
definitions alone establish neither a passing native run nor OS-window
interaction, physical fit or standardized gage acceptance.
