# Walls, doors and windows

Walls are ordinary parametric model features. They live in the same canonical
document as every other feature, and the window and the MCP server use the same
core API. The code is `crates/multicad-core/src/model/wall.rs` (source,
planning and transactions) and `crates/multicad-model/src/walls.rs` (native
solid and plan symbols).

## Types and instances

- **Wall type.** An `architecture.wall_type` extension entity. It holds the
  thickness, height and justification, plus an optional material.
  Justification says which side of the drawn path the wall occupies: center,
  left or right, looking along the path.
- **Opening type.** An `architecture.opening_type` extension entity. It holds
  the kind, width, height and sill. The kind is either a window or a door; a
  door also records its hinge jamb (start or end along the segment) and its
  swing side (left or right of the path).
- **Wall.** An `architecture.wall` feature with one output Part. It owns:
  - its path: world X/Y points on the plane z = base, open or closed into a
    room;
  - a link to its wall type;
  - explicit thickness, height and justification overrides;
  - its openings. Each opening names its type, segment and offset (from the
    segment start to the start jamb), with optional width, height and sill
    overrides;
  - its vertical cutouts (see below).

### Vertical cutouts

A vertical cutout is a rectangular cut in one wall segment that runs the full
wall height, from the wall base to its top. It names its segment, offset
(from the segment start to its start side) and width.

- **Through cut.** Without a depth it removes the whole wall thickness: a
  full-height gap.
- **Chase.** With a depth it is cut into one face (left or right, looking
  along the segment) and the wall stays whole behind it, for example a
  service chase.

Cutouts belong to the wall, not to a type. A wall with cutouts makes the
project require the `architecture.wall_cutout.1` capability, so a build that
does not know cutouts refuses the file instead of dropping them.

**What it cuts.** A vertical cutout chooses what it cuts: this wall, the
floor slabs under it, or both. The slab part is a rectangular hole with the
same cutout identity in every slab it cuts, so one tool makes shafts and floor
penetrations too. See [vertical cutouts through slabs](#vertical-cutouts-through-slabs).

Every dimension can be a parameter expression.

### What a type edit changes

Editing a type updates every wall and opening that does not override the
changed value. The new wall Part takes the type's material when it is created.
A later material change on the type moves the Parts that still carry the
previous type material. A Part given another material keeps it.

The window and `prepare_wall_change` can create new wall and opening types in
the same transaction as the wall that uses them.

## Native solid

1. **Plan.** The plan builds one closed outline from the left and right face
   offsets.
   - An open wall gets one loop, with `wall.start` and `wall.end` caps.
   - A closed room gets an outer and an inner loop, whatever the path winding.
   - Interior joints are mitered.
   - Every edge has a stable role: `wall.seg.N.left`, `wall.seg.N.right`,
     `wall.start` or `wall.end`.
2. **Extrusion.** The kernel extrudes the outline to the wall height through
   the named-profile path, so faces carry those roles as provenance.
3. **Openings.** Each opening is subtracted as an exact prism. The prism spans
   the wall thickness plus 1 mm on each side. Its roles are
   `opening.<id>.sill`, `.jamb.start`, `.jamb.end` and `.head`.
   Each vertical cutout is a plan rectangle extruded from 1 mm below the base
   to 1 mm above the top. Its roles are `cutout.<id>.jamb.start` and
   `.jamb.end`, plus `.back` for the face at a chase's depth.
4. **Result check.** The result must be one connected solid.

### What the planner refuses

The planner reports these cases instead of building overlapping or invalid
geometry:

- a path that reverses on itself;
- a joint sharper than an 8 × thickness miter;
- a self-intersecting outline;
- a zero-length segment;
- an opening that does not fit its segment, keeping one thickness of wall at
  each joint;
- an opening that reaches the wall top;
- openings or cutouts that overlap on the same segment;
- an opening or cutout on a segment the wall does not have;
- a cutout deeper than the wall thickness, or without width or depth;
- a cutout that does not fit its segment, with the same joint margins as an
  opening;
- a through cutout that would split the wall: inside an open wall (a through
  cut touching an open end only shortens the wall), a second one in a closed
  room, or one removing the whole wall.

### Vertical cutouts through slabs

Slabs and pedestals carry rectangular holes (see [levels](levels.md#slab-holes)).
Each hole names the vertical cutout it belongs to. One cutout identity can
appear in one wall (as the record above) and in any number of slabs.

- **Wall and slabs.** The slab hole is the wall cutout's plan footprint: its
  span along the segment by its depth across the wall (the whole thickness
  for a through cut). It is stored on each slab as literal millimeters at
  placement time.
- **Slabs only.** A shaft or floor penetration needs no wall. It is placed by
  a plan rectangle: center, width, depth and rotation.
- **Which slabs.** A level range picks every floor slab (not pedestals) on
  the levels from the lower to the upper one whose outline the footprint
  touches. The API and MCP also accept an explicit slab list.
- **One transaction.** `prepare_vertical_cutout` rewrites the wall and every
  affected slab feature in a single transaction labeled "Place vertical
  cutout", "Edit vertical cutout" or "Remove vertical cutout". An edit makes
  the cutout cut exactly the given wall and slabs: it removes the hole from
  slabs it no longer cuts and the record from a wall it leaves.
  `plan_vertical_cutout` runs the same checks without staging.
- **Refused.** The change is refused, and nothing is staged, when:
  - it cuts nothing;
  - it names the same slab twice, or something that is not a slab;
  - a hole is not strictly inside its slab outline, meaning it crosses,
    touches or lies outside the edge, or wraps an inner corner of a concave
    slab (a cutout may not notch or split a slab);
  - a hole overlaps or touches another hole in the same slab;
  - the wall rules above are broken, for example a through cut that splits
    an open wall.
- **Capability.** Slabs with holes make the project require
  `architecture.slab_hole.1`.

## Plan representation

A drawing view whose frame looks straight down or up gets extra vectors for
each wall in the view, alongside the projected or sectioned solid:

- **Doors:** the leaf drawn open at 90° from the hinge on the swing-side face,
  plus a quarter-circle swing arc.
- **Windows:** two pane lines along the wall, plus a jamb line at each end.

The vectors carry the wall Part's identity. The Part's material supplies the
section hatch, so a horizontal section view gives a hatched floor plan with
door swings and window symbols. These symbols reach PDF and DXF through the
shared page render.

## Access

- **Window.** Model → Architecture → **Wall**. The Room tab entry opens the
  same tool. Preview builds the native wall, Apply commits it, and an applied
  wall reopens in the same form for editing. The wall form keeps the wall's
  vertical cutouts and says how many there are.
  - **Drawing the path.** The path is a corner list edited by the shared
    plan outline editor (`crates/multicad-ui/src/modeling/outline.rs`; the
    floor slab form uses the same editor). **Draw in viewport** clears the
    corners and takes clicks on the wall base plane (level elevation plus
    base elevation). Clicks snap, in this order, to the first corner
    (closes the room), to corners of this path and of other walls (path and
    face corners) and slabs, to orthogonal lines from the previous corner,
    and to a 10/50/100/500 mm grid (default 10 mm; each snap can be turned
    off). A dashed rubber band shows the pending segment's length and angle.
    **Next segment** appends a corner at a typed length and angle. Enter
    finishes an open path; Esc or **Cancel drawing** restores the previous
    corners. **Rectangle by dimensions** replaces the corners with a closed
    rectangle from a corner, width and depth.
  - **Adjusting.** While the form is open the viewport shows every segment
    with its length and a numbered handle per corner. Clicking a handle
    picks that corner up; the next click places it (with the same snapping)
    while dashed lines show both neighbouring lengths live; Esc cancels the
    move. Clicking a segment selects it; its length can then be typed (the
    end corner moves along the segment) or the segment split at its
    midpoint. The Inspector lists each corner's x and y as exact fields
    (millimeters or `=$parameter` expressions, kept untouched unless
    edited) and each segment's length; selection is shared between the
    viewport and the list. **Paste coordinates** still replaces all corners
    from `x, y` lines.
  - **Outline diagnostics.** Too few corners, zero-length segments, a path
    that doubles back and self-crossing segments are reported in the
    Inspector with the corner or segment to fix, and drawn in red in the
    viewport with a marker at the crossing point. The native planner stays
    authoritative at Preview.
- **Room tools.** Sketch → Room and Model → Architecture → **Door**,
  **Window** and **Vertical cutout** work on an existing wall; the vertical
  cutout also works on floor slabs alone. A wall Part selected in the tree is
  the starting wall, otherwise the first wall.
  - **What the cutout cuts.** The vertical-cutout Inspector has **Wall** and
    **Floor slabs** checkboxes and a **From level** / **To level** range
    (default: the wall's level only). With the wall unchecked it becomes a
    slab-only shaft: a click on a slab centers its rectangle there (snapped
    to 10 mm) and takes the slab's level. Center, width, depth and rotation
    can also be entered. The live note names the wall and slabs it will cut.
    Applying it is one transaction on the wall and every cut slab. Existing
    shafts appear in the same list for editing or removal.
  - **Placing.** Click a wall face in the viewport. The tool finds the
    segment under the click and centers the opening there, snapped to
    10 mm; an orange outline previews the footprint while hovering. The
    wall, segment and offset can also be entered in the Inspector.
  - **Type.** Doors and windows use an existing type of their kind or define
    a new one (with hinge and swing for doors), saved in the same
    transaction. A size override applies to this opening only. Cutouts take
    a width and either cut through or set a chase depth and face.
  - **Diagnostics.** The shared planner checks every input change. A fit
    note or the refusal reason is shown, and Preview stays disabled while a
    placement is refused.
  - **Apply.** Preview builds the wall natively; Apply commits one wall
    transaction labeled "Place door", "Place window", "Place vertical
    cutout", "Edit …" or "Remove wall opening". Undo restores the wall.
  - **Editing.** An existing door, window or cutout of the wall is chosen in
    the same tool's list to edit or remove it. Editing keeps its identity.
- **Core API.** `prepare_wall_opening` adds, replaces or removes one opening
  or cutout (`WallOpeningChange`), with any new types, through the same
  planner as a wall edit. `plan_wall_opening` runs the same checks without
  staging, and `locate_on_wall` maps a plan point to a segment and distance.
- **MCP:** `create_wall_type`, `edit_wall_type`, `create_opening_type`,
  `create_wall`, `edit_wall` (keeps cutouts), `inspect_wall`,
  `set_wall_opening`, `set_vertical_cutout` (wall, `slabs` or
  `from_level`/`to_level`, and `plan`), `remove_wall_opening`,
  `remove_vertical_cutout` and `inspect_vertical_cutout`. Each mutation is
  evaluated natively before commit.

## Limits

- **Joints and shapes.**
  - Walls are straight-segment polylines; there are no arc walls.
  - Separate walls do not join or clean each other up at T or L junctions.
    Draw connected runs as one path or room.
- **Openings.**
  - Openings are rectangular.
  - Door and window types carry no frame, leaf or glazing solids. The plan
    symbols are drawing graphics only.
  - Vertical cutouts are rectangular and run the full wall height. They get
    no plan symbol of their own; the plan shows them through the cut wall
    and slab solids.
  - A wall-and-slab cutout stores its slab footprint as numbers when it is
    placed. Editing the wall path or thickness later does not move the slab
    holes; reapply the cutout from the tool or MCP to resync them.
  - The window's Preview proves the rebuild of the anchor feature (the wall,
    or the first slab of a shaft). The other cut slabs are rebuilt by the
    same native evaluation but are not individually proven by the preview.
- **Room-tool picking.** The click is mapped through the wall's plan, so it
  works on any wall face; the outline preview is drawn on the wall's left
  face. Centering uses literal widths; a width written as an expression
  places the start side at the click.
- **Path editing in the window.** Corners are picked up and placed by
  clicks; there is no press-and-drag of a handle. A corner stored as an
  expression shows its formula; moving it in the viewport replaces it with
  a literal. Arc segments are not supported. The overlay is drawn on the
  base plane in any camera orientation; from a steep side view the plane is
  edge-on and clicks do not hit it.
- **Not certified.** These are geometric tools only; they carry no building
  code, accessibility or structural certification.
