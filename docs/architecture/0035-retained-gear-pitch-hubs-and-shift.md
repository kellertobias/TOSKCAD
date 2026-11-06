# ADR-0035: Retain gear pitch intent, intrinsic hubs and matched shift context

| | |
| --- | --- |
| Status | Accepted engineering contract; execution evidence and human acceptance remain separate |
| Date | 2026-11-06 |
| Decider | Codex, using delegated implementation authority |

This record owns the source and construction choices for external cylindrical
gear pitch input, intrinsic hubs and shifted Spur profiles. The
[gear reference](gears.md) owns family coverage; generated MCP schemas own wire
arguments. This decision does not record completed verification.

## Context

A saved gear must resolve independently of its placement or a currently open
editor. Diametral pitch needs a typed reciprocal-length source, and hub material
must participate in the same native body, bore, ancestry and pair checks as its
teeth. Matched shifted gears also need pair-dependent tip shortening without
introducing a feature-to-pair-to-feature dependency cycle.

[KG Technical Data, section 1.5, printed pages 13-14](https://www.kggear.co.jp/en/wp-content/themes/bizvektor-global-edition/pdf/1.5_Fundamental-dimensions-for-various-sizes-of-Tooth-profiles_TechnicalData_KGSTOCKGEARS.pdf)
defines diametral pitch as teeth per inch of reference diameter.
[MAHLE's module reference](https://www.mahle-powertrain.com/transmissions/en/encyclopedia/module.jsp)
distinguishes normal and transverse pitch planes.
[KHK's official gear guide, PDF page 12/printed 604, Table 3.5, and PDF pages 16-17/printed 608-609, Tables 4.3-4.4](https://khkgears.us/media/izojpong/gear_guide1.pdf)
supplies shifted tooth thickness, the undercut condition and clearance-preserving
pair dimensions. The retrieved guide has no stated publication edition; its PDF
creation date is not treated as an edition. The source PDF is not redistributed.

## Decision

Retain one active scalar dimension in `GearParameters.module`, interpreted by
`pitch_input`. `module` means millimeters; `diametral_pitch` means the genuine
`teeth_per_in` unit with dimension count/length. `module_system` independently
selects Normal or Transverse. For the selected plane, `m_mm=25.4/DP`; the
conversion uses exact 25.4 millimeters per inch. For helix angle beta,
`mn=mt*cos(beta)` and `Pn=Pt/cos(beta)`. Retain the original expression and
bindings rather than a converted literal. There is no second authoritative
pitch scalar. Absent `pitch_input` retains the legacy module interpretation and
serialization.

Allow optional `hub` on Spur, Helical, Herringbone and DoubleHelical. It retains
`diameter`, `axial_start` and `axial_end` dimensions in the gear's local frame;
the tooth band occupies `Z=0..face_width`. Canonical resolution requires a
positive axial extent, at least 0.01 mm overlap with the tooth band, hub radius
at least 0.01 mm below the root radius, and the authored `minimum_wall` outside
the bore. A hub contained entirely in existing root-core material is allowed.
Gapped double-helical gears retain their existing connected root core; their
tooth gap is still checked above that core.

The [native adapter](../../crates/multicad-kernel/src/gears.cpp) fuses a real
coaxial hub cylinder into the gear and cuts one bore through the full combined
axial extent. It requires a valid connected solid, preserves Boolean face
history and names surviving hub side/end faces. Coincident tooth-band caps keep
both their cap classification and surviving Boolean-derived origins. Hidden hub
interfaces need no output face. Whole-solid pair intersection checks include hub
material. Native tolerances and manufacturing export guards remain unchanged.

Support nonzero profile shift only for 20-degree external Spur, with coefficient
`-0.5..1`. This is an explicit engineering coverage bound, not a standard limit.
Counts remain within the canonical whole-count domain, further narrowed by the
source undercut and actual profile checks. Other families continue to refuse
nonzero shift. Circular roots are custom tangent transitions, without a
rack-cutter trochoid or standard hob-conformity claim.

For module m, count z, own shift x, pressure angle alpha and total tangential
thinning t, the reference thickness is
`s=m*(pi/2+2*x*tan(alpha))-t`. The base half-angle is
`pi/(2*z)+2*x*tan(alpha)/z+inv(alpha)-t/(m*z)`, where
`inv(a)=tan(a)-a`. An independent standalone gear has
`ra=m*(z/2+1+x)` and `rf=m*(z/2-1.25+x)`.

A matched recipe retains optional `shift_mate { teeth, profile_shift }` as typed
source dimensions. For `Z=z1+z2` and `X=x1+x2`, solve
`inv(aw)=inv(alpha)+2*X*tan(alpha)/Z`, then compute
`y=Z/2*(cos(alpha)/cos(aw)-1)` and `delta=X-y`.
The operating center is `ac=m*(Z/2+y)`. Each matched tip is
`rai=m*(zi/2+1+xi-delta)`; its root remains `m*(zi/2-1.25+xi)`.
The generator resolves that geometry from retained mate dimensions without
reading an assembly or pair feature.

Require `x>=1-z*sin(alpha)^2/2`, positive crest thickness, separated circular
root transitions, `root<pitch<tip`, and minimum bore wall. Pair resolution also
requires a positive acute operating angle and a transverse contact ratio greater
than one. With operating radii `rwi=ac*zi/Z` and
`qi=sqrt(rai^2-rbi^2)`, approach is `q2-rw2*sin(aw)` and recess is
`q1-rw1*sin(aw)`. Both must be positive; their sum divided by
`pi*m*cos(alpha)` is the contact ratio. The complete active path must remain
above each actual circular-fillet/involute join, using that join's native
involute parameter. A base-circle-only interference test is insufficient.

The [coupling helper](../../crates/multicad-core/src/model/gears/coverage.rs)
checks explicit mate dimensions against the opposite original typed count and
shift before copying either context. Equal evaluated numbers from different
bindings do not erase conflicting authored intent. Shared geometry edits update
both recipes and retained contexts atomically. Drive-only edits keep those
recipes intact. At drive theta, phase A is theta and phase B is
`pi+pi/zB-theta*zA/zB`; the operating center follows the equations above.
Parameter dependencies rebuild both matched bodies through the ordinary typed
source graph.

The [model guard](../../crates/multicad-model/src/gear_pairs.rs) checks unchanged
whole-solid no-overlap at nine phases over one tooth cycle and at the current
pose. It then seeks actual named native flank faces within total authored
thinning plus ten linear tolerances. `shifted_spur_flank_faces` identifies a
sufficient existence witness, not a global minimum. A candidate miss retains
the whole-solid distance query and records `whole_solid_fallback`. Face ordinals
are evaluation-local. The cache key includes complete constructor inputs,
identities, frames, policy and engagement mode; current pose is checked even
when the completed cycle is reused. These finite checks do not certify
continuous motion, loaded contact or physical fit.

Saved data conditionally requires `model.gear.coverage.1` for hub, DP, retained
mate context or nonliteral-zero shift intent. `parameters.teeth_per_inch.1`
protects the new unit independently of a gear. Capability discovery includes
suppressed and template-only source and nested unit-bearing data. A reader
without the required capability refuses the source rather than interpreting a
DP scalar as a millimeter module. Old recipes without these inputs retain their
existing behavior and capability requirements.

## Consequences

Positive: each saved gear remains independently resolvable, including matched
tip shortening and parameter-bound pitch. Hub material and its continuous bore
stay within the generator's actual native geometry and interference checks.

Negative: one legacy-named scalar has two explicitly tagged unit conventions.
Callers must preserve the tag when editing it. Matched shift context duplicates
opposite typed dimensions, so explicit-conflict checks, grouped edits and reader
capability discovery are required wherever recipes can be retained or copied.

Neutral: source equations define a bounded geometric system with authored
thinning and custom root transitions. They do not assign an accuracy grade,
printer calibration, strength rating or standardized fit. Test execution and
actual macOS workflows remain separate verification records.

## Alternatives considered

Separate module and DP fields would make each convention directly named, but
would leave two independently editable authoritative values and stale expression
bindings. One tagged scalar keeps the source unambiguous while retaining legacy
serialization.

Assembly-only tip shortening would avoid stored mate context, but would make a
saved standalone member resolve differently outside its assembly. A literal
cutback would be simpler to serialize, but would become stale after parameter
changes. Retained typed mate context keeps the geometry source-independent.

A downstream hub Part would reuse ordinary component tools, but would bypass
single-generator pair validation and leave the through-bore and provenance
split across outputs. An intrinsic hub keeps that material within the generator.
Restricting shift to zero-sum pairs would avoid changed center distance, but
would exclude the sourced positive and negative center modifications.
