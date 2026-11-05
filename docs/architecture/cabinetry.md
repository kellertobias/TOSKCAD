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
defined in Project → Materials or with `create_panel_stock`, and holds:

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
stock. The panel has one of four sources:

- **Rectangle.** A length (u) and width (v) on the XY, XZ or YZ plane, with
  its minimum corner at a world origin. The thickness runs along u × v: up,
  toward −Y or toward +X.
- **Sketch region.** A solved sketch region, extruded by the stock thickness
  along the sketch normal.
- **Carcass role.** One panel of a carcass, described below.
- **Wood Box face.** One of six optional faces of a canonical box recipe.

Each panel also records these values. Every dimension can be a parameter
expression.

- **Grain axis.** U, V or free. The cut list reports sizes along and across
  the grain. A free grain lets either orientation be cut.
- **Edge banding.** Each of the four rectangle edges can carry a band, with
  a thickness and an allowance. Banding is supported on rectangle and
  carcass and Wood Box panels only.
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

## Wood boxes

Model → Wood → Box opens a six-face assistant, initially an open-top five-panel
box. Choose shared panel stock (including its thickness and material), outside
width/depth/height and any nonempty subset of Left, Right, Front, Back, Bottom
and Top. The labeled projection resolves the same recipe used by native Preview.
Apply requires that exact successful preview; Cancel leaves the source unchanged.

Outside dimensions are the design envelope. Usable inner dimensions subtract
stock thickness for each present opposing face. Actual world bounds are the
bounds of the generated panels; a single bottom panel has only stock thickness
in Z even when the envelope is taller. The inspector and MCP distinguish these.
Sides span full depth/height, front/back fit between sides, and top/bottom fit
between the vertical faces. This butt policy avoids overlapping corner material.

The `cabinetry.wood_box` extension stores dimensions, placement, shared stock,
face presence, a group and reserved feature/Part identities for all six faces.
Generated ordinary panels use schema 2 with `PanelSource::WoodBox`; existing
rectangle/profile/carcass panels keep schema 1. Files require the
`cabinetry.wood_box.1` and `cabinetry.panel.2` capabilities. See
[ADR-0032](0032-editable-wood-box-panels.md) for the ownership contract.

Reopening a recipe, its group or a generated panel returns to the box assistant.
Retained faces keep their identities and downstream history. Dimension and stock
changes rebuild that history. Omission of a referenced face, or a resize that
invalidates a downstream operation, requires repair and leaves the document
unchanged. Grain, edge banding and oversize remain ordinary panel data.

The typed MCP tools are `preview_wood_box`, `set_wood_box`, `inspect_wood_box`
and `remove_wood_box`. Preview and Set require `expected_revision`, `name`,
`stock`, unit-bearing `width`, `depth`, `height`, and a nonempty `faces` list
using lowercase face names. Optional `origin` is three unit-bearing lengths;
`rotation` is an angle. An existing `recipe` retains omitted placement fields.
For new creation, reuse Preview's `recipe` and `box.identities` on Set to keep
the exact proposed UUIDs. Identities contain a `group` and six `panels` records
with `face`, `feature` and `part`; missing, duplicate or changed reserved IDs
are refused. Inspect accepts an optional recipe; omission lists all recipes.
Remove requires the recipe and expected revision.

Preview/Set return `revision`, `committed`, `recipe`, `box`, `panels` and
`native_geometry_checked`. The box includes its canonical `definition`,
`identities`, `outside_mm`, `usable_inner_mm`, `clear_origin_mm`,
`actual_world_bounds_mm`, `thickness_mm` and corner policy. Panel records include
face, identities, native final volume and solid count, including downstream cuts.
Removal returns a null box and no panels. Preview is audited as previewed and
has no committed revision. Stale revisions, cancellation, invalid stock/sizes,
empty or duplicate faces and dependency conflicts commit nothing.

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

### Editing a cabinet in the window

Selecting a cabinet's group, a carcass panel Part or a panel feature shows
**Edit cabinet "<name>"** in the Inspector
(`crates/multicad-ui/src/modeling/cabinet_edit.rs`). It reopens the saved
carcass definition — envelope, placement, construction, back, shelves and
front band — in the Panel / carcass form. Starting Panel / carcass from the
ribbon always drafts a new cabinet.

While the form is open:

- the viewport draws the planned envelope with width, height and depth
  dimensions and an outline and height for each shelf. The guide uses the
  same planner as Apply and is display only;
- the Inspector lists how many panels are kept, which roles are added and
  which panels are removed. A removed panel that a joint or machining still
  uses is named with that operation, and Preview refuses the edit until it
  is deleted or the panel is kept.

Preview builds the whole cabinet; Apply commits it as one undoable
"Edit carcass" transaction on the same carcass.

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

- **Wood groups.** Create offers Panel, Batten and Screw / Hardware. Modify
  offers Drill, Pocket and Joint. Assistants offers System 32 shelf pins,
  Cabinet and Box. These commands open the existing configuration forms.
- **Batten.** The member form offers solid rectangular profile stock, defined
  in Project → Materials and linked to the chosen timber material. It uses
  the same member source and native geometry as Tube / profile; hollow, round
  and angle stock are excluded from Batten creation. An applied member
  reopens in the shared member editor with its existing stock and identity.
- **Hardware.** Screw / Hardware offers the shared Hardware form with the
  canonical metric machine-screw preset and Browse Library, which opens the
  existing global browser. Washer, nut, screw and bearing choices reuse the
  same definitions as Mechanical; there is no Wood catalog. The menu explicitly
  identifies attached screw connections as unavailable in the native editor.
- **Window.** Model → Wood → Create → **Panel**. The form builds a carcass
  or places a rectangular panel; Preview builds the native solids and Apply
  commits them. Opening any carcass panel opens its carcass; opening a
  rectangle panel edits it in place.
- **Joints in the window.** Model → Wood → Modify → **Joint** joins panel
  pairs. Opening a joint cut reopens its joint to change the kind, count or
  fit. Its kind selector includes butt, miter, fingers, loose tenons (Domino)
  and paired dowels. The retained `domino` command opens the same form preset
  to loose tenons, seeded with up to two selected panels. Clicking panels
  on their visible faces fills panel A, then panel B, of the next pair; for
  loose tenons the viewport draws both panels, every mortise outline and
  each tenon's insertion direction, or the reason the values are refused.
- **MCP.** `create_panel_stock`, `set_carcass`, `create_panel`,
  `create_panel_stack`, `set_panel_joint`, `delete_panel_joint`,
  `set_hardware_pattern`, `set_machining`, `delete_machining` and
  `cut_list`.
- **Machining in the window.** Model → Wood → Modify → **Drill** or **Pocket**
  opens that operation in the shared machining form. Groove, rabbet and hardware
  patterns remain selectable there; `machining`, `groove`, `dado`, `rabbet` and
  `hardware-holes` retain generic form access. Model → Wood → Assistants →
  **System 32 shelf pins** opens the same form preset
  to shelf-pin rows on the inside faces. Hardware patterns are created
  through MCP for now.
- **Targets from the selection.** Drill and Pocket start with the selected
  rectangular panels as targets (a selected panel feature counts for its
  Part), Panel joint and Domino joint with the first two as panel A and B,
  and System 32 shelf pins with both sides of a selected carcass side. The
  status line names the targets taken and every selected item that is not
  a rectangular panel; nothing else is chosen for you. Targets can still
  be added or removed in the Inspector or the viewport before Apply.
- **Placing on faces.** While either form is open, a click on a visible
  rectangular panel face adds that panel as a target and takes its face. A
  drilling click adds a hole, or selects an existing hole so that the next
  click moves it; a pocket or hardware click sets its center; a groove
  click sets its offset; a rabbet click picks the nearest edge; a
  shelf-pin click moves the active row. Points snap to whole millimeters
  and to the face edges and center lines. The viewport draws each target's
  face outline and name, its u and v axes, the hovered point's offsets from
  the start edges, and the resolved holes and recess outlines; a target the
  current values refuse is outlined in red with the reason, and the hole or
  pocket under the pointer turns red with a note where it would leave the
  face. A selected hole shows its u and v in the Inspector, where it can be
  typed or deleted. The Inspector edits the same values, and an edit keeps
  the operation, its cut features and its targets.
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

## File capabilities

A file declares `cabinetry.shelf_pins.1` when any machining operation uses
shelf-pin rows, `cabinetry.machining_inside.1` when any operation enters on
the inside face, and `cabinetry.loose_tenon.1` when any joint stores the
loose-tenon kind. A build that predates these refuses the file by name
instead of failing to parse it. Files that use only grooves, rabbets,
pockets, top/bottom/edge drilling, hardware and corner joints keep the
earlier capabilities.

## Shipped cabinet demo

`examples/cabinet.tcd` is an editable 2000 × 800 × 500 mm
cabinet generated by `cargo run -p multicad-app --example cabinet_demo --
examples` through the same typed transactions as the window and MCP. The
generator also writes `examples/libraries/cabinet-hardware-library.tcd`, the
library its hardware comes from; `examples/examples.json` lists it in the
cabinet's `requires`, so the gallery copies it beside the user copy. Identities are fresh on each run, so regenerated
files match in content, not byte for byte.

Bottom to top:

- **Door compartment**, 800 mm clear: a left-hand and a right-hand
  full-overlay door (18 mm birch ply, 3 mm gaps) with one adjustable shelf
  on System 32 pins.
- **Drawers**: a 400 mm compartment with two equal openings. Each drawer has
  an 18 mm front and a 150 mm high box of 12 mm birch ply with five-finger
  corners (0.2 mm fit) and a 6 mm bottom in a 6.5 × 6 mm groove 8 mm up.
- **Open shelving** above the upper divider, with two adjustable shelves.

The carcass has full-height 18 mm sides, an 8 mm HDF back inset 10 mm and
two fixed dividers. The top, bottom and both dividers are joined to the
sides with Domino 8 × 22 × 40 mm loose tenons, three per joint (24 tenons
in eight tenon Parts).

### Hardware

The runners, hinges and plates are pinned symbols imported from the
hardware library file; their symbol identities are fixed, so a later
library revision updates them through Compare and an explicit update.
Hole patterns are cabinetry hardware patterns in the cabinet file.

| Item | Example specification | Drilling |
| --- | --- | --- |
| Runner, cabinet and drawer members | Side mount, full extension, 450 × 45 mm; each member 6.35 mm of the 12.7 mm side gap | Carcass side: 4 × Ø3 × 10 at 37/165/293/421 mm from the front; box side: 4 × Ø2.5 × 8 |
| Concealed hinge | 35 mm cup, 110°, full overlay | Door: Ø35 × 13 cup 22.5 mm from the hinge edge and 100 mm from the ends, 2 × Ø2.5 × 10 (45/9.5) |
| Hinge mounting plate | Cruciform, 0 mm | Side: 2 × Ø5 × 12 at 37 mm from the front, 32 mm apart |
| Shelf support pin | Ø5 × 16, 8 mm in the hole | System 32 rows Ø5 × 10, 37 mm from front and rear, 32 mm pitch |

These are generic example dimensions, not a compatibility claim for a
particular product.

### Movement

Each door and drawer is a reusable assembly placed once and driven by one
joint. A door turns on a revolute joint about a vertical axis on the carcass
front corner of its hinge side (0 to `door_max_angle`); a drawer slides out
along −Y on a slider (0 to `drawer_travel`). The runner drawer members and
the hinge cups and arms are members of these assemblies, so they move with
them; the runner cabinet members and hinge plates stay on the sides. The
saved poses are Closed, Doors open 90°, Drawers open and Doors and drawers
open; the file is saved Closed.

Limits of this representation:

- The pivot approximates the swing of a full-overlay door. A real concealed
  hinge moves on a four-bar path, and its arm does not stay on the plate as
  modeled here.
- Runners are two members; the intermediate member of a full-extension
  runner is not modeled, so a fully open drawer member no longer overlaps
  its cabinet member.
- Overlap checks cover the Closed and Doors and drawers open poses for the
  listed pairs (`crates/multicad-app/tests/cabinet_demo.rs`); intermediate
  positions are not sampled.

### Drawings

The drawing book has the assembly (C-101: front, side and top views with
width, height and depth dimensions), cutaway sections and joint details
(C-102: a horizontal section through the lower divider showing the
tenons, a vertical cutaway, a cropped Domino detail and the finger-jointed
drawer box), drilling details (C-103: the left side's inner face, the left
door with its hinges and a drawer box side), then the hardware schedule,
BOM, panel cut list and machining schedule, split into pages of at most 36
rows. Drawings show the current pose. Exploded drawing views have no
default producer, so the book uses cutaway sections; the viewport's
Exploded view tool shows an exploded illustration that is not saved.

### Parameters

`cabinet_width`, `cabinet_depth`, `cabinet_height`, `door_zone`,
`drawer_zone`, `door_max_angle` and `drawer_travel` drive the carcass,
fronts, boxes, drilling, hardware placements and joints. Drawer box depth,
runner length, shelf depth and the lower shelf-pin rows are fixed values; a
large change to `cabinet_depth` or `door_zone` can leave them out of
proportion and is not checked.
