# Weldment members

Weldments are built from ordinary model features in the canonical document.
The window and the MCP server use the same core API.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/weldment/` | Source types, section and member-frame resolution, and transactions |
| `crates/multicad-model/src/weldment.rs` | Native member solids, solved sketch paths and the member cut list |
| `crates/multicad-ui/src/modeling/weldment.rs` | The window's Tube / profile form |
| `crates/multicad-mcp/src/tools/weldment.rs` | MCP tools |

## Profile stock

Members use stock library entries with a profile specification. The stock
is defined in Model → Materials or with `create_profile_stock`, and holds:

- the designation, for example "SHS 40×40×2";
- the section shape:
  - **rectangular**: a square or rectangular hollow section (SHS/RHS) when
    it has a wall, otherwise a solid flat or square bar;
  - **round**: a circular hollow section (CHS) when it has a wall,
    otherwise a round bar. Width and height are both the outside diameter;
  - **angle**: an L section. The wall is the leg thickness and is required;
- the section width and height;
- the wall, optional except for angles;
- the bar length the stock is supplied in;
- its manufacturing Material.

The shape defaults to rectangular and is omitted from stored stock when
rectangular, so stock written before shapes existed is unchanged.

Sections are modeled with sharp corners. Real tube corner radii and angle
root radii are not modeled, so volumes and masses are slightly high.

## Members

A **member** is a `weldment.member` feature with one output Part. Members
of one frame are separate Parts; they are never fused into one body. A new
member Part gets the stock's Material and a manufacturing-properties record
of its stock, like a cabinetry panel.

### Paths

Each member follows one straight path:

- **Sketch line.** One solid (non-construction) line of a constrained
  sketch, named by its stable curve identity. The member follows the line
  through sketch edits. Arcs, circles,
  splines and construction lines are refused.
- **Point pair.** World start and end points. Each coordinate can be a
  parameter expression.

Model edges cannot yet be used as member paths.

### Member frame

- The **axis** runs from the path start to its end.
- The section **height** (y) starts along a reference direction projected
  square to the axis:
  - the sketch normal for a sketch-line member;
  - world +Z for a point-pair member, or +Y when the member is vertical.
- The section **width** (x) is y × axis.
- A **rotation** turns the section about the axis, counterclockwise looking
  from the path end back toward its start.

### Placement

- **Anchor.** Which point of the section bounding box lies on the path,
  chosen separately across the width and the height: the minimum edge, the
  center or the maximum edge. Section coordinates put the bounding box at
  [0, width] × [0, height]; an angle's heel is the origin, with its legs
  along +x and +y. The default is centered.
- **End extensions.** Extra length beyond the path start and end. A
  negative value trims. Extensions that remove the whole member are
  refused.

### Native solid

The evaluator sweeps the exact section along the axis over the member
length. Hollow sections keep their bore as an inner boundary, so the solid
has the real wall. Profile edges carry the provenance roles
`member.outer.N` and `member.inner.N`.

## Frames

A frame transaction turns every solid line of a sketch into its own member
Part, named "<prefix> 1", "<prefix> 2" and so on in sketch order. One
template supplies the stock, anchor, rotation and extensions. With a group
name, the members are held by a new model group.

Editing the sketch moves and resizes its members in place. Their feature
and Part identities stay, so later connections, welds, BOM rows and
drawing references keep pointing at the same members.

## What the planner refuses

The planner and evaluator report these cases instead of building wrong
geometry:

- stock that is not profile stock;
- round stock whose width and height differ, and angle stock without a
  wall;
- a zero-length path;
- end extensions that trim away the whole member;
- a construction line, arc, circle or spline as a path;
- a path line that no longer exists in its sketch;
- two members on the same sketch line.

## Member cut list

`Model::member_cut_list` lists every evaluated member Part. It measures the
Part's final solid in its member frame, so later features that change the
Part are included. Each record contains:

- the Part, feature, path, Material and stock;
- the designation, shape, section size and wall;
- the **length**: the overall length along the member axis, from the
  farthest point of one end to the farthest point of the other;
- the nominal length (path length plus extensions);
- the measured section size;
- the rotation, anchor and world axis, width and height directions;
- the volume and whether the length fits one stock bar;
- review issues: a measured section that differs from the stock, or a Part
  stock assignment that differs from the member stock.

The MCP `weldment_cut_list` tool returns these records. They are the
producer contract for BOM and cut-list exports (CAD-89).

## Access

- **Window.** Model → Weld → **Tube / profile**. The form turns the solid
  lines of a chosen sketch into members, or places one member between two
  points. Preview builds the native solids and Apply commits them. Opening
  an applied member edits it in place; a sketch-line member keeps its line.
- **MCP.** `create_profile_stock`, `create_members`, `edit_member` and
  `weldment_cut_list`.

## Limits

- **Paths.** Straight sketch lines and point pairs only; no model edges,
  arcs or 3D sketches. A sketch-line path is solved without external
  reference geometry, so a path sketch that projects model edges is not
  yet supported.
- **Sections.** Rectangular, round and angle sections with sharp corners;
  no channels, beams or custom profile sketches.
- **Path changes in the window.** A sketch-line member cannot be moved to
  another line from the form; edit the sketch or recreate the member.
- **Not a structural tool.** Members, records and lengths are geometric
  aids. They are not structural, strength or code checks.
