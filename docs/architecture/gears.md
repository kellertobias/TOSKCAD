# Parametric gears

<!-- Kind: implementation reference. Audience: operators and maintainers. Purpose: identify supported gear geometry, source ownership and verification limits. Non-goals: strength, fit or standards certification; generated tools/list owns MCP argument schemas. Comparable: threads.md; indexed from README.md. -->

The Mechanical Gear inspector and [MCP gear tools](mcp-server.md#gear-control)
prepare the same canonical transactions. A single gear stores a
[`model.gear` recipe](../../crates/multicad-core/src/model/gears/mod.rs).
A pair retains two independent recipes and a
[`model.gear_pair` definition](../../crates/multicad-core/src/model/gears/pair.rs).
[CAD-199](https://plans.tokenet.de/CAD/199) owns the complete generator outcome
and its acceptance across the implemented families and parameter coverage.

## Coverage

The [profile registry](../../crates/multicad-core/src/model/gears/profiles.rs)
owns family availability. Unsupported families are disabled in the native
inspector and refused by canonical preparation and MCP.

| Family | Single gear | Compatible pair | Current limits or remaining work |
| --- | --- | --- | --- |
| External spur | Native involute solid, optional hub and through bore | Parallel axes, derived center and phase | Module or DP; bounded shifted profiles and retained matched tips |
| External helical | Native twisted involute solid, optional hub and through bore | Matching helix magnitude, opposite hands | Module or DP; retained normal/transverse shift context |
| Herringbone | Continuous center, opposite-handed tooth bands; optional hub and through bore | Matching external family and width | Module or DP; retained shift; hand names the lower band |
| Double-helical | Two tooth bands separated by a root-core center gap; optional hub and through bore | Matching external family, width and gap | Module or DP; retained shift; root core remains connected |
| Internal ring | Native inward involute teeth and connected outer rim | 24-tooth spur pinion with 48/60-tooth ring; either member order | Unshifted 20-degree spur subset; explicit outside diameter |
| Rack | Native straight flanks, circular root transitions and connected backing | One matching unshifted spur pinion; either member order | 4-64 whole centered pitch cells, zero helix, conservative finite travel |
| Straight bevel | Spherical-involute flanks, back-cone heel/toe boundaries and custom spherical-circle root transitions | Intersecting 90-degree shafts with a shared pitch apex and derived tooth-count ratio | Declared 20-degree, zero-shift subset; 24-64 teeth per member; availability follows the registry |
| Worm and wheel | ZA axial-straight worm or generated single-envelope wheel | One right-hand start, perpendicular shafts, derived center and worm-owned drive | Axial module 0.5–2 mm, normal pressure 20°, quotient 10 and 32–64 wheel teeth; see the declared subset below |

[ADR-0035](0035-retained-gear-pitch-hubs-and-shift.md) owns the retained pitch,
intrinsic hub and shifted external-profile contracts. The four external cylindrical families
accept module or diametral pitch in an explicit normal/transverse plane. Rack,
internal ring and bevel retain module input. Worm gearing uses axial module
with normal pressure angle, as declared below. Recipes retain tooth count, face width,
helix magnitude/hand, bore, center gap, circular-root-fillet ratio, minimum
root wall and two separate tangential thinning inputs. Design backlash applies
per gear at its reference circle; fabrication allowance adds further thinning.
Neither is a strength, standardized fit or printer calibration setting.

The canonical policy bounds module to at least 0.01 mm, rotary count to 4-128 whole
teeth (rack count is 4-64 cells; bevel and worm subsets have separate limits below), the selected pressure angle to 14.5-35 degrees and helix magnitude to
0-45 degrees. Additional resolved geometry checks can refuse a recipe within
those input ranges. In particular, the conservative unshifted rotary gear/pinion tooth-count
policy uses `ceil(2 / sin(normal_pressure_angle)^2)` without virtual-tooth
credit for a helix. Nonzero shift or retained matched context
applies the direct basic-rack transverse undercut projection and additional
profile/contact constraints in the
[current ADR-0035 expansion](0035-retained-gear-pitch-hubs-and-shift.md#2026-11-06-extend-retained-shift-to-external-helical-families).
This is the supported envelope, not a universal cutting or undercut theorem. Root fillets are circular arcs rather than cutter trochoids.

The dimensional derivation follows the normal/transverse conventions in
[KHK's gear calculation reference](https://khkgears.net/new/gear_knowledge/gear_technical_reference/calculation_gear_dimensions.html)
and [Basic Guide B](https://khkgears.us/media/fqrlut1m/abcs-of-gearing-b.pdf).
For the transverse unshifted system, addendum is the transverse module; the
normal system uses normal module. The registry records source references.
No accuracy grade, current-standard compliance, load or lifetime rating is
implied by these geometric conventions.

## Native geometry and paired placement

The [rotary involute adapter](../../crates/multicad-kernel/src/gears.cpp) constructs a
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
internal-ring/spur, finite rack/spur or intersecting-axis bevel combinations described below. It requires matching transverse module and
pressure angle, face width, gap and applicable helix magnitude/opposite hands.
The Part heads must remain the gear generators. It checks actual B-reps at
nine phases over one pinion tooth cycle and at the current pose. Completed cycle
checks have a bounded worker-owned cache keyed by actual geometry inputs,
frames, identities and policy. Current pose is always checked; a partial or
cancelled cycle is never reusable. These finite samples do not establish
continuous collision-free motion or manufacturing fit.

## Retained pitch, hubs and shifted external profiles

The [coverage resolver](../../crates/multicad-core/src/model/gears/coverage.rs)
keeps one active `GearParameters.module` dimension. `pitch_input` declares
whether that scalar is module in millimeters or diametral pitch in
`teeth_per_in`. `module_system` independently selects the normal/transverse
plane. Expressions and bindings remain authored source; conversion supplies
resolved millimeter geometry. Omitted new fields preserve legacy module input,
no hub and unshifted geometry.

Optional intrinsic hubs share the gear axis and retain diameter plus local axial
start/end coordinates relative to the tooth band. The native generator includes
the hub in one solid and cuts a continuous bore through the complete extent.
A hub wholly inside the existing root core is allowed. Double-helical root-core
gaps keep their existing connected core; the hub does not fill the tooth gap.
[ADR-0035](0035-retained-gear-pitch-hubs-and-shift.md) owns dimensional constraints,
shift equations, source references and conditional reader capabilities.

Matched shifted external members retain each other's original typed count and
shift sources. Non-Spur pairs require the same selected module system; their
selected-plane radial depths remain distinct from reference pitch conversion. Shared edits update both recipes atomically; drive-only edits preserve
them. Canonical checks require a valid contact path above both actual custom
fillet joins. Native evaluation retains whole-solid overlap refusal at every
sample and the current pose. Spur and Helical use named-flank existence witnesses
with whole-solid distance fallback. Herringbone and DoubleHelical require a
separate named-flank witness in each band; whole-body proximity cannot stand in
for either one. These witnesses do not claim a global closest distance or loaded
contact. The current ADR expansion owns the exact normal/transverse equations,
basic-rack versus custom-root distinction and conditional reader gate.
The cycle cache includes the engagement mode as well as complete native geometry,
frames and policy. Final test runs and actual macOS checks belong in verification
records; this reference states the implementation contract.

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

## Spherical-involute straight bevel subset

The [bevel source](../../crates/multicad-core/src/model/gears/bevel.rs)
defines a bounded straight-bevel system with a separate resolved cone geometry.
This section states the declared implementation contract; it does not record
completed native or interactive verification. The registry controls advertised
availability. Gleason/octoid, spiral and Zerol systems remain unavailable.

A bevel recipe adds optional `bevel` dimensions: `tooth_system`, `mate_teeth`
and `shaft_angle`. Other families omit the object. The supported system is
`spherical_involute_straight`, with 20-degree pressure, 90-degree intersecting
shafts and 24-64 whole teeth on each member. Outer module must be at least
0.01 mm. Face width follows the pitch-cone generator and must not exceed
`min(R/3,10*m)`. Helix, center gap and profile shift are zero, hand is `none`
and outside diameter is absent; intrinsic hubs are unavailable. Normal and transverse
module coincide at zero helix. The source frame origin is the pitch apex.
The toe root envelope must retain the authored minimum wall around the bore.

The active flank follows Ligata and Zhang's
[2011 IAJC-ASEE paper, PDF page 2, equations 1-3](https://ijme.us/cd_11/PDF/Paper%20163%20ENG%20107.pdf).
For base half-cone angle beta and roll parameter t, let
`B=(sin(beta)*cos(t),sin(beta)*sin(t),cos(beta))` and
`T=(-sin(t),cos(t),0)`.
The unit spherical flank is
`C=cos(t*sin(beta))*B-sin(t*sin(beta))*T`, with
`sin(beta)=sin(delta)*cos(alpha)`.
Multiplying C by distance from the apex generates the flank surface.
Its centering uses the spherical involute at the pitch cone.
The paper distinguishes octoidal cutting approximations on PDF page 3 and
uses a separately designed smooth root transition on PDF page 6.

Blank dimensions follow
[KHK Table 4.17, PDF page 31, printed page 623](https://khkgears.us/media/izojpong/gear_guide1.pdf)
and the [KG standard straight-bevel table, PDF page 1, printed page 113](https://www.kggear.co.jp/en/wp-content/themes/bizvektor-global-edition/pdf/9.7_Calculation-for-Standard-straight-bevel-gear_TechnicalData_KGSTOCKGEARS.pdf).
Neither retrieved manufacturer excerpt identifies a publication revision.
The [source manifest](../../artifacts/bevel-review-20261105/source-file-manifest.json)
retains their byte hashes. These blank conventions combined with the academic
flank form define the declared engineering system; they do
not establish Gleason or Klingelnberg cutter conformity.

For module m and tooth counts z and zm, `delta=atan(z/zm)` and
`R=m*z/(2*sin(delta))`. Heel addendum is m and dedendum is 1.25m.
The tip and root cone angles are `delta+atan(m/R)` and
`delta-atan(1.25*m/R)`.
In a radial meridian, heel and toe satisfy
`r*sin(delta)+z_axial*cos(delta)=R` and `R-b` respectively.
A direction at polar angle gamma therefore reaches the heel at
`rho=R/cos(gamma-delta)` and the toe at `(R-b)/cos(gamma-delta)`.
These back-cone boundaries determine the reported diameters and axial limits.
Spherical caps or flat axial caps would give different dimensions.

The [native adapter](../../crates/multicad-kernel/src/bevel.cpp) uses these
spherical-involute directions and cone boundaries. Roots use explicitly custom
tangent small circles on the outer reference sphere, projected onto the back
cones. Their radius is `module*root_fillet_ratio`; they connect the root cone
to the active flank or its bounded meridian extension below the base cone.
They are engineering transitions without a cutter-generated root claim.
[Fuentes-Aznar, Gonzalez-Perez and Pasapula, March 2017, printed pages 28-29](https://thermalprocessing.com/wp-content/uploads/2017/201703/0317-Forging.pdf)
corroborate spherical flank orientation and cone dimensions, but their radial
surface parameter does not define the manufacturer's back-cone trimming.

A bevel pair clones each member's opposite original typed tooth-count dimension
into its `mate_teeth`. Both recipes share one typed `shaft_angle`; an expression
remains an expression with its existing bindings. Canonical preparation refuses
conflicting complete recipes. Updating both recipes is atomic, and a single
member edit cannot break the pair's coupling. Existing assembly anchors remove
local definition frames before applying the common apex and shaft layout.

In the pair basis, member A's shaft is +Z and member B's shaft is rotated about
+Y through the shared shaft angle. Their mating meridians are A=0 and B=pi.
For drive theta, phases are `theta` and `pi-pi/zB-theta*zA/zB`.
The second member follows the negative tooth-count ratio. Pair inspection
returns a separate `bevel` cone context; its center-distance field is zero
because the pitch apices coincide.

The [native pair guard](../../crates/multicad-model/src/gear_pairs.rs) checks
whole-solid no-overlap before seeking engagement proximity. All nine tooth-cycle
samples and the current pose retain these checks. Completed cycle reuse excludes
the drive coordinate only; current pose still runs on every compile.
`current_engagement` records `distance_mm`, `kind`, `faces` and `points_mm`.
`bevel_flank_faces` identifies an actual close named-face pair within the sum
of authored thinning plus ten linear tolerances. It is an existence witness,
not the global minimum distance. If the bounded face search misses, the guard
runs the whole-solid query and records `whole_solid_fallback` with null `faces`.
Face ordinals are evaluation-local and cannot serve as persistent references.
The points use the evaluated pair layout coordinates before any independent
outer occurrence placement. This report does not certify continuous motion,
loaded contact or physical fit.

`model.gear.bevel.1` is required when direct, suppressed or template-only saved
source contains a bevel recipe. A reader lacking this capability refuses the
project instead of interpreting it as a spur gear. The
[core reader checks](../../crates/multicad-core/tests/model_bevel_gears.rs),
[native adapter tests](../../crates/multicad-kernel/tests/spherical_bevel.rs) and
[model pair tests](../../crates/multicad-model/tests/bevel_gears.rs) define the
executable checks; their presence alone does not record a passing run.

## ZA worm and generated wheel subset

[ADR-0034](0034-generated-za-worm-wheel.md) owns the construction equations,
primary sources, approximation proof and limits. This system is explicitly ZA;
it does not relabel KHK's stock Type III worm. The custom cylindrical wheel
blank has radius `zm/2+m`. Actual generating-hob flanks, swept sharp-tip root
transitions and a torus root form its cutting space.

Both members use `GearFamily::Worm` with an explicit `WormMember` role.
`teeth` is the member's own count: one start for Worm, 32–64 teeth for Wheel.
`WormDimensions` retains tooth system, the opposite original typed count and
the shared typed diameter quotient. Coupling clones the opposite source
dimension; it does not replace a parameter reference or expression with its
evaluated number. Conflicting explicit source dimensions are refused.

Module is axial, while the pressure-angle input is normal 20°. The derived
axial pressure and lead angle are displayed separately. Worm face width means
threaded length `16m–32m`; Wheel face width means axial width `0.5m–2m`.
Each member's backlash plus fabrication allowance lies within `0–0.1m`.
The generated wheel uses an enlarged hob tip for `0.25m` root clearance and
its own pitch-space allowance. Bore checks retain the requested minimum wall
at the actual minimum root radius. Other tooth systems, starts, hands,
quotients, shifts and fillets are refused.

Pair drive always belongs to the worm, including when it is member B. Existing
assembly relationships derive center distance and perpendicular axes. Native
validation checks the current pose and nine phases over one full worm turn,
requiring no intersection solid and an actual proximity witness within the
authored allowance. Selected named flank faces may supply that existence
witness; a miss falls back to the whole native bodies. It is not a claim that
the selected distance is the global minimum. Worm intersection checks opt into
OCCT's internal parallel workers with the same validity and error guards.

Reader capability `model.gear.worm.1` is required for retained direct,
suppressed or template-only worm recipes. Unsupported readers refuse these
documents. The focused [core](../../crates/multicad-core/tests/model_worm_gears.rs),
[kernel](../../crates/multicad-kernel/tests/worm_gears.rs),
[model](../../crates/multicad-model/tests/worm_gears.rs) and
[MCP](../../crates/multicad-mcp/tests/worm_gears.rs) tests define the checks;
passing runs and actual macOS workflows belong in the verification record.
Finite phase samples do not establish continuous contact or clearance,
self-locking, efficiency, load capacity, standards compliance or physical fit.

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
