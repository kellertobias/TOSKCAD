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
| Internal ring | Unavailable | Unavailable | [CAD-227](https://plans.tokenet.de/CAD/227) |
| Rack and pinion | Unavailable | Unavailable | [CAD-228](https://plans.tokenet.de/CAD/228) |
| Bevel | Unavailable | Unavailable | [CAD-229](https://plans.tokenet.de/CAD/229) |
| Worm and wheel | Unavailable | Unavailable | [CAD-230](https://plans.tokenet.de/CAD/230) |

Intrinsic hubs, nonzero profile shift and explicit diametral-pitch input remain
[CAD-231](https://plans.tokenet.de/CAD/231). Existing recipes use normal or
transverse module and pressure angle. They retain tooth count, face width,
helix magnitude/hand, bore, center gap, circular-root-fillet ratio, minimum
root wall and two separate tangential thinning inputs. Design backlash applies
per gear at its reference circle; fabrication allowance adds further thinning.
Neither is a strength, standardized fit or printer calibration setting.

The canonical policy bounds module to at least 0.01 mm, count to 4–128 whole
teeth, the selected pressure angle to 14.5–35 degrees and helix magnitude to
0–45 degrees. Additional resolved geometry checks can refuse a recipe within
those input ranges. In particular, the conservative unshifted tooth-count
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

The native pair guard requires matching external family, transverse module and
pressure angle, face width, gap and applicable helix magnitude/opposite hands.
The Part heads must remain the gear generators. It checks actual B-reps at
nine phases over one pinion tooth cycle and at the current pose. Completed cycle
checks have a bounded worker-owned cache keyed by actual geometry inputs,
frames, identities and policy. Current pose is always checked; a partial or
cancelled cycle is never reusable. These finite samples do not establish
continuous collision-free motion or manufacturing fit.

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
