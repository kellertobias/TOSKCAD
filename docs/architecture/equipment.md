<!-- Kind: reference; Audience: native architecture and drawing maintainers; Purpose: define equipment source, host and drawing contracts; Non-goals: installation certification, circuits, routing or manufacturer catalog certification. -->
# Apartment equipment

Equipment belongs to Model → Architecture and uses the existing pinned symbol and included Library infrastructure. The catalog contains 22 owned generic families in Electrical and Lighting, Heating, Sanitary and Appliances. Width, depth and height are editable planning dimensions. Power outlets identify Type F; lamp holders identify an E27 planning envelope and remain distinct from luminaires. A cooktop is a separate family from a full cooker with oven. Junction boxes have a distinct enclosure, cover and JB plan mark; they are not socket aliases. Built-in and standalone options follow the selected type's declared support.

These definitions contain original simple native geometry and original monochrome symbols. They claim no manufacturer dimensions, connector fit, IEC/DIN symbol compliance, electrical approval or plumbing connections. VDE's [consumer guidance](https://www.vde.com/topics-en/consumer-protection/power-supply-and-security-when-traveling) supplies the Type F/Schuko terminology; no source artwork was copied. Switzerland and other connector conventions are not silently represented as Type F.

## Canonical source

`architecture.equipment_template` schema 1 identifies the component, three length parameters, a true-size footprint sketch, an optional normalized paper-mark sketch, mounting class, installation modes and explicit `tosk_cad_generic_v1` convention. Symbol authoring captures and remaps these typed references with their source. `architecture.equipment_instance` schema 1 points to the canonical symbol ownership record and retains installation mode, optional label and separate service/door envelope references and visibility. It duplicates neither dimensions nor pose.

`architecture.equipment_attachment` schema 1 is an absolute placement driver owned by the outer Instance. It depends on an explicit floor and optional wall, rather than the owner or its symbol record. This keeps occurrence evaluation acyclic. Floor placements store XY, an anchor height above the floor and yaw. Wall placements store wall identity, segment and path signature, side, along distance, face standoff, anchor height and relative yaw. Ceiling placements store XY, drop and either the next slab underside or an explicit ceiling height above the floor.

Wall-center types measure their center above the floor; wall-bottom and floor types measure their bottom. Countertop types measure their top. Ceiling types measure drop below their selected ceiling datum. The next slab underside is the level's floor-to-floor height minus slab thickness. A 300 mm outlet and 1100 mm switch therefore follow changes to floor elevation without storing stale world Z coordinates.

Coordinate/length/thickness/height edits to a retained wall path preserve attachments. A missing wall/floor, changed path point count or open/closed state, out-of-segment anchor or anchor outside the wall height reports a repair error. Source inspection remains available before pose resolution, so choosing replacement host intent can repair it. Upright Move/Copy operations convert their world edit back to host-relative intent; copies retain separate private symbol inputs, literal labels and owned clearance envelopes. Crossing a wall face requires explicitly selecting the other side.

Equipment projects require `architecture.equipment.1`; hosted placements additionally require `architecture.equipment_attachment.1`. Stored drawing category filters also require the equipment capability. Older readers must reject an unknown required capability.

## Plan and clearance output

A horizontal top projection replaces equipment solid projection with the solved authored footprint at the current occurrence pose. The physical footprint retains model scale. An electrical mark has a separate explicit paper size, normally 3.5 mm, so an outlet below the plan cut remains readable. Wall marks sit beyond the physical front with a 0.5 mm paper gap and a line back to the real attachment point. Optional labels use 3.5 mm paper text offset from the drawn extent. Linework is transparent, monochrome and uses 0.18 mm paper strokes. PDF and drawing DXF consume the common checked page geometry. Physical sections, elevations, bottom views and tilted occurrences retain solid projection.

The owned generic mark legend is F/2F/3F for single/double/triple Type F power outlets, RJ for RJ45 data, TV for antenna, S for switch and LH for lamp holder. Luminaires use a crossed circle. Appliance codes WM, DR, FR and DW use 3.5 mm paper text. These conventions are explicit generic symbols rather than a claim of standards conformance.

Per-view equipment category filters permit electrical-only and furnished plans from the same source. Absent filters include all categories; an explicit set includes only its listed categories. Non-equipment context remains present. Private footprint and mark sketches are hidden from the ordinary 3D sketch overlays.

Optional door/access and service envelopes use existing `EnvelopeDefinition` geometry, target the same outer Instance and remain separate from the physical footprint. Display visibility does not disable explicit clearance checking. These are conservative access boxes, not articulated door sweeps or manufacturer service clearances. Plan rendering computes their outline boxes directly; it does not run the clearance intersection checks merely to draw a page.

## Shared transactions and evidence

The equipment Inspector and MCP `set_equipment` prepare the same atomic source candidate and require native preflight before Apply/commit. `inspect_equipment` returns retained source and host errors; `equipment_profiles` describes available generic families. Generated `tools/list` remains authoritative for wire schemas.

`equipment_review` builds a saved all-family layout with furnished pages at 1:100 and 1:50 and an electrical-only page at 1:50. Core tests cover host following, failure/repair intent, source history and pins. Native tests measure physical bounds, authored plans and independently parsed PDF/DXF output. Native macOS interaction and human visual acceptance require separate recorded evidence; these source contracts alone do not establish either.
