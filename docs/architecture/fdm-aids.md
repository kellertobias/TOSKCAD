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
  place, keeping its Part. A new aid takes its finished Part from the
  selection: one selected finished Part (or a feature of it). With nothing
  selected it takes the document's only finished Part. Several selected
  Parts, a selected print aid or a non-Part leave the choice open, and the
  form and status line say why; the tool never falls back to an unrelated
  first Part.
- **MCP.** `set_fdm_process`, `set_fdm_aid`, `remove_fdm_aid` and
  `fdm_aid_report`.
- **Undo.** Creating, editing and removing aids and processes are single
  transactions with Undo and Redo.

## Limits

- **No slicing.** There is no slicer, automatic support generation, support
  reliability guarantee, resin workflow or G-code output.
- **Shapes.** Aids are boxes. Supports are flat-topped, so under a sloped
  or curved underside the gap is the authored gap only at the lowest
  point. Supports are solid blocks, not lattices or trees.
- **Placement.** Aids are placed by world coordinates. They are not picked
  from faces or edges in the viewport, and they do not follow a target that
  moves; only a support's height follows the target.
- **Wall inspection.** The inspection measures aids, not the finished
  part's own wall thickness.
- **Process records in the window.** The form chooses an existing process
  record; create or edit process records with `set_fdm_process`.
- **Instances.** Aids serve Parts. They are not created per Instance or
  occurrence. An aid on a component definition stays where it was modeled
  when the component moves to another pose, and the fit-clearance check
  covers only world Parts, not component occurrences.
