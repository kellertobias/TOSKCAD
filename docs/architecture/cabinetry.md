# Cabinetry panels, carcasses and stacks

Cabinetry is built from ordinary model features in the canonical document.
The window and the MCP server use the same core API.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/cabinetry/` | Source types, panel and carcass resolution, cut dimensions and transactions |
| `crates/multicad-model/src/cabinetry.rs` | Native panel solids, the cut list and stack checks |
| `crates/multicad-ui/src/modeling/cabinet.rs` | The window's Panel / carcass form |
| `crates/multicad-mcp/src/tools/cabinetry.rs` | MCP tools |

## Panel stock

Panels use stock library entries with a panel specification. The stock is
defined in Model → Materials or with `create_panel_stock`, and holds:

- the thickness;
- the sheet length and width;
- the grain relative to the sheet length;
- its manufacturing Material.

The panel solid takes its thickness from the stock, so a stock thickness
edit rebuilds every panel that uses it. The Part's acknowledged stock
assignment then shows the changed-content review diagnostic from
[manufacturing properties](manufacturing-properties.md).

## Panels

A **panel** is a `cabinetry.panel` feature with one output Part. A new panel
Part gets the stock's Material and a manufacturing-properties record of its
stock. The panel has one of three sources:

- **Rectangle.** A length (u) and width (v) on the XY, XZ or YZ plane, with
  its minimum corner at a world origin. The thickness runs along u × v: up,
  toward −Y or toward +X.
- **Sketch region.** A solved sketch region, extruded by the stock thickness
  along the sketch normal.
- **Carcass role.** One panel of a carcass, described below.

Each panel also records these values. Every dimension can be a parameter
expression.

- **Grain axis.** U, V or free. The cut list reports sizes along and across
  the grain. A free grain lets either orientation be cut.
- **Edge banding.** Each of the four rectangle edges can carry a band, with
  a thickness and an allowance. Banding is supported on rectangle and
  carcass panels only.
- **Oversize.** Extra core material added to both cut dimensions.

The four edge faces of a rectangular panel carry the provenance roles
`panel.u_start`, `panel.u_end`, `panel.v_start` and `panel.v_end`.

### Finished and core sizes

The modeled solid is the finished panel, banding included. The core is the
board cut from stock:

> core = finished − band thickness + band allowance (for each banded edge
> across that dimension) + oversize

For example, an 800 × 400 mm panel has:

- 1 mm bands on both u edges;
- a 2 mm band with a 1 mm allowance on one v edge;
- a 2 mm oversize.

Its core is 800 × 401 mm.

A core fits the stock when it fits on one sheet. When both the panel grain
and the stock grain are set, the along-grain size must run along the stock
grain.

## Carcasses

A **carcass** is a `cabinetry.carcass` extension holding:

- **Placement.** The front-left-bottom corner and a rotation about +Z.
  Unrotated, the width runs along +X and the depth along +Y, so the front
  faces −Y.
- **Size.** The outside width, height and depth.
- **Panel stock** for the sides, top, bottom and shelves.
- **Construction.** Either full-height sides with the top and bottom
  between them, or a full-width top and bottom with the sides between them.
- **Back.** One of:
  - none;
  - inset: captured between the sides, top and bottom, a recess in from the
    rear;
  - overlay: across the rear edges.

  An inset or overlay back has its own stock.
- **Shelves.** Either a count with equal interior gaps, or explicit offsets
  from the interior bottom to each shelf underside. Shelves also have a
  front setback and a clearance at each end.
- **Front band.** An optional band on the front edges of the sides, top,
  bottom and shelves.
- **Group.** An optional model group holding the panels.

Creating a carcass creates one named panel Part per role, for example
"Base cabinet left side" or "Base cabinet shelf 2". Each panel feature links
to the carcass, so a carcass or parameter edit resizes the panels in place.

Carcass panels have fixed grain:

- the sides and back run along their height;
- the top, bottom and shelves run along the width.

### Carcass edits

Editing a carcass keeps every panel whose role remains, with the same Part
and feature identities.

- **Removed roles.** Shelves or a back that are no longer needed are
  removed, together with their manufacturing records and group membership.
  This happens in the same transaction.
- **New roles.** New shelves or a new back are created in the carcass group.
- **Stock changes.** A panel follows its new stock in both its Material and
  its stock assignment.

### What the planner refuses

The planner reports these cases instead of building overlapping or invalid
panels:

- a carcass that is not wider and taller than two panel thicknesses;
- a back recess and thickness that leave no depth in front;
- an overlay back thicker than the cabinet depth;
- even shelves that leave no interior gap;
- an offset shelf outside the interior;
- an offset shelf that overlaps the shelf below;
- a setback or clearance that leaves no shelf.

## Stacks

A **stack** is a `cabinetry.stack` extension listing panel Parts in
stacking order, with an optional adhesive name. Each layer keeps its own
Part, Material, stock and grain. A Part can be a layer of only one stack.

With grouping, the layers move into a new model group named after the
stack. Grouping requires the layers to share one container.

The native evaluator checks each stack:

- the layers are parallel to the first layer;
- consecutive layers touch face to face in the listed order, within
  0.001 mm;
- consecutive layers overlap in plan.

A stack whose layers have moved apart is reported, not regrouped.

## Joints

A **joint** is a `cabinetry.joint` extension with a kind and one or more
pairs of rectangular panel Parts (A and B). All pairs share the kind.

### The corner block

The two panels of a pair must overlap at a right-angle corner. Both panels
reach the outside corner, so they share a **corner block**: the region
inside both finished panels.

- The block must be one full thickness of each panel. Its length runs along
  the corner.
- Each panel must continue past the block on its own side.

### Kinds

| Kind | Panel A | Panel B |
| --- | --- | --- |
| Butt, A through | Keeps the whole block | Loses the block and is shortened by A's thickness |
| Butt, B through | Loses the block and is shortened by B's thickness | Keeps the whole block |
| Miter | Keeps the half beside its body | Keeps the other half; the split runs from the outside corner to the inside corner |
| Fingers | Keeps fingers 1, 3, 5… | Keeps fingers 2, 4… |
| Loose tenon | Mortises in its face or end | The matching mortises; see below |

A finger joint splits the block into `count` equal fingers along the corner.
Its `fit` is the total gap at each finger face, split between the two
panels. A miter between panels of different thickness follows the block
diagonal, so it is 45° only for equal thicknesses.

### Loose tenons (Domino style)

A loose-tenon joint joins panels that meet **face to edge**: one panel's
edge face lies flat on the other panel's top or bottom face, as a cabinet
top between two sides does. The joint finds which panel of each pair
offers the face and which the edge, so pair order does not matter. It
needs no corner block.

Each pair takes `count` tenons, evenly spaced along the contact between an
`inset` from each end, centered across the edge panel's thickness unless an
`offset` moves them. A tenon is a rounded slot shape: its `thickness` is
the cutter diameter, its `width` runs along the joint and includes the
rounded ends, and its `length` runs from mortise to mortise.

- The **face mortise** goes `face_depth` into the face panel.
- The **end mortise** goes `edge_depth` into the edge panel's end.
- Each mortise is the tenon section lengthened by its own play
  (`face_play`, `edge_play`; 0 is tight) and thickened by `fit`.
- The tenon sits centered on the contact plane in the depth the mortises
  leave over: with 15 + 28 mm mortises and a 40 mm tenon, 1.5 mm clearance
  at each end.

Creating the joint adds a joint cut to each panel and one more Part per
pair, named "<joint> tenons", built by a `cabinetry.loose_tenon` feature
from the same resolution. A panel or carcass change therefore rebuilds both
mortises and the tenons together. Opening a tenon Part reopens its joint.

The example values are a Festool DOMINO 8 × 22 × 40 mm tenon, cut with the
8 mm DF 500 cutter: 15 mm into an 18 mm face (leaving 3 mm), 28 mm into the
mating end, tight in the end mortise and with 6 mm play along the face
mortise, two per joint 60 mm from the ends. The tenon size comes from
Festool's published tenon range; the depths and plays are this project's
example choices. Neither is a tested compatibility claim for a machine or
cutter.

A loose-tenon joint refuses, before staging anything:

- panels that do not meet face to edge;
- a face mortise that leaves less than 2 mm of the face panel;
- an end mortise as deep as the edge panel is long there;
- mortises shallower in total than the tenon is long;
- a tenon or mortise that leaves the face or the end, including through
  insets, offsets or a short contact;
- tenons that overlap along the joint;
- a width smaller than the thickness, and 0 or more than 32 tenons.

An edit changes the tenon, spacing, depths and fits; it cannot turn a
loose-tenon joint into a corner joint or back.

### Cut features

Creating a joint adds one `cabinetry.joint_cut` feature per panel and pair,
in one transaction. Each cut:

- removes the other panel's share from its own Part;
- builds on the Part's previous head, so a panel can take part in several
  joints.

Each cut depends on the joint, and the joint depends on both panel
features. A change to either panel or its stock therefore rebuilds both
sides. The cut list then measures the changed solids, so a butt-joined panel
reports its shortened finished size.

- **Edits.** Editing a joint changes its kind, finger count or fit for every
  pair. Its pairs are fixed when it is created.
- **Deletion.** Deleting a joint removes its cuts and any tenon Parts, and
  restores the panels.
  A later joint cut on the same Part is re-attached to the feature before
  it; any other later feature must be removed first.
- **Removed panels.** A carcass edit refuses to remove a panel that a joint
  uses.

### What a joint refuses

A joint refuses these cases before staging anything:

- panels that do not overlap;
- parallel panels;
- a block that is not one full thickness of each panel;
- a panel that ends inside the block;
- sketch-region panels;
- a finger count outside 2–128;
- fingers no wider than the fit plus 0.5 mm.

A cut that would split a panel is refused by the native evaluator. Stock or
position changes that break a corner make the model fail to evaluate with
the same reason; they are not silently mis-cut.

## Machining

A **machining operation** is a `cabinetry.machining` extension that targets
one or more rectangular panel Parts. Each target gets one
`cabinetry.machining_cut` feature, chained onto its history like a joint
cut. Positions are in the panel's face coordinates, so an operation follows
its panel when the panel moves or is resized.

### Faces

- **Top.** The face at the full thickness, cut downward.
- **Bottom.** The panel plane, cut upward.

Both faces use the panel's (u, v) coordinates. Drilling can also enter an
**edge face**. There the first coordinate runs along the edge from its
start, and the second up the thickness from the bottom face.

- **Inside.** Each target's top or bottom face that looks into the
  cabinet: toward the carcass interior for a carcass panel, otherwise
  toward the other targets. A carcass left side is cut on its top face and
  the right side on its bottom face, so one operation reaches both inner
  faces. A lone non-carcass target has no inside face and is refused.

### Kinds

| Kind | Defined by |
| --- | --- |
| Groove or dado | Runs the full panel length along u or v, at an offset from the start edge of the other axis, with a width and depth |
| Rabbet | Along one edge, with a width and depth |
| Pocket | A rectangle with a center, size and depth |
| Drilling | Holes of one diameter, blind with a depth or through |
| Shelf pins | System 32 style rows of blind holes, described below |
| Hardware | A placed hardware pattern, turned in quarter turns |

All dimensions can be parameter expressions. Grooves, rabbets, pockets and
shelf pins are cut into the top, bottom or inside face only.

### System 32 shelf pins

A shelf-pin operation holds:

- the axis the rows run along (for a carcass side, v runs up the height);
- up to eight rows, each an offset from the start or end edge across them;
- the pitch, 32 mm by default;
- the first hole center from the start edge and an end limit, the closest
  a hole center may come to the end edge;
- an optional fixed hole count per row; without it the row fills the range;
- the hole diameter and blind depth.

The tool's example settings are Ø5 × 10 mm holes in rows 37 mm in from
the front and rear edges, from 100 mm above the bottom edge to 100 mm
below the top edge. These are editable project choices, not a claim of
compatibility with particular shelf supports or line-boring machines.

Targeting both sides of a carcass with the inside face drills the same
rows into each inner face. The rows are measured from each side's own
edges, so they stay aligned while both sides share their size and
position, and they rebuild when the carcass changes.

`shelf_support_offsets` lists the shelf levels of a carcass: for every hole
height both sides share, the offset from the interior bottom to a shelf
underside that rests on a pin of the hole diameter, half a diameter above
the hole center. Levels without room for a shelf below the top are left
out. A carcass shelf set to one of these offsets rests on the pins.

### Hardware patterns

A **hardware pattern** is a reusable `cabinetry.hardware` extension. It
holds hole positions, diameters and blind or through depths, plus
per-material allowances. An allowance adds to the diameter and blind depth
in panels whose stock has that Material. Each placement links to its
pattern, so editing the pattern rebuilds every placement.

### Holes from other parts

Holes that come from other parts use the general transfer engine
([associative feature transfer](0021-associative-feature-transfer.md)), not
a cabinetry copy. A drawer rail's screw holes, for example, reach a cabinet
side panel this way. A panel used as an assembly component definition stays
in the cut list.

### Edits and refusals

Editing an operation changes its kind, sizes or face, or suppresses it. Its
targets are fixed when it is created. A suppressed operation keeps its cuts
in each Part's history but removes no material and reports no holes.
Deleting an operation removes its cuts, re-attaching later cuts on the same
Part.

The resolver refuses:

- a blind groove, rabbet, pocket or hole as deep as the material there;
- a groove that does not fit across the panel;
- a rabbet that leaves no material;
- a pocket or hole that leaves its face;
- a groove, rabbet, pocket or shelf-pin row on an edge face;
- holes of one operation that overlap each other;
- a hole that overlaps a hole of another operation on the same face of the
  same panel;
- shelf-pin limits that leave no hole, or fewer holes than a fixed count;
- more than 256 holes on one panel.

These are modeled solids, not CNC toolpaths, tool-access checks or
machining certification.

### Machining records

`Model::machining` lists every machining cut with:

- the Part, operation, kind, the face actually cut (inside resolved to top
  or bottom), Material and stock;
- each hole's face coordinates, world center, axis into the material,
  diameter, depth and whether it goes through;
- each groove, rabbet or pocket's size and depth;
- the volume measurably removed from the solid before the cut.

Loose-tenon joints add one record per panel and pair, of kind
"loose-tenon mortise": the face or end it is cut into, each mortise's
length, thickness and depth, and the tenon Part it receives.

The MCP `cut_list` tool includes these records, and each cut record lists
its panel's operations.

## Cut list

`Model::cut_list` lists every evaluated panel Part. It measures the Part's
final solid in its panel frame, so later features that change the Part are
included. Each record contains:

- the Part, carcass and role, stack, Material and stock;
- the finished and core sizes along and across the grain, and the
  thickness;
- the banded edges, the oversize and the volume;
- whether the core fits one stock sheet;
- review issues: a measured thickness that differs from the stock, or a
  Part stock assignment that differs from the panel stock.

`Model::stacks` returns each stack's layers, its total thickness and its
contact findings. The MCP `cut_list` tool returns both lists. This list is
the producer contract for BOM and cut-list exports (CAD-89).

## Access

- **Window.** Model → Wood → **Panel / carcass**. The form builds a carcass
  or places a rectangular panel; Preview builds the native solids and Apply
  commits them. Opening any carcass panel opens its carcass; opening a
  rectangle panel edits it in place.
- **Joints in the window.** Model → Wood → **Panel joint** joins panel
  pairs. Opening a joint cut reopens its joint to change the kind, count or
  fit. Model → Wood → **Domino joint** opens the same form preset to loose
  tenons, seeded with up to two selected panels. In both, clicking panels
  on their visible faces fills panel A, then panel B, of the next pair; for
  loose tenons the viewport draws both panels, every mortise outline and
  each tenon's insertion direction, or the reason the values are refused.
- **MCP.** `create_panel_stock`, `set_carcass`, `create_panel`,
  `create_panel_stack`, `set_panel_joint`, `delete_panel_joint`,
  `set_hardware_pattern`, `set_machining`, `delete_machining` and
  `cut_list`.
- **Machining in the window.** Model → Wood → **Drill / pocket**, and
  Model → Wood → **System 32 shelf pins**, which opens the same form preset
  to shelf-pin rows on the inside faces. Hardware patterns are created
  through MCP for now.
- **Placing on faces.** While either form is open, a click on a visible
  rectangular panel face adds that panel as a target and takes its face. A
  drilling click adds a hole, or selects an existing hole so that the next
  click moves it; a pocket or hardware click sets its center; a groove
  click sets its offset; a rabbet click picks the nearest edge; a
  shelf-pin click moves the active row. Points snap to whole millimeters
  and to the face edges and center lines. The viewport draws each target's
  face outline and name, its u and v axes, the hovered point's offsets from
  the start edges, and the resolved holes and recess outlines; a target the
  current values refuse is outlined in red with the reason. The Inspector
  edits the same values.
- **MCP shelf pins.** `set_machining` with `kind: "shelf_pins"` and
  `face: "inside"`; its result lists each carcass's `shelf_supports`.

## Limits

- **Panel sources.**
  - Panels cannot yet be made from selected face boundaries.
  - Sketch-region panels can be created and edited through the API only,
    not the window form, and they carry no banding.
- **Parametric limits.**
  - The shelf count and offsets are carcass values. Parameter expressions
    can drive sizes and offsets, but a parameter cannot change the number of
    shelves.
  - Carcass grain follows the fixed rule above.
- **Stacks in the window.** The form does not create stacks; use
  `create_panel_stack`.
- **Joint geometry.**
  - Joints need square, overlapping rectangular panels.
  - There are no angled (non-90°) corner miters, T-joints, dados or
    through-tenons. Loose tenons need a face-to-edge contact; edge-to-edge
    (panel widening) joints are not supported.
  - A joint's panel pairs cannot be changed after creation.
  - A carcass back recess is a position; cut a rabbet or groove for it
    with a machining operation.
- **Machining geometry.**
  - Grooves run the full panel length; there are no stopped grooves.
  - Pockets have square corners, with no tool radius.
  - There are no countersinks or counterbores, and no tool-access or
    collision checks.
- **No outputs.** The cut list is a measured record, not a nesting, yield,
  machining or BOM output.
