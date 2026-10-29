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

A finger joint splits the block into `count` equal fingers along the corner.
Its `fit` is the total gap at each finger face, split between the two
panels. A miter between panels of different thickness follows the block
diagonal, so it is 45° only for equal thicknesses.

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
- **Deletion.** Deleting a joint removes its cuts and restores the panels.
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
  fit.
- **MCP.** `create_panel_stock`, `set_carcass`, `create_panel`,
  `create_panel_stack`, `set_panel_joint`, `delete_panel_joint` and
  `cut_list`.

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
    through-tenons.
  - A joint's panel pairs cannot be changed after creation.
  - Grooves, rabbets and drilling are CAD-75; back recesses are positions,
    not cut rabbets.
- **No outputs.** The cut list is a measured record, not a nesting, yield,
  machining or BOM output.
