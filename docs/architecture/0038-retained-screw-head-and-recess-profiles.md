# ADR-0038: Retained screw lengths and native head/recess profiles

Status: accepted implementation design, 2026-11-06, for
[CAD-236](https://plans.tokenet.de/CAD/236). Implementation verification and human
acceptance remain separate.

Countersunk screw lengths commonly include the head. Reinterpreting the existing
`underhead_length` field would change saved recipes and pinned definitions.
The canonical recipe therefore retains either the legacy optional
`underhead_length` or a tagged `length_input`, with exactly one present.
`length_input` selects Underhead or Overall and retains its authored Dimension,
including expressions and stable parameter references. Overall derives the
neck-to-tip span by subtracting the physical head height. Thread length remains
a separate extent ending at the tip. A countersunk neck construction datum does
not imply a flat bearing seat.

Legacy serialization omits the new alternative and retains its old field and
meaning. MCP omission preserves the exact source; explicit length replacement
clears the other alternative, and supplying both refuses. Changing the head
does not silently change the length convention. Readers require
`model.mechanical_part.machine_screw.length_input.1` when that field is retained,
including explicit null. New head or drive variants additionally require
`model.mechanical_part.screw_profiles.1`. Discovery includes references,
templates, private instances and suppressed/nested source.

The same typed recipe now supports pan, button and countersunk heads, tangent
circular hexalobular recesses, and Phillips H0–H4 recesses alongside the existing
socket-cap, external-hex and internal-hex forms. New variant payloads are boxed
in Rust to keep enum storage bounded, with flat serialization. Lengths and
angles retain explicit units. UI head, drive and Phillips-size drafts retain
their own exact Dimensions and invalid edits; incompatible choices require an
explicit repair. MCP kind changes require complete new dimensions, while
partial edits retain omitted fields and expressions. Card dimensions contain
lengths only; angles and Phillips sizes appear as separate labeled facts.

Head meridians retain a measurable physical front land, spherical crown and
cylindrical skirt where applicable, a countersunk cone/rim/chamfer where
applicable, and real tangent neck blends. Hexalobular sections use twelve
tangent circular arcs. Their selected arc radius and conical bottom are
engineering targets, distinct from normative functional gauges. Phillips uses
the actual inclined C–C construction plane, its H0 rounded inner form or H1–H4
shallow V, and a separate mouth roundover. Projected source dimensions, entry
rounding, functional gauge penetration and removed cutter depth have distinct
meanings.

The native builder uses existing audited revolve, extrusion and Boolean APIs.
It fuses the thread before cutting the recess, preserving countersunk cavities
that legitimately extend into the neck. Nonfunctional cutter closures extend
outside the head. Completed-part cache keys include every resolved physical
input and policy; authored semantic geometry names carry provenance. The
shared native thread factory remains unchanged. An observed warm-cache source
identity test is required before adding a copy adapter to that path.

Canonical validation requires positive geometric separation. Native preflight
also enforces the selected GeometryPolicy minimum feature on the physical top
annulus, remaining blind floor and radial body/recess clearance. Piecewise
monotone envelope bounds include the crown, skirt, rim, cone, neck, shaft,
thread-root and recess joins; bounded subdivision refuses an unresolved case.
This applies the application's geometry policy without inventing an ISO wall
limit or rejecting every recess that reaches below the head.

Sourced presets keep exact dated standard/manufacturer facts separately from
the selected native engineering contour. The existing screw curation retains
its revision and 19 legacy rows, with additive profile revision, entries,
coverage and primary references. Every selected family/nominal row names its
discrete length and convention; unsupported matrix cells carry reasons.
Immutable dimensional references remain literal and self-contained, while
configurable length/head/thread dimensions retain their expressions. A single
Count input selects modeled or cosmetic representation in each new pin.

Source PDFs, artwork and supplier CAD are not redistributed. Bundled notices
identify scalar references, preserved BOLTS licensing and independent geometry
adaptations. Dimensional agreement does not establish current-standard
compliance, normative gauge acceptance, physical driver fit, thread fit class,
material, strength or supplier stock. Configured deviations retain the
original reference and become custom through the canonical comparison.
