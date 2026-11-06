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
| ISO metric fitted subset | M3x0.5 and M6x1; single-start 60-degree profile, circular external root, flat internal cutter | BIS IS 14962 Parts 1/2:2001, identical ISO 965-1/2:1998; uncoated 6g external / 6H internal, bounded normal engagement |
| Custom | Straight or tapered axial profile with explicit depth, crest width and two flank angles | User-defined dimensions; no standard designation or tolerance class |
| Unified UNC | Single-start, uncoated UN flat-root profile; 1/4-20 and 5/16-18 | Historical H28 (1969) limits; 2A external and 2B internal; threaded length at most 1.5 nominal diameters |
| Unified UNF | Single-start, uncoated UN flat-root profile; 1/4-28 and 5/16-24 | Historical H28 (1969) limits; 2A external and 2B internal; threaded length at most 1.5 nominal diameters |
| Legacy ambiguous Unified | Unavailable | Select UNC or UNF explicitly; `unified` remains unsupported |
| BSW | Single-start rounded 55-degree profile; 1/4-20 and 5/16-18 | BS 84:2007 uncoated Medium external/internal; engagement at most 15 pitches |
| BSF | Single-start rounded 55-degree profile; 1/4-26 and 5/16-22 | BS 84:2007 uncoated Medium external/internal; engagement at most 15 pitches |
| BSPP | Single-start rounded 55-degree parallel profile; G1/8-28 and G1/4-19 | Elesa+Ganter 2018 external Class B/internal limits; NBK JIS B 0202:1999 basic form; sealing outside the thread |
| Legacy ambiguous Whitworth | Unavailable | Select BSW or BSF explicitly; `whitworth` remains unsupported |
| BSPT | Right-hand, single-start R external and Rc internal tapered rounded 55-degree form; 1/8-28 and 1/4-19 | ISO 7-1:1994 with Cor 1:2007; nominal gauge convention and bounded position tolerance; Rp unavailable |
| NPT | Right-hand, single-start external/internal 60-degree axial form; 1/8-27 and 1/4-18 | NBS H28 (1957) Part II, 1959 printing; plain-face nominal gauge, historical one-turn tolerance, explicit midpoint truncation; no NPTF or vanish/runout |
| Trapezoidal | Parallel 30-degree nominal sharp design profile; Tr10x2 and Tr12x3 | ISO 2901:2016 and ISO 2904:2020 basic/design dimensions; source crest/root clearance, no ISO 2903 tolerance class |
| Acme | Parallel 29-degree nominal sharp profile; 3/8-12 and 1/2-10 | NBS H28 (1957) Part III, issued 1960; basic form plus general-purpose crest/root clearance, no G-class fit |
| Buttress | Parallel ST20x4 nominal profile; 3/30-degree flanks, circular external root and flat internal cutter; RH/LH, single start | BIS IS 4696 Parts 1/2/3:2004; explicit steep-flank axial side, source relief clearance, no 7e/7H fit class |
| Round | Parallel 30-degree nominal Rd10x1/10 and Rd20x1/8; true circular crests/roots with distinct internal radii; RH/LH, single start | Mitsubishi Materials C010B G009 profile and BAER 2022 named sizes; source radial crest clearance, no DIN 405-2 fitted class |

The metric data comes from the
[NBK metric-thread technical reference](https://www.nbk1560.com/images/en-US/product/technical_data/meternezi_NBK/meternezi_NBK_1.pdf),
consulted on 2026-11-01. For pitch `P`, the implemented radial depth is
`5*sqrt(3)*P/16`, the crest width is `P/8`, and each flank angle is 30 degrees.
The registry contains only explicitly listed diameter/pitch pairs.
It includes nominal diameters 2, 2.5, 3, 3.5, 4, 4.5, 5, 6, 7, 8, 9 and 10 mm,
plus selected larger sizes through 30 mm.

The unchanged Nominal basic selection does not implement ISO 965 tolerance
classes or certify compliance with the current ISO 724 edition. The separate
[metric fitted subset](#metric-fit-classes) has explicit limits. A Custom tapered profile does
not establish NPT or BSPT compliance. Each pipe family requires its own sourced
profile, gauge-plane and tolerance rules.

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

## Metric fit classes

The [fit registry](../../crates/multicad-core/src/model/threads/metric_fit.rs)
supports `metric_6g_external` and `metric_6h_internal` for M3x0.5 and M6x1.
Its primary basis is BIS IS 14962 Part 1:2001, December 2001, and Part 2:2001,
August 2001, national adoptions identical to ISO 965-1/2:1998 without deviations.
The [Part 2 tables](https://law.resource.org/pub/in/bis/S01/is.14962.2.2001.pdf)
are printed pages 2 and 3; [Part 1](https://law.resource.org/pub/in/bis/S01/is.14962.1.2001.pdf)
clause 11, Figure 6 and Table 7 are printed pages 14 and 15. These public
research copies are not redistributed. This is historical uncoated dimensional
coverage; it does not establish current-standard, gage, strength or physical-fit
certification. Other sizes and classes remain unavailable for these selections.

| Size | Normal engagement (mm) | 6g major (mm) | 6g pitch (mm) | Printed minimum external root radius (mm) | 6H pitch (mm) | 6H minor (mm) |
| --- | --- | --- | --- | --- | --- | --- |
| M3x0.5 | >1.5 and <=4.5 | 2.874-2.980 | 2.580-2.655 | 0.063 | 2.675-2.775 | 2.459-2.599 |
| M6x1 | >3 and <=9 | 5.794-5.974 | 5.212-5.324 | 0.125 | 5.350-5.500 | 4.917-5.153 |

The tables supply no two-sided external minor or internal major interval.
Those fields remain unspecified in `fit.limits`; no stress-calculation diameter
from Part 3 is substituted as a manufacturing limit. The internal cutter's
major diameter starts at nominal `d`, and a separate basic-profile envelope
check protects the root contour.

Targets are explicit engineering choices inside the simultaneous limits.
External major and pitch diameters use their table midpoints. With
`H=sqrt(3)*P/2`, external root radius is `R=P/(4*sqrt(3))`, matching Part 1's
recommended H/6 truncation. It exceeds both `0.125P` and the rounded tabulated
minimum. For major diameter `M` and pitch diameter `Q`, the axial crest is
`W=P/2-(M-Q)/sqrt(3)`, and radial depth is `(P-W)/(2*tan(30 degrees))-R`.
This gives a true tangent circular root and `minor=Q-H+2R`. The invariant test
also checks the source minimum/maximum truncation envelope across each
supported pitch-diameter interval. Internal major is nominal; internal pitch
and minor targets are their independent midpoints, with a flat root.
The new symmetric circular root uses the native explicit meridian sweep with
quarter-turn domains and a checked surface-approximation error. The existing
asymmetric buttress representation remains separate.
The symmetric root retains its three measured unions and ancestry transfers,
with OCCT parallel work enabled inside each union through its existing shared
pool. Other thread construction keeps its existing execution policy. A single
combined union failed the analytic volume regression and is not used.

Nominal basic pitch diameter remains `d-3*sqrt(3)*P/8`. Target dimensions and
source limits stay in `resolved.metric_fit`; `resolved.fit` reports actual
diameters after fabrication allowance. Positive radial allowance shrinks
external material or enlarges the internal cutter. Axial allowance changes
tooth width; the external tangent root depth is recomputed with its fixed
radius. Every applicable class limit is checked again. An invalid allowance
fails atomically rather than changing the selected class or widening limits.

The saved fit enum uses existing canonical fields. The conditional
`model.thread.metric_fit.1` reader capability also covers suppressed features,
template-only sources, nested hardware and retained reference recipes.
Existing Nominal basic shapes and literal catalog references are unchanged.

`ThreadParameters.metric_fit_engagement` optionally separates fit context from
physical `length`. Omission preserves the legacy rule: the complete physical
profile after endpoint treatments must lie within the table's normal interval.
`{ "kind": "intended_length", "length": Dimension }` declares a mating length
within that same interval and no longer than the available complete profile.
`{ "kind": "normal_group_assumed" }` explicitly selects normal group while the
mate is unknown. Part 1 clause 12, printed page 15, recommends this group when
actual engagement is unknown, including standard bolt manufacture. This branch
reports no numeric engagement. As an engineering feasibility check, enough
complete profile must remain to permit some engagement above the table's lower
bound. All physical length, diameter, root, allowance and native geometry guards
continue to apply to the entire thread.

The conditional `model.thread.metric_fit_engagement.1` reader capability covers
both tags in active, suppressed, private, template and retained reference
sources. Legacy omitted fields do not require it. The UI exposes the three
choices; MCP thread and nested hardware recipes accept the same tagged intent.
MCP edits preserve omitted intent, and `clear_metric_fit_engagement: true`
restores legacy validation; supplying both clear and replacement is an error.
The intended length accepts the usual unit-bearing literal or bound expression.
Inspection reports physical length, complete-profile interval, engagement basis
and normal limits separately. For example, an 8 mm M3 thread with an intended
3 mm engagement still constructs 8 mm of fitted thread.

Intended or assumed fit context does not prove installed overlap. A connection
must independently check the placed complete-profile intervals, compatibility,
actual normal engagement, required strength and blind clearance. Unknown usable
insert-thread endpoints remain unknown. This source field supplies no assembled
connection acceptance or manufacturing certification.

## Custom metric endpoint geometry

Optional `end_treatments` names a start treatment at local z=0 and an end
treatment at z=authored length, independently of hand. Start is `flat_cut` or
`chamfer45 { radial_depth }`; end is `flat_cut` or
`cylindrical_relief { length, radial_depth }`. Dimensions can retain parameter
expressions. Omission preserves legacy serialization and geometry.

These are custom engineering treatments for modeled, parallel, single-start
standalone metric threads. A 45-degree entry consumes axial length equal to
its radial depth. A square cylindrical relief consumes the final authored
axial extent. External relief radius is the thread root radius minus its
radial depth; internal relief enlarges the cutter beyond its major radius.
Authored total length stays unchanged. The receipt reports the complete
central profile interval and length; at least one pitch must remain. Metric
fit engagement uses that complete interval by default or the explicit
[engagement intent](#metric-fit-classes), bounded by the available profile.

The evaluator intersects the external thread with an exact revolved mask or
unites the internal cutter with it before the receiving-body operation.
Positive shaft core, connected solids and minimum receiving wall remain
mandatory. Internal wall proof uses the largest of ordinary major radius,
minor radius plus entry depth, and major radius plus end-relief depth.
Nested screw junction and nut-mouth treatments remain explicitly unavailable.
Saved endpoint intent requires `model.thread.end_treatments.1`, including
suppressed and embedded sources.

These treatments do not claim DIN 76 shoulder relief or natural incomplete-thread
runout. Schaeffler's [Technical Pocket Guide STT](https://www.schaeffler.com/remotemedien/media/_shared_media/08_media_library/01_publications/schaeffler_2/catalogue_1/downloads_6/stt_de_en.pdf),
fourth expanded edition March 2025, printed pages 508-510, distinguishes
shoulder grooves from runout lengths. Those diagrams require shoulder context
and do not prescribe one complete helical ramp. Its historical handbook rows
also precede the [DIN 76-1:2025-10 revision](https://www.dinmedia.de/en/standard/din-76-1/394715258).
No such standard treatment is inferred from the custom terminal recess.

## Metric buttress profile

The [buttress registry](../../crates/multicad-core/src/model/threads/buttress.rs)
bounds coverage to nominal ST20x4 from BIS IS 4696:2004. The primary
[Part 1 profile](https://law.resource.org/pub/in/bis/S01/is.4696.1.2004.pdf)
is dated November 2004; printed pages 2-3 define the 3-degree pressure flank,
30-degree opposing flank and circular external root. The
[Part 2 designation](https://law.resource.org/pub/in/bis/S01/is.4696.2.2004.pdf)
is dated October 2004 and uses `ST`. The
[Part 3 dimensions](https://law.resource.org/pub/in/bis/S01/is.4696.3.2004.pdf)
are dated September 2004; printed page 3 lists the nominal diameter 20 mm,
pitch 4 mm, external/internal pitch diameters 17/17.635 mm, external/internal
minor diameters 13.058/14 mm, depth 3.471 mm and root radius 0.497 mm.
The source was aligned with historical DIN 513 sheets. Coverage does not
assert a current DIN edition, 7e/7H tolerance class, physical fit or load capacity.

Persisted `buttress` intent selects `bis_4696_nominal_2004` and the axial side
of the external tooth's 3-degree face. `negative_axis_side` gives lower/upper
flank angles 3/30 degrees; `positive_axis_side` gives 30/3 degrees. This choice
is independent of right/left hand. Internal material normals reverse after
subtraction, so the saved side is a profile direction, not an applied-force sign.
Only one start, `nominal_basic`, the listed diameter/pitch and zero axial
fabrication allowance are supported. Radial fabrication allowance is separate.

For pitch `P`, let `S = tan(3 degrees) + tan(30 degrees)` and
`w = (0.5 - 0.375*S)*P`. The external crest width is `w` and its tangent
root circle has `R = 0.12427*P`. Exact tangency gives external depth
`0.8677683831826664*P`; the source's rounded coefficients/table dimensions
remain separately visible. No sharp-root approximation substitutes for the circle.
The internal cutter is flat, has radial depth `0.75*P` and crest width `w+a`,
where the source clearance is `a = 0.1*sqrt(P)` with `P` numerically in mm.
Its material crest is `w-a`. The common basic pitch diameter is `d-0.75*P`;
the internal construction pitch diameter is `d-0.75*P+2*a/S`.

The internal cutter crest is shifted axially by `+a/2` for the negative-side
3-degree flank and `-a/2` for the positive-side flank. A local-axis rotation
of `-hand_sign*2*pi*offset/lead` implements this phase without moving the end
planes. It aligns the pressure flank and puts the source clearance on the
opposing flank. The resolved receipt exposes the signed phase, actual root
circle, nominal construction dimensions and source dimensions separately.

## Round profiles

The [Round registry](../../crates/multicad-core/src/model/threads/round.rs)
bounds coverage to Rd10x1/10 and Rd20x1/8. Nominal diameter is in millimetres;
pitch is a reciprocal inch, converted as `P = 25.4/TPI`. The
[BAER 2022 catalog](https://www.gewindebohrer.de/_Resources/Persistent/281607539e8454e0872443e825ba712c98d64b82/Gewindebohrer_Katalog_2022_DE.pdf)
prints these size/pitch designations on page 134. Its tool diameters, drill
sizes and die-blank recommendations are not normative product-limit targets.

[Mitsubishi Materials GENERAL CATALOGUE C010B / THREADING](https://data.mmc-carbide.com/4116/8197/5012/catalog_c010b_threading.pdf),
printed G009 (PDF page 10), supplies the nominal 30-degree profile, radial
crest clearance `ac = 0.05P`, equal external/internal profile depth `0.5P`
and separately rounded radius coefficients. C010B is the verified version
identifier; this chapter has no verified printed publication year. Walter's
[Special Edition 2026](https://cdn2.walter-tools.com/files/a5ea48ae-5fa6-0161-3cb3-0ac22248a0fb/adc1bdc1-e1ce-456c-8408-fd9ccf0c860f/flyer-small-part-auto-2026-en.pdf),
imprint 11/2025, page 59 independently confirms the distinct internal radii.
These manufacturer sources do not establish a complete DIN 405-2 fit class,
calibrated gage acceptance or current full DIN compliance.

For half-angle `a = 15 degrees`, the exact tangent external crest/root radius
is `Re = P*(cos(a)-sin(a))/(4*(1-sin(a)))`. The internal material radii are
`R2 = Re+delta` at the crest and `R3 = Re-delta` at the root, where
`delta = ac*sin(a)/(1-sin(a))`. They round to the C010B coefficients
`R1 = 0.238507P`, `R2 = 0.255967P` and `R3 = 0.221047P`. The construction
retains actual exact radii and these independently rounded source coefficients
separately. A positive internal cutter reverses the material terminology:
its outer crest uses `R3` and its root uses `R2`.

The source dimension arrows give external major `d`, external minor `d-P`,
common basic pitch diameter `d-0.5P`, internal construction major `d+0.1P`
and internal minor `d-0.9P`. These are derived nominal dimensions, not unseen
DIN table transcriptions. Internal pitch diameter is not the midpoint of
its construction major/minor diameters. The two profiles align their straight
flanks without an additional axial phase; source radial crest clearance is
separate from any saved fabrication allowance.

Saved intent uses the existing Round family, listed nominal diameter/pitch,
NominalBasic, hand and allowances. One start, zero taper and zero axial
fabrication allowance are supported. Both hands are checked engineering
geometry, not a claim that the cited right-cutting tool rows prescribe every
handed product. No extra authored `round` object is needed. Direct and nested
resolved receipts distinguish material and cutter radii, actual circle centres,
basic pitch diameter and construction dimensions. Physical fit, load capacity
and sealing performance remain unverified.

## Trapezoidal and Acme lead-screw profiles

The [lead-screw registry](../../crates/multicad-core/src/model/threads/lead_screw.rs)
keeps metric Trapezoidal and inch Acme as separate families. Persisted intent
uses the existing family, matched nominal diameter/pitch, `nominal_basic`,
hand, starts and fabrication allowances. No extra authored profile object is
needed for the single pinned convention in each family. Saved direct, nested,
suppressed, template and retained-reference sources require
`model.thread.lead_screw.1`. The resolved `lead_screw` receipt identifies the
source row, actual mode's basic construction dimensions, source radial
crest/root clearance, hand, start count and lead.

Metric sources are ISO 2901:2016, third edition 2016-09-15, Figure 2/Table 2
(printed pages 4-5), and ISO 2904:2020, second edition 2020-01, Table 1
(printed pages 2-3). The ISO-authored public previews contain the implemented
[profile equations](https://cdn.standards.iteh.ai/samples/68337/c87112d8e3dc487c95fd964d13101ca4/ISO-2901-2016.pdf)
and [diameter rows](https://cdn.standards.iteh.ai/samples/78729/7d158726108b478c889fa887d460ba78/ISO-2904-2020.pdf).

| Basic row | Half-angle | Radial clearance c | Basic pitch diameter | External minor | Internal minor | Internal major |
| --- | --- | --- | --- | --- | --- | --- |
| Tr10x2, mm | 15 degrees | 0.25 | 9 | 7.5 | 8 | 10.5 |
| Tr12x3, mm | 15 degrees | 0.25 | 10.5 | 8.5 | 9 | 12.5 |
| 3/8-12 Acme, inch | 14.5 degrees | 0.005 | 0.375 - 0.5/12 | 0.375 - 1/12 - 0.010 | 0.375 - 1/12 | 0.385 |
| 1/2-10 Acme, inch | 14.5 degrees | 0.010 | 0.450 | 0.380 | 0.400 | 0.520 |

Acme uses [NBS H28 (1957), Part III](https://nvlpubs.nist.gov/nistpubs/Legacy/hb/nbshandbook28supp1957pt3.pdf),
issued 7 October 1960, section XII.2 and Figure XII.1 (printed pages 1-2).
Table XII.3 supplies the diameter/TPI series. Exact inch pitch is `1/TPI`;
conversion uses 25.4 mm/in. General-purpose minimum crest/root clearance is
0.020 inch on diameter for 10 TPI and coarser, and 0.010 inch for finer pitches.
The registry uses half those values on radius. It retains exact basic-formula
dimensions rather than substituting independently rounded fitted limits.

For nominal external diameter `d`, adjacent pitch `P`, source radial clearance
`c` and half-angle `a`, both profiles have depth `P/2 + c`. The external
major/minor are `d` and `d - P - 2c`; the internal cutter major/minor are
`d + 2c` and `d - P`. Both share basic pitch diameter `d - P/2`.
At construction major diameter `M`, the cutter/tooth crest width is
`P/2 - (M - basic_pitch_diameter)*tan(a)`. Internal cutter crest width is the
nut's root flat; copying the external width would give the wrong pitch
diameter. Exact trigonometry remains distinct from Acme's printed rounded
0.3707P basic-flat coefficient. Sharp corners are the limiting design choice;
source maximum or optional corner rounding is not represented as a fixed arc.

These basic members have zero theoretical flank allowance before fabrication
adjustments. They are not an ISO 2903 fit or an Acme 2G/3G/4G product. For
example, the 1/2-10 basic external pitch diameter is 0.450 inch, exceeding
H28 Table XII.10's 2G maximum 0.4443 inch. Native mating tests use explicitly
recorded additional fabrication clearance; that is not a sourced fit class
or a free-running physical-fit result. Stub and Centralizing Acme, other rows,
fit classes and rounded/runout variants remain unavailable.

The native implementation allows 1-8 starts and either hand within its
geometry budget. The profile still uses adjacent pitch, while lead magnitude
is `pitch*starts`; hand controls its direction. Source lead/LH conventions
appear in H28 XII.6 and historical ISO 2902:1977 clause 4, available in its
[identical SIST 1995 adoption](https://cdn.standards.iteh.ai/samples/7935/31bcf5ecb73d401687b08ba20569ff17/SIST-ISO-2902-1995.pdf).
This historical notation evidence is separate from current ISO 2902:2016,
which delegates designation to ISO 2903. Multi-start coverage here is an
engineering extension of the sourced basic profile, not certification of
arbitrary standard lead/fit combinations, load, precision or physical fit.
The display reports start count, lead and RH/LH explicitly.

## NPT construction and source limits

The [NPT registry](../../crates/multicad-core/src/model/threads/npt.rs) retains
form, nominal pipe size, `nbs1959_plain_face` gauge convention and the explicit
`midpoint_cone_parallel` truncation target. Its primary source is
[NBS H28 (1957), Part II](https://nvlpubs.nist.gov/nistpubs/Legacy/hb/nbshandbook28supp1957pt2.pdf),
issued 16 November 1959. Figure VII.1 is printed page 3 (PDF page 13),
truncation Table VII.1 is printed page 4 (PDF page 14), and product Table VII.2
is printed pages 5-6 (PDF pages 15-16). The pipe-size rows are cross-checked
against [Hy-Lok Pipe Fittings, May 2016](https://english.hy-lok.com/data/PRODUCTS/FITTINGS/4-8._Pipe_Fittings_%282016%29-02.pdf),
page 38. These historical sources establish the implemented subset; they do
not establish current ASME compliance, calibrated-gage acceptance, pressure
sealing or physical fit. NPTF remains unavailable.

| Nominal size | TPI | Stock OD, inch | Gauge pitch diameter E1, inch | L1, inch | External extent L2, inch | Source basic L5, inch | Truncation f/P range |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1/8 | 27 | 0.405 | 0.37360 | 0.1615 | 0.2639 | 0.1898 | 0.033-0.096 |
| 1/4 | 18 | 0.540 | 0.49163 | 0.2278 | 0.4018 | 0.2907 | 0.033-0.088 |

Pitch is exactly `25.4/TPI` mm. The external datum is the small end, with
nominal gauge at L1 and radius slope +1/32. The internal datum is the fitting
face, with nominal gauge at zero and inward radius slope −1/32. Both pitch
cones anchor the published E1 once; derived E0/E2/E3 remain distinct from
independently rounded printed values. The historical Table VII.9 gage E0 entry
for 1/8 conflicts with the product row; construction uses Table VII.2 and
retains its raw values. It does not substitute that gage-table entry.

The axial flanks have a true 60-degree included angle, with bisector normal
to the axis. For `m = cot(30°)` and signed radius taper `k`, the compensated
slopes sum to `S = 1/(m-k) + 1/(m+k)`. Fundamental height is `H = P/S`,
slightly below the conventional `sqrt(3)*P/2` approximation. The explicit
engineering target chooses the midpoint of each source truncation range:
`h = H - 2f`, with cone-parallel crest and root flat widths `f*S`.
The native meridian construction preserves these axial angles. Legacy Custom
geometry keeps its existing construction.

External length is fixed to source L2. A real cylinder intersection clips
crests to source pipe OD, producing the imperfect crest region while retaining
roots and flanks. Raw source basic L5 and the actual target profile's stock
intersection are reported separately: midpoint truncation and radial allowance
move the actual intersection. The incomplete crests do not retain complete
profile truncation limits. The L2-to-L4 root vanish/tool withdrawal is omitted;
other external lengths are refused until that construction exists.

Internal depth must be at least `L1 + 3P + 32*radial_clearance`. The three
pitches are the source wrench allowance; the receiving-depth convention is
an explicit engineering choice. The default `L1 + 4P` includes one pitch of
positive gauge travel. Both forms allow nonnegative radial clearance through
`P/32`, covering the supported half of the historical bilateral one-turn gauge
range. Axial allowance is zero; left hand, multiple starts and unlisted sizes
are refused before commit. `model.thread.npt.1` is required for saved direct,
nested hardware, suppressed and embedded or retained reference definitions.

## Tapered pipe construction

The [pipe registry](../../crates/multicad-core/src/model/threads/pipe.rs)
retains explicit R/Rc form, nominal pipe size and `iso7_nominal1994` gauge intent.
It uses [ISO 7-1:1994, third edition](https://www.iso.org/standard/20819.html)
with [Cor 1:2007](https://www.iso.org/standard/46479.html).
The [ISO-authored preview](https://cdn.standards.iteh.ai/samples/20819/7860e4c71b9647f68baf76cd62d6e320/ISO-7-1-1994.pdf)
contains Table 1 and length rules but omits the profile page.
The identical [BIS IS 554:1999 adoption](https://law.resource.org/pub/in/bis/S01/is.554.1999.pdf)
supplies Figures 1-2 and clauses 7.1.1-7.1.2 on printed page 4.
The [corrigendum copy](https://www.evs.ee/en/StandardDownload/DownloadPdf?language=EnglishLanguage&productId=74799)
supplies corrected Figure 5. Source PDFs are not redistributed.

| Pipe size | TPI | Gauge major/pitch/minor (mm, printed) | R nominal gauge offset (mm) | R nominal minimum useful length (mm) | Rc complete receiving minimum (mm) |
| --- | --- | --- | --- | --- | --- |
| 1/8 | 28 | 9.728 / 9.147 / 8.566 | 4.0 | 6.5 | 7.4 |
| 1/4 | 19 | 13.157 / 12.301 / 11.445 | 6.0 | 9.7 | 11.0 |

R uses its small-end reference with radius slope `+1/32` along the local axis.
Rc uses its fitting-face reference with slope `-1/32` and a gauge plane half
a pitch inward, following corrected Figure 5. Diameter taper is 1:16.
The saved `major_diameter` remains the basic diameter at the local base:
`baseD = gaugeD - 2*slope*gaugeOffset`. Resolved pipe metadata separately
reports construction diameters at the nominal gauge plane and the shifted
actual gauge offset. It retains the rounded printed source values separately.
Pitch uses exact `25.4/TPI`; pipe labels are nominal sizes.

For half-angle `a = 27.5 degrees`, let `m = cot(a)` and signed slope `k`.
The tapered fundamental height is `H = P*(m*m-k*k)/(2*m)`.
Depth remains `h = P/(3*tan(a))`, with circle radius
`r = (H-h)/(2*(csc(a)-sqrt(1+k*k)))`.
These analytic coefficients round to the source's `0.960237`, `0.640327`
and `0.137278`. Root centers are offset by `H/(m+k)` from the previous crest
center. Their asymmetric spacing preserves true circular tangency and axial
55-degree flanks. The tapered builder uses a radial/axial meridian frame along
the conical helix, with finite native surface-approximation error bounded by
the requested linear tolerance.

The supported fit is `bspt_nominal`. Mandatory gauge-position tolerances are
plus/minus one turn for R and 1.25 turns for Rc; printed millimetre conversions
are informative rounded values. Radial allowance shifts the intended gauge
position and must remain inside that tolerance. An R thread must also retain
at least 2.75 pitches of fitting allowance beyond its shifted gauge plane.
Complete Rc receiving length uses Table 1 column 16 and clause 7.2.2.
Incomplete/free runout, washout, Rp, other sizes, left-hand source designation
and axial allowance are unavailable. Suitable jointing medium and materials
are required by the source; CAD geometry does not certify pressure sealing,
calibrated-gage acceptance or physical fit.

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
Saved BSPT additionally requires `model.thread.bspt.1`, including suppressed
features, imported templates, nested hardware and retained literal references.
Older source omits the optional pipe intent field.
Saved Buttress requires `model.thread.buttress.1`, including suppressed and
template-only features, nested hardware and retained literal references.
Older source omits the optional buttress intent field.
Saved Round requires `model.thread.round.1`, including suppressed and
template-only features, nested hardware and retained reference geometry.
The receipt is resolved data; it introduces no additional saved intent object.

## Native construction and output

The [native adapter](../../crates/multicad-kernel/src/threads.cpp) sweeps a
profile along a cylindrical or conical helix, joins each ridge to the core,
and clips the joined result to flat ends. Both hands use the same construction;
the kernel's multiple-start support remains restricted to the families that
allow it. Flat cut ends remain the default. Standalone modeled metric threads
can use the explicit [custom endpoint treatments](#custom-metric-endpoint-geometry).
Natural helical runout and DIN 76 shoulder relief remain unavailable.

The rounded builder sweeps a circular crest with tangent flanks and two
separate root-arc shoulders. Narrow profiles avoid full-period sweep
self-contact; buried overlap joins them to the core. Its spine uses bounded
half-turn parallel or quarter-turn tapered edges, preserving the requested
physical phase and exact fractional endpoint. Boolean ancestry retains
the two root arcs, crest and flanks. Rounded-only material checks verify the
circles across the height; legacy flat-profile construction remains separate.

The buttress external builder adds two tangent root-circle sectors to the
flat asymmetric main ridge. Quarter-turn spine domains preserve valid
trimmed face wires. Overlap is buried, and exposed closure faces remain
unnameable and refused. The internal cutter uses the separate flat profile.
Both source flank angles, source root radius and the exact end planes survive
these representation boundaries.

The separate knuckle input keeps the 30-degree source form independent of
the 55-degree rounded adapter. Internal cutter crest/root circles have their
own radii. Two exact 75-degree arcs retain the same tangent crest circle,
with bounded face domains for native intersection. Root shoulders remain
separate exact arcs. Quarter-turn spine domains preserve authored phase,
profile dimensions and exact fractional ends; ordinary validity, physical,
volume and ancestry guards apply.

Display tessellation operates on a detached native copy. If any face lacks
triangles, it retries at one quarter of the linear deflection and half the
angular deflection, for at most three attempts including the original.
Every exact face must contribute triangles, and the original triangle/index
budget still applies. Refinement does not alter manufacturing geometry.
A complete finer result can satisfy the original maximum-deflection request.

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
form, single start and either zero or signed 1:16 diameter taper;
a zero flat crest width is intentional.

Modeled threads contain helical native faces. Cosmetic envelopes retain
thread intent but produce only a major external envelope or minor internal
bore. A cosmetic thread in a selected producer's dependency history makes
STEP, mesh, planar manufacturing-outline and Print output unsupported.
Switching the source to Modeled rebuilds manufacturing geometry.

A thread may bind its representation to an existing Count parameter through
`representation_parameter`. That input has sole authority: exact 0 selects
Cosmetic envelope and exact 1 selects Modeled helix. The optional binding is
retained by standalone and nested hardware source, captured as a dependency and
remapped into pinned instances. Library configuration presents named choices
with independent overrides and reset behavior. Every manufacturing-history
guard resolves the effective receiving value, including private instance
parameters. Invalid bindings fail without using the dormant literal.
[ADR-0037](0037-parameterized-thread-representation.md) owns the reader capability
and semantic update rules; unbound legacy source keeps its existing wire form.

The test sources cover
[kernel pitch, handedness, starts, taper and STEP readback](../../crates/multicad-kernel/tests/threads.rs),
[canonical transactions and persistence](../../crates/multicad-core/tests/model_threads.rs),
[Unified limits, fit resolution and reader capabilities](../../crates/multicad-core/tests/model_unified_threads.rs),
[rounded source limits and compatibility](../../crates/multicad-core/tests/model_rounded_threads.rs),
[buttress source and reader compatibility](../../crates/multicad-core/tests/model_buttress_threads.rs),
[native asymmetric circular roots](../../crates/multicad-kernel/tests/asymmetric_root_threads.rs),
[Round source dimensions and reader gates](../../crates/multicad-core/tests/model_round_threads.rs),
[native knuckle circles and small-size material regression](../../crates/multicad-kernel/tests/knuckle_threads.rs),
[Round mating pairs and actual manufacturing sections](../../crates/multicad-model/tests/round_threads.rs),
[buttress mating, phase and manufacturing exports](../../crates/multicad-model/tests/buttress_threads.rs),
[native rounded crest/root sections](../../crates/multicad-kernel/tests/rounded_threads.rs),
[pipe source, gauge rules and reader compatibility](../../crates/multicad-core/tests/model_pipe_threads.rs),
[tapered circular sections](../../crates/multicad-kernel/tests/tapered_rounded_threads.rs),
[native R/Rc pairs and manufacturing exports](../../crates/multicad-model/tests/pipe_threads.rs),
[rounded fit pairs and STEP/print sections](../../crates/multicad-model/tests/whitworth_threads.rs),
[receiving solids, Unified pitch sections, mating clearance and manufacturing output](../../crates/multicad-model/tests/threads.rs),
[native inspector preview/history and independent-session reopen](../../crates/multicad-ui/src/modeling/controller/thread_tests.rs),
and [MCP family/fit control and output](../../crates/multicad-mcp/tests/threads.rs).
Verification results belong in the corresponding verification report. Test
definitions alone establish neither a passing native run nor OS-window
interaction, physical fit or standardized gage acceptance.
