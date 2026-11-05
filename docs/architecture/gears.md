# Parametric gears

<!-- Kind: implementation reference. Audience: operators and maintainers. Purpose: identify supported gear geometry, source ownership and verification limits. Non-goals: strength, fit or standards certification; generated tools/list owns MCP argument schemas. Comparable: threads.md; indexed from README.md. -->

The Mechanical Gear inspector and [MCP gear tools](mcp-server.md#gear-control)
prepare the same canonical transactions. A single gear stores a
[`model.gear` recipe](../../crates/multicad-core/src/model/gears/mod.rs).
A pair retains two independent recipes and a
[`model.gear_pair` definition](../../crates/multicad-core/src/model/gears/pair.rs).
[CAD-199](https://plans.tokenet.de/CAD/199) owns the complete generator outcome;
the external foundation does not complete its remaining families or parameters.

## Coverage

The [profile registry](../../crates/multicad-core/src/model/gears/profiles.rs)
owns family availability. Unsupported families are disabled in the native
inspector and refused by canonical preparation and MCP.

| Family | Single gear | Compatible pair | Current limits or remaining work |
| --- | --- | --- | --- |
| External spur | Native involute solid | Parallel axes, derived center and phase | Unshifted; through bore |
| External helical | Native twisted involute solid | Matching helix magnitude, opposite hands | Unshifted; through bore |
| Herringbone | Continuous center, opposite-handed tooth bands | Matching external family and width | Hand names the lower band; no center groove |
| Double-helical | Two tooth bands separated by a root-core center gap | Matching external family, width and gap | Distinct from continuous-center herringbone |
| Internal ring | Native inward involute teeth and connected outer rim | 24-tooth spur pinion with 48/60-tooth ring; either member order | Unshifted 20-degree spur subset; explicit outside diameter |
| Rack | Native straight flanks, circular root transitions and connected backing | One matching unshifted spur pinion; either member order | 4-64 whole centered pitch cells, zero helix, conservative finite travel |
| Bevel | Unavailable | Unavailable | [CAD-229](https://plans.tokenet.de/CAD/229) |
| Worm and wheel | Unavailable | Unavailable | [CAD-230](https://plans.tokenet.de/CAD/230) |

Intrinsic hubs, nonzero profile shift and explicit diametral-pitch input remain
[CAD-231](https://plans.tokenet.de/CAD/231). Existing recipes use normal or
transverse module and pressure angle. They retain tooth count, face width,
helix magnitude/hand, bore, center gap, circular-root-fillet ratio, minimum
root wall and two separate tangential thinning inputs. Design backlash applies
per gear at its reference circle; fabrication allowance adds further thinning.
Neither is a strength, standardized fit or printer calibration setting.

The canonical policy bounds module to at least 0.01 mm, rotary count to 4–128 whole
teeth (rack count is 4-64 cells), the selected pressure angle to 14.5–35 degrees and helix magnitude to
0–45 degrees. Additional resolved geometry checks can refuse a recipe within
those input ranges. In particular, the conservative unshifted rotary gear/pinion tooth-count
policy uses `ceil(2 / sin(normal_pressure_angle)^2)` without virtual-tooth
credit for a helix. This is the supported envelope, not a universal cutting or
undercut theorem. Root fillets are circular arcs rather than cutter trochoids.

The dimensional derivation follows the normal/transverse conventions in
[KHK's gear calculation reference](https://khkgears.net/new/gear_knowledge/gear_technical_reference/calculation_gear_dimensions.html)
and [Basic Guide B](https://khkgears.us/media/fqrlut1m/abcs-of-gearing-b.pdf).
For the transverse unshifted system, addendum is the transverse module; the
normal system uses normal module. The registry records source references.
No accuracy grade, current-standard compliance, load or lifetime rating is
implied by these geometric conventions.

## Native geometry and paired placement

The [native adapter](../../crates/multicad-kernel/src/gears.cpp) constructs a
C1 cubic Hermite approximation to analytic involutes under a conservative
linear-tolerance error bound, with native circular tip/root arcs and fillets.
Helical faces follow a continuous auxiliary-spine sweep. Continuous herringbone
and gapped double-helical construction retain distinct center geometry.
Native tooth/space section checks and face ancestry accompany the result.
Display tessellation is derived from this B-rep.

[ADR-0025](0025-derived-gear-pair-anchors.md) records pair ownership. Derived
local/mating anchors feed existing Fixed relationships; they follow current
recipe dimensions and frames. Pair creation atomically adds its two definition
Parts, reusable Assembly, internal relationships and one ordinary outer
occurrence. Definition Parts do not become extra physical outputs. Normal
component Move/Copy controls outer placement; copies share the pair definition's
internal drive coordinate.

The native pair guard accepts matching external families or the bounded
internal-ring/spur or finite rack/spur combinations described below. It requires matching transverse module and
pressure angle, face width, gap and applicable helix magnitude/opposite hands.
The Part heads must remain the gear generators. It checks actual B-reps at
nine phases over one pinion tooth cycle and at the current pose. Completed cycle
checks have a bounded worker-owned cache keyed by actual geometry inputs,
frames, identities and policy. Current pose is always checked; a partial or
cancelled cycle is never reusable. These finite samples do not establish
continuous collision-free motion or manufacturing fit.

## Internal ring subset

The [canonical source](../../crates/multicad-core/src/model/gears/mod.rs)
supports 48- and 60-tooth unshifted 20-degree spur rings. Normal/transverse
module coincide at zero helix. For module `m` and ring count `z`, pitch diameter
is `mz`, inward tip diameter `m(z-2)` and outward root diameter `m(z+2.5)`.
`outer_diameter` is an explicit independent dimension; radial material beyond
the outward roots must meet `minimum_wall`. `bore_diameter` must be zero because
the inward toothed opening is derived. External recipes omit the optional
outside field and retain their previous serialization.

The dimensional basis is KHK section 4.2, Tables 4.6/4.7 and equations
4.4–4.12, corroborated by Brian Dengel's KHK-USA article
[Internal ring gears, December 2019, printed pages 28–30](https://khkgears.us/media/beqhhrye/gear-solutions_khk_internal-ring-gears-design-and-considerations.pdf).
The live table's external-root plus sign contradicts its example; the dated
article prints the consistent subtraction. An ambiguous operator in the live
trochoid equation is not implemented. The supported pair uses exactly a
24-tooth unshifted 20-degree spur pinion. Canonical checks evaluate KHK's
involute and trimming inequalities and its explicit standard-20-degree
tooth-count difference greater than nine. Other ring/pinion counts, pressure
angles, profile shifts and internal helices remain unavailable. These bounds
are an implementation subset, not a published KHK product range or cutter
accessibility certificate.

Internal material tooth half-width grows with radius as
`physical_thickness/(2*pitch_radius) - inv(alpha) + inv(acos(base_radius/r))`,
where `inv(a)=tan(a)-a`. Backlash and fabrication allowance independently
reduce material thickness. The native adapter constructs the complementary
void with a custom circular fillet at the outward material root; it does not
subtract an unchanged external gear or simulate a generated cutter trochoid.
Phase zero denotes a material tooth on +X for both families. Internal pair
center distance is `m*(z_ring-z_pinion)/2`, with parallel axes and same-direction
rotation at the tooth-count ratio. The first member owns the drive coordinate;
either ring/pinion order is supported. Existing assembly identities and typed
transactions own both recipes and placements.

`model.gear.internal.1` gates saved ring source, including suppressed features
and template-only reusable symbols. Reader refusal and independent project
reopen are covered by the [core tests](../../crates/multicad-core/tests/model_internal_gears.rs).
The [kernel matrix](../../crates/multicad-kernel/tests/internal_gears.rs) checks
actual inward sections, custom circles, rim, ancestry, independent analytic
volume and STEP. The [model matrix](../../crates/multicad-model/tests/internal_gears.rs)
checks both sizes/orders, known wrong-phase overlap, native sampled clearance,
associative edits, history and saved reopen. Samples do not prove continuous
collision-free motion, physical fit, load capacity or manufacturing conformity.

## Finite straight rack subset

The [rack recipe](../../crates/multicad-core/src/model/gears/rack.rs) stores
an optional `rack.backing_thickness` alongside the shared module, pressure,
width, count and thinning dimensions. Rotary recipes omit this field. A rack
has no bore, outside diameter, helix, hand, center gap or profile shift.
Its 4-64 tooth count denotes whole centered pitch cells: length is `L=N*pi*m`,
tooth centers are `(i+0.5-N/2)*pi*m`, pitch line is local Y=0 and width occupies
Z=0 through the authored face width. Ends lie at X=±L/2. The count bound retains
complete per-tooth native ancestry within the existing provenance budget.

[KHK Basic Guide B](https://khkgears.us/media/fqrlut1m/abcs-of-gearing-b.pdf),
printed pages 6-7 and 14-15, supplies straight rack geometry and the full-depth
convention. The [KG standard spur table](https://www.kggear.co.jp/en/wp-content/themes/bizvektor-global-edition/pdf/9.1_Calculation-for-Standard-spur-gear_TechnicalData_KGSTOCKGEARS.pdf),
printed page 107, corroborates pitch `p=pi*m`, nominal thickness `p/2` and
addendum `ha=m`. The implementation chooses dedendum `hf=1.25*m`.
Physical pitch-line thickness is `t=p/2-backlash-fabrication_allowance`,
with both reductions authored per member. At height y, the right flank relative
to a tooth center is `x=t/2-y*tan(alpha)`; the left flank is its mirror.

Root transitions are custom tangent circles, not simulated cutter trochoids.
For fillet radius f, the right circle center is
`yc=-hf+f`, `xc=t/2+f/cos(alpha)-yc*tan(alpha)`.
Its flank tangent is `(xc-f*cos(alpha),yc-f*sin(alpha))` and root tangent is
`(xc,-hf)`. Positive tip width, surviving straight flank, root/end flats and
connected backing are required. The [native adapter](../../crates/multicad-kernel/src/rack.rs)
extrudes exact line/circular edges; the backing bottom is Y=`-hf-backing`.

A pair contains exactly one rack and one matching unshifted spur pinion.
The pinion must meet the existing conservative unshifted count policy. Module,
pressure and width must match. For pitch radius rp and authored drive theta,
the rack moves `s=rp*theta` along the pair frame's positive X while the pinion
rotates positively about Z at center `(0,rp,0)`. Its zero phase places a gap
above an odd-count rack's central tooth and a tooth above an even-count central
valley. Existing derived assembly anchors cancel each member's local frame;
source edits recompute pose without saved copied transforms.

The conservative travel envelope is `|s|<=L/2-pinion_tip_radius`. It keeps the
full pinion tip-circle projection inside the rack ends and requires room for
one whole pitch cycle. It is not maximum theoretical engagement travel.
Every resolve checks it, including source and drive edits. Native evaluation
checks nine phases over a pinion tooth period, both finite endpoints and the
current pose. No-overlap and actual native proximity within the sum of authored
thinning provide sampled engagement evidence. They do not certify continuous
motion, tooth contact, load capacity or physical fit.

`model.gear.rack.1` gates direct, suppressed and template-only saved source.
The [core tests](../../crates/multicad-core/tests/model_rack_gears.rs),
[kernel section/STEP matrix](../../crates/multicad-kernel/tests/straight_rack.rs)
and [native pair/export tests](../../crates/multicad-model/tests/rack_gears.rs)
own the corresponding executable checks.

## Editing and verification boundaries

Choose **Gear → Create mating pair** for creation. Select the pair occurrence,
Assembly, either member or its recipe and choose **Edit gear pair** to reopen
it. By default, editing changes only drive and layout. **Update both gear
recipes** explicitly enables common geometry controls and independent wheel
count, bore and thinning. Preview validates the complete atomic candidate;
Cancel preserves source and Apply commits the matching native result.

The native tests cover
[kernel sections and exports](../../crates/multicad-kernel/tests/gears.rs),
[canonical recipes](../../crates/multicad-core/tests/model_gears.rs),
[pair transactions](../../crates/multicad-core/tests/model_gear_pairs.rs),
[native pair placements and failure controls](../../crates/multicad-model/tests/gear_pairs.rs),
and [MCP control](../../crates/multicad-mcp/tests/gear_pairs.rs).
Actual macOS interaction and artifact readbacks are recorded in CAD-199's
Working Log; they are separate from tests and Windows/Linux runtime evidence.

The first herringbone pair preview remains expensive in the development build.
Separate Preview, canonical display and export lanes can repeat cycle work;
[CAD-232](https://plans.tokenet.de/CAD/232) tracks bounded cross-lane reuse and
operator-visible performance verification. No near-instant pair-generation
claim follows from cache unit tests.
