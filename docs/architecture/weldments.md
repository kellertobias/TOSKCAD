# Weldment members, connections and welds

Weldments are built from ordinary model features in the canonical document.
The window and the MCP server use the same core API.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/weldment/` | Source types, section and member-frame resolution, connection geometry and transactions |
| `crates/multicad-model/src/weldment.rs` | Native member solids and end cuts, solved sketch paths and the member cut list |
| `crates/multicad-model/src/weld_schedule.rs` | Weld schedule, cut groups and weld-plan annotations |
| `crates/multicad-ui/src/modeling/weldment.rs` | The window's Tube / profile form |
| `crates/multicad-ui/src/modeling/weld_connection.rs` | The window's Miter / cope form |
| `crates/multicad-ui/src/modeling/weld_form.rs` | The window's Weld joint form and sequence order |
| `crates/multicad-mcp/src/tools/weldment.rs` | MCP tools |

## Profile stock

Members use stock library entries with a profile specification. The stock
is defined in Project → Materials or with `create_profile_stock`, and holds:

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

## Connections

A **connection** is a `weldment.connection` extension naming two member
Parts, a kind and an optional gap. It holds intent only. Whenever either
member is built, the evaluator derives that member's end cut from both
members' current resolved geometry. A path, section or anchor edit therefore
updates both cuts, and both members stay separate Parts.

### The joint

The joint point is where the two member paths meet: the closest points of
the two path lines must coincide within 0.001 mm. For each member the
evaluator decides whether the joint is at its start, at its end or inside
the member. Members are refused when:

- their paths are parallel;
- their paths pass each other without meeting (non-coplanar members);
- the joint lies beyond the end of a member;
- the members meet at less than 15°.

### Kinds

- **Miter.** Both members end at the joint. Each is cut on the plane that
  bisects the two member directions. A gap moves each cut back by half the
  gap along the plane normal. The cut is only accepted when the two placed
  sections are mirror images across that plane: same shape and size, and an
  anchor and rotation that mirror. Different widths, anchors or shapes are
  refused with a message to use a butt joint or cope. No half-angle formula
  is applied to sections that do not mirror.
- **Butt.** The first member terminates; the second continues.
  - The terminating member must end at the joint and be square to a flat
    face of a rectangular continuing member. It is cut square at that face,
    short by the gap.
  - When the continuing member also ends at the joint (a corner), its end is
    cut square at the far side of the terminating member, covering it.
  - Angled butt joints, and butt joints against round or angle members, are
    refused.
- **Cope.** The first member is the branch, the second the receiving
  member. The branch must end at the joint; the receiving member may pass
  through or end there. The branch is cut by:
  - the receiving member's actual outside (rectangular, round or angle),
    grown by the gap and swept over its length. The receiving bore is not
    used;
  - the plane through the receiving section center, square to the branch
    direction across the receiving axis.

  This gives a fish-mouth saddle for round tubes, at 90° or another angle,
  and a square notch against a flat face. A branch wider than the receiving
  member across both axes is refused.

Two connections may not cut the same end of a member, and two connections
may not join the same pair of members.

### Native cuts

A connected end starts long enough to pass the joint, then:

- a plane cut subtracts an exact half-space prism;
- a cope subtracts the grown envelope prism, then the half-space beyond the
  receiving center.

Cut faces carry the provenance role `connection.<id>.…` of their
connection. A cut that removes the whole member, or leaves it in separate
pieces, is refused.

## Welds and the weld sequence

Welds and sequences are documentation for drawings and schedules. Nothing
here is a welding procedure, and nothing computes distortion, strength,
heat input or structural safety.

### Welds

A **weld** is a `weldment.weld` extension attached to one connection. It
holds:

- a **mark**, its ID on drawings and schedules, for example "W1". Marks are
  unique in the document;
- the drawing weld **symbol** (fillet, square groove, V groove, bevel
  groove, plug, spot or seam) and the arrow, other or both **side**;
- an optional authored **size** and **length**; no length means the full
  joint. Both can be parameter expressions;
- **all-around** and **field** flags;
- a **process** annotation, for example "135 MAG";
- **notes** on access, welding position and orientation.

A connection can carry more than one weld.

### Sequence

A **sequence** is a `weldment.sequence` extension listing welds in the
order they are to be made, each with an optional step note. Reordering
replaces the steps. The welds keep their identities, so their connection
and member references are unchanged. A weld appears at most once in one
sequence and in at most one sequence.

### Weld schedule

`Model::weld_schedule` lists every weld: sequenced welds by sequence and
step, then the others by mark. Each record holds:

- the weld, its mark, sequence, step and step note;
- the connection, its kind, and both member Parts and names, in connection
  order;
- the **position**: where the member paths meet, resolved from the current
  members, so it follows path edits;
- the **joint perimeter** for plane cuts: the first member's outside
  perimeter on its cut plane. A round tube uses Ramanujan's ellipse
  approximation. Copes report none;
- the symbol, side, size, length, flags, process and notes.

### Weld plans on drawings

`weld_plan_annotations` turns the schedule into page annotations for one
drawing view, using the existing drawing weld symbol and table kinds:

- one weld symbol per weld, with its arrow at the joint projected into the
  view, its size and length, and a tail with the mark and process;
- a "Weld sequence" table with the step, mark, joint kind, members,
  symbol, size, process and notes.

The annotations are page content placed from the current schedule. They do
not follow later geometry edits; placing the plan again replaces the weld
symbols with current marks and earlier weld sequence tables.

## Member cut list

`Model::member_cut_list` lists every evaluated member Part. It measures the
Part's final solid in its member frame, so later features that change the
Part are included. Each record contains:

- the Part, feature, path, Material and stock;
- the designation, shape, section size and wall;
- the **length**: the overall length along the member axis, from the
  farthest point of one end to the farthest point of the other;
- the measured axial span of the solid from the path start;
- the start and end treatments: square, or the connection kind, its
  connection and partner member, whether this member terminates there, and
  the angle. Plane cuts give the cut-face angle from square (0 = square,
  45 = a right-angle miter); copes give the branch angle to the receiving
  axis;
- for an angled plane cut, the **long point**: the world direction from the
  axis toward the side where the cut end reaches farthest, and the section
  face it lies on (`+x`, `-x`, `+y` or `-y`, x being the width and y the
  height direction; a corner such as `+x-y` for a diagonal cut). Square ends
  and copes have none;
- the nominal length (path length plus extensions; connected ends are cut
  to their joint instead);
- the measured section size;
- the rotation, anchor and world axis, width and height directions;
- the volume and whether the length fits one stock bar;
- review issues: a measured section that differs from the stock, or a Part
  stock assignment that differs from the member stock.

`group_cuts` groups identical cuts: the same stock, an overall length
within 0.01 mm and the same end treatments within 0.01°, in either
direction. A member with two angled ends also needs the same long-point
relation: "same face" (a picture-frame rail), "opposite faces" (a
parallelogram) or "adjacent faces". The relation does not change when a
member is turned end for end. Each group has a quantity, its Parts and
names, the length, both end treatments with their long-point faces, and the
relation.

The MCP `weldment_cut_list` and `weld_schedule` tools return these
records. The member records, cut groups and weld schedule are the producer
contract for BOM, cut-list and weld schedule exports (CAD-89).

## Access

- **Window.** Model → Weld → **Tube / profile**. The form turns the solid
  lines of a chosen sketch into members, or places one member between two
  points. Preview builds the native solids and Apply commits them. Opening
  an applied member edits it in place; a sketch-line member keeps its line.
- **Window.** Model → Weld → **Miter / cope**. Choose the kind, the two
  members and the gap. An existing connection can be picked from the form
  and edited.
- **Window.** Model → Weld → **Weld joint**. Attach a weld to a
  connection, or reopen one; switch to Sequence to order the welds with Up,
  Down, Remove and Add. Weld plans are placed through MCP for now.
- **MCP.** `create_profile_stock`, `create_members`, `edit_member`,
  `connect_members`, `weldment_cut_list`, `set_weld`, `set_weld_sequence`,
  `weld_schedule` and `place_weld_plan`.

## Limits

- **Paths.** Straight sketch lines and point pairs only; no model edges,
  arcs or 3D sketches. A sketch-line path is solved without external
  reference geometry, so a path sketch that projects model edges is not
  yet supported.
- **Sections.** Rectangular, round and angle sections with sharp corners;
  no channels, beams or custom profile sketches.
- **Path changes in the window.** A sketch-line member cannot be moved to
  another line from the form; edit the sketch or recreate the member.
- **Connections.**
  - Only coplanar members whose paths meet are connected. Offsets between
    paths, and members that cross without meeting, are not supported.
  - Butt joints need a square, flat face. There are no angled butt joints
    and no butt joints against round or angle members.
  - Miters need mirroring sections; unequal-width miters are refused.
  - A cope's envelope covers the receiving member's nominal length, not its
    own end cuts.
  - Connections are picked in the form, not in the viewport.
- **Welds.**
  - Weld positions are joint points, not modeled weld beads; there is no
    weld bead geometry or filler mass.
  - Joint perimeters are computed for plane cuts only, not for copes.
  - Placed weld plans are static page content; the window has no Place
    weld plan command yet, and the ribbon's Weld sequence entry remains a
    placeholder (the sequence is ordered in the Weld joint form).
  - No weld schedule, BOM or cut-list export files; those are CAD-89.
- **Not a structural tool.** Members, records and lengths are geometric
  aids. They are not structural, strength or code checks.
