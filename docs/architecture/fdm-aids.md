# FDM design aids

FDM design aids are breakaway tabs, thin sacrificial bridges and removable
support blocks. They are ordinary model features in the canonical document.
The window and the MCP server use the same core API.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/fdm.rs` | Aid and process source types, classification and transactions |
| `crates/multicad-model/src/fdm.rs` | Native aid solids, derived support height and the aid inspection |
| `crates/multicad-model/src/export/mod.rs` | Removable-aid exclusion in STEP, planar DXF and mesh recipes |
| `crates/multicad-ui/src/modeling/fdm_aid.rs` | The window's Tab / bridge / support form |
| `crates/multicad-ui/src/modeling/fdm_aid/placement.rs` | Viewer picking, contact marker, direction handle and review overlay |
| `crates/multicad-mcp/src/tools/fdm.rs` | MCP tools |

## Aids are their own Parts

An **aid** is an `fdm.aid` feature with one output Part. It names:

- the **target**, the finished Part it serves;
- its **shape**, described below;
- optional **process allowances** to inspect it against.

The target is an input of the aid. The aid never changes the target's
solid, so the finished Part stays the same with or without its aids.

A new aid Part takes the target's Material, or a Material given in the
request. An aid cannot serve another aid.

### Classification

A Part is a **removable aid** when one of its features is an `fdm.aid`
feature. `model::aid_parts` returns every such Part. Names and tags never
classify a Part: a finished Part called "Support" is finished geometry.

## Shapes

Aids are placed in world millimeters with +Z as the build direction. Every
dimension can be a parameter expression.

- **Breakaway tab.** A flat pad on the build plane. The **contact point**
  is a point on the part outline. The pad runs a **length** outward along a
  **direction** about +Z and an **overlap** back under the part. It has a
  **width** across and a **thickness** up from the contact point.
- **Thin bridge.** A bar whose cross-section centre runs from a **start**
  to an **end** point. The **width** is horizontal; the **thickness** is
  perpendicular to the bar and the width. A bridge can name a **second
  Part** it ties the target to. A vertical bridge is refused.
- **Removable support.** A flat-topped block over a rectangular
  **footprint**: a corner, a direction about +Z, a length and a width. The
  block rises from a **base** height, usually the build plate. Its top is
  not authored, as described next.

The side faces carry the provenance roles `tab.contact`, `tab.end`,
`bridge.start`, `bridge.end`, `support.start` and so on.

### Derived support height

The native evaluator intersects the target's current exact solid with a
prism over the footprint. The support top is the lowest point of that
intersection, less the support's **gap**. A target edit therefore moves the
support top with it.

The evaluator refuses a support, naming the aid, when:

- the target lies entirely below the base;
- the footprint is not under any target material;
- the target reaches down to the base over the footprint, leaving no room.

## Process allowances

An `fdm.process` extension records explicit allowances:

- the **layer height** and **line width**;
- the **minimum layers** for tab and bridge thickness;
- the **minimum lines** for aid widths;
- the **support gap**, the smallest Z gap between a support and the part;
- the **fit clearance**, the smallest gap between an aid and any Part it
  does not serve.

The allowances are not a slicer profile. They only drive the inspection.

## Inspection

`Model::fdm_aids` measures each evaluated aid from the exact solids, using
the native solid-pair inspection (Boolean COMMON and minimum distance). Each
record contains:

- the aid's world bounds and volume;
- its relation to the target: separated, in contact or overlapping, with
  the distance and common volume;
- the same for a bridge's second Part;
- the neighbouring Parts inside the fit clearance;
- review findings.

The findings are:

- a tab that does not touch its target;
- a bridge that does not reach its target or second Part, or that lies
  entirely inside them;
- a support that overlaps its target, comes closer than its own gap, or
  leaves a gap below the process support gap;
- a tab or bridge thinner than the minimum layers, or not a whole number of
  layers;
- an aid narrower than the minimum lines;
- a neighbouring finished Part inside the fit clearance. Aids are not
  checked against each other.

The **placement findings** are the contact findings: a tab or bridge that
misses what it serves, a buried bridge, a support that overlaps or comes
closer than its gap, a missing target solid and a crowded neighbour. They
are listed apart as `contact_issues` (and also in the findings). The
window refuses to Apply an aid with any placement finding. Allowance
findings, such as a thickness that is not a whole number of layers, stay
advisory.

For a tab, the record also holds the **exact contact** near its own: the
outline point at the target's lowest height found along the tab's
direction, described next.

A full model evaluation of a document with aids records these inspections
with the evaluation, so a preview carries the inspection of its own
candidate.

## Exact tab contact

`Model::fdm_tab_contact` finds where a tab meets a finished Part from the
Part's exact solid. It takes a point on or near the Part's side and an
optional outward direction in the build plane. The point is moved to just
above the Part's lowest height, and 1 mm along the direction to clear a
display surface. The exact minimum distance from that probe to the solid
gives:

- the outline point, placed at the Part's lowest height;
- the outward direction, normal to the Part at that point, and its angle
  about +Z.

Both hold to within the 0.1 µm probe. A point inside the outline, or a
point no side faces, is refused.

A target that is only a reusable Assembly component, such as a printed
drawer on its slide, has no world body. It is measured as its component
definition, the same solid and frame its aids are built from.

## Export

Every STEP, planar DXF and mesh recipe has an aid policy:

- `IncludeAll` exports aid Parts like any other Part;
- `ExcludeRemovable` leaves aid Parts out.

With exclusion, a separate-file recipe writes no file for an aid, and an
assembly file leaves the aid out of its objects. The recipe report lists
the Parts left out in `excluded_aids`, and a recipe left with no Parts is
Empty. Exclusion uses the classification above; it never deletes design
intent.

Print orientation and mesh packaging stay Export controls. Aids assume the
model's +Z is the build direction, so a print recipe must keep that
orientation. Export → Print writes a [3MF print package](print-packaging.md)
that keeps model coordinates and shows the process records and inspection
findings of the aids serving the chosen Parts. A chosen placed copy brings its
aids at the copy placement.

## Access

- **Window.** Model → FDM aids → **Tab / bridge / support**. The form
  places a tab, bridge or support for a finished Part. Preview builds the
  native solid and Apply commits it. Opening an applied aid edits it in
  place, keeping its Part. Placing in the viewer is described below.
- **MCP.** `set_fdm_process`, `set_fdm_aid`, `remove_fdm_aid`,
  `fdm_aid_report` and `fdm_tab_contact`.
- **Undo.** Creating, editing and removing aids and processes are single
  transactions with Undo and Redo.

## Placing in the viewer

A new aid opens ready to pick. The Inspector numbers stay editable, and
picks only fill them in:

- **Tab: Pick contact.** Click a side face of a finished Part. The
  contact is the clicked point at the Part's exact lowest height, and the
  direction is the face's outward normal in the build plane. A top or
  bottom face is refused. The clicked Part becomes the target.
- **Bridge: Pick start, Pick end.** The start sets the target. An end on
  a different Part makes it the second Part.
- **Support: Pick footprint.** The footprint is centred under the click.
- **Aim direction** (tab and support). Click in the build plane; the
  direction is rounded to whole degrees.

On a planar face the clicked point and normal are exact: they are
projected onto the face's analytic plane. On a curved face they come from
the display triangle under the pointer, so the point may lie up to the
display tolerance inside the true surface.

The viewer draws the contact marker, the direction handle and the aid's
outline from the current Inspector values in every frame, and the
hovered pick with its normal before a click.

After Preview, the form shows the native inspection of the candidate:
whether the aid touches its target and how far it is, each crowded Part
with its distance, and the allowance findings. The viewer outlines each
crowded Part's bounds in red with its distance. For a tab, **Snap to
exact contact** uses the exact contact of the inspection; preview again
before Apply. With any placement finding, Apply is refused and the
reason is shown, while the preview stays visible. Cancel leaves the
document unchanged.

## Limits

- **No slicing.** There is no slicer, automatic support generation, support
  reliability guarantee, resin workflow or G-code output.
- **Shapes.** Aids are boxes. Supports are flat-topped, so under a sloped
  or curved underside the gap is the authored gap only at the lowest
  point. Supports are solid blocks, not lattices or trees.
- **Placement.** Aids are stored in world coordinates. Picks fill in
  those coordinates once; aids do not follow a target that moves, and
  only a support's height follows the target. Curved-face picks are
  display-accurate until snapped. A tab is placed at the target's lowest
  height, so a Part with a raised or chamfered base may need a manual
  contact. Picking works on Parts, not component occurrences.
- **Direction handle.** The direction is aimed by clicking in the build
  plane; there is no drag handle.
- **Wall inspection.** The inspection measures aids, not the finished
  part's own wall thickness.
- **Process records in the window.** The form chooses an existing process
  record; create or edit process records with `set_fdm_process`.
- **Instances.** Aids serve Parts. They are not created per Instance or
  occurrence. An aid on a component definition stays where it was modeled
  when the component moves to another pose, and the fit-clearance check
  covers only world Parts, not component occurrences.
