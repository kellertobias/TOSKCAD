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
| BSW | Single-start rounded 55-degree profile; 1/4-20 and 5/16-18 | BS 84:2007 uncoated Medium external/internal; engagement at most 15 pitches |
| BSF | Single-start rounded 55-degree profile; 1/4-26 and 5/16-22 | BS 84:2007 uncoated Medium external/internal; engagement at most 15 pitches |
| BSPP | Single-start rounded 55-degree parallel profile; G1/8-28 and G1/4-19 | Elesa+Ganter 2018 external Class B/internal limits; NBK JIS B 0202:1999 basic form; sealing outside the thread |
| Legacy ambiguous Whitworth | Unavailable | Select BSW or BSF explicitly; `whitworth` remains unsupported |
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

The [rounded registry](../../crates/multicad-core/src/model/threads/whitworth.rs)
keeps BSW, BSF and BSPP separate. BSW/BSF use BS 84:2007 Figure 1 and the
uncoated Medium columns in Tables 3/4 and 9/10. The
[BSI record](https://knowledge.bsigroup.com/products/parallel-screw-threads-of-whitworth-form-requirements)
dates publication to 30 November 2007. The consulted
[BSI-authored copy](https://ssmalloys.com/wp-content/uploads/2024/11/BS-84-2007.pdf)
is manufacturer-hosted; the PDF is not redistributed. Figure 1 is printed
page 9; the four limits tables are printed pages 17, 18, 23 and 24.

For half-angle `a = 27.5°`, the circular basic form uses
`H = P/(2*tan(a))`, depth `h = 2H/3`, and crest/root radius
`r = P*cos(a)/(12*(1-sin(a)))`. Printed decimal coefficients and rounded table
diameters remain distinct from this analytic construction. The native profile
has real tangent arcs at both crest and root, with zero flat crest width.

BSPP uses [NBK's basic-form excerpt](https://www.nbk1560.com/images/en-US/product/technical_data/heikounezi_NBK/heikounezi_NBK_1.pdf)
and [Elesa+Ganter Technical Data](https://www.elesa-ganter.com/siteassets/technical/Technical_Data_General_EN.pdf),
copyright 2018, section 10.6 (full PDF page 21, footer A21; a standalone copy
has footer A20). The table names DIN 228 / UNI-ISO 228/1 without a normative
edition. G1/8 and G1/4 mean basic physical diameters 9.728 and 13.157 mm,
not those fractions of an inch. The selected manufacturer limits do not
establish full ISO compliance. The
[ISO 228-1:2000 abstract](https://committee.iso.org/standard/33777.html?browse=tc)
identifies parallel fastening threads with any pressure seal outside the
thread. No pressure-sealing result is claimed.

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

Rounded fits preserve the complete circular form. The resolver intersects
the published pitch-diameter range with the major range shifted by `-h`
and minor range shifted by `+h`, then selects the feasible midpoint.
It derives major diameter as `PD+h` and minor diameter as `PD-h`.
After radial allowance, every applicable bound is checked again. This target
is an engineering choice within the recorded intervals, not a gage result.
The supported rounded families require explicit external/internal classes,
one start and zero axial allowance; unsupported combinations refuse preparation.
BSW/BSF are bounded to 15 pitches. BSPP's table supplies no engagement bound,
so its resolved field is null; native construction limits still apply.

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
Rounded source similarly requires `model.thread.rounded.1`, including
suppressed features, imported templates, nested mechanical recipes and retained
literal references after a family edit. Existing metric, Custom and Unified
capability inventories and serialized values remain unchanged.

## Native construction and output

The [native adapter](../../crates/multicad-kernel/src/threads.cpp) sweeps a
profile along a cylindrical or conical helix, joins each ridge to the core,
and clips the joined result to flat ends. Both hands use the same construction;
the kernel's multiple-start support remains restricted to the families that
allow it. Flat cut ends are the only supported runout;
automatic relief grooves and chamfers are unimplemented.

The rounded builder sweeps a circular crest with tangent flanks and two
separate root-arc shoulders. Narrow profiles avoid full-period sweep
self-contact; buried overlap joins them to the core. Its spine uses bounded
half-turn edges as the primary representation, preserving the requested
physical phase and exact fractional endpoint. Boolean ancestry retains
the two root arcs, crest and flanks. Rounded-only material checks verify the
circles across the height; legacy flat-profile construction remains separate.

Rounded sweeps have trimmed, doubly-curved B-spline faces. Their bounds use
native directional support queries against six exterior planes, with finite
witness and distance checks. Unchanged shapes share a synchronized bounds
cache; geometry operations create a fresh cache. Analytic and ruled faces
retain the existing bounds path. Proper rigid transforms retain OCCT location
representations rather than copying the underlying geometry and trimming
curves, preserving the receiver geometry through STEP export.

The legacy flat-profile spine first uses the established continuous helix
representation. Native
crest/gap checks sample every quarter-pitch band across its height. If those
checks detect lost ridge material, or typed native volume integration fails
its convergence guard, construction retries with per-turn edges
and an exact final fractional endpoint. This bounds the swept faces for short,
non-integral thread lengths while retaining existing metric representation
when it passes. Pitch, phase and authored length stay unchanged; validity,
solid-count, physical, volume-convergence and complete-face-ancestry checks apply to both paths.
Other construction failures retain their original refusal behavior.

The [kernel input policy](../../crates/multicad-kernel/src/threads.rs) bounds
construction to 1-8 starts, pitch of at least 0.001 mm, and length of at least
one pitch. It limits `length/pitch + 2*starts` to 128.
Profiles also need a positive root, separated adjacent starts and resolvable
crest width. Flank angles must lie within 1-80 degrees and absolute radius
taper within 0.1.
The separate rounded input instead requires the validated tangent 55-degree
form, single start and zero taper; a zero flat crest width is intentional.

Modeled threads contain helical native faces. Cosmetic envelopes retain
thread intent but produce only a major external envelope or minor internal
bore. A cosmetic thread in a selected producer's dependency history makes
STEP, mesh, planar manufacturing-outline and Print output unsupported.
Switching the source to Modeled rebuilds manufacturing geometry.

The test sources cover
[kernel pitch, handedness, starts, taper and STEP readback](../../crates/multicad-kernel/tests/threads.rs),
[canonical transactions and persistence](../../crates/multicad-core/tests/model_threads.rs),
[Unified limits, fit resolution and reader capabilities](../../crates/multicad-core/tests/model_unified_threads.rs),
[rounded source limits and compatibility](../../crates/multicad-core/tests/model_rounded_threads.rs),
[native rounded crest/root sections](../../crates/multicad-kernel/tests/rounded_threads.rs),
[rounded fit pairs and STEP/print sections](../../crates/multicad-model/tests/whitworth_threads.rs),
[receiving solids, Unified pitch sections, mating clearance and manufacturing output](../../crates/multicad-model/tests/threads.rs),
[native inspector preview/history and independent-session reopen](../../crates/multicad-ui/src/modeling/controller/thread_tests.rs),
and [MCP family/fit control and output](../../crates/multicad-mcp/tests/threads.rs).
Verification results belong in the corresponding verification report. Test
definitions alone establish neither a passing native run nor OS-window
interaction, physical fit or standardized gage acceptance.
