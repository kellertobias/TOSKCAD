# Levels, slabs, stairs and service envelopes

Levels and everything placed on them live in the same canonical document as
every other feature. The window and the MCP server use the same core API. The
code is `crates/multicad-core/src/model/level.rs` (source, resolution and
transactions) and `crates/multicad-model/src/levels.rs` (native solids,
clearance checks and plan graphics).

## Source entities

- **Level.** An `architecture.level` extension entity with an elevation (the
  finished floor height above Z = 0), a floor-to-floor height up to the next
  level, and a default slab thickness.
- **Slab or pedestal.** An `architecture.slab` feature with one output Part.
  It names its level, a closed world X/Y outline and its kind.
  - A **floor slab** sits below the level's floor. Its thickness is the
    level's slab thickness unless the slab overrides it.
  - A **pedestal** rises above the floor by its own height, which is
    required.
- **Stair.** An `architecture.stair` feature with one output Part. It names
  its base level and optionally an upper level; without one it rises by the
  base level's floor-to-floor height. It also holds its form, the start of
  the first riser, the walking direction, the width, the riser count and the
  going. The forms are:
  - straight;
  - quarter turn, left or right, with a square landing at the turn;
  - half turn, left or right, with a landing spanning both flights.
- **Service envelope.** An `architecture.envelope` extension entity. It names
  a placed Part or Instance and holds front, back, left, right and top
  clearances plus a label. Front is the target's local −Y side and left its
  local −X side.

Every length can be a parameter expression.

### What a level edit changes

Walls, slabs and stairs measure their height from their level. Editing a
level's elevation or height rebuilds everything placed on it, and Undo
restores the exact document.

## Native solids

- **Slabs and pedestals** are the outline extruded to their thickness. The
  outline must have area and must not self-intersect. Holes are subtracted
  through the full thickness; their sides carry the roles
  `cutout.<id>.side.0`–`3`.
- **Stairs** are built flight by flight. Each flight is a stepped profile
  extruded across the stair width, and each piece reaches down to the base
  level so the stair is one solid. Landings are added for turning forms, and
  the union must be one connected solid.

### Slab holes

A slab or pedestal can carry rectangular holes, each with a center, width,
depth and rotation, all parameter expressions apart from the rotation. Each
hole names the vertical cutout it belongs to, so one shaft shares its
identity across the slabs of several levels and with a wall cutout. Holes are
placed and edited through the vertical-cutout operation (see
[walls](walls.md#vertical-cutouts-through-slabs)); a slab edit keeps them.

- A hole must lie strictly inside the outline, with slab material all
  around. One that crosses, touches or lies outside the edge, or wraps an
  inner corner of a concave outline, is refused. This also refuses a slab
  edit that would leave a hole outside the new outline.
- Holes in one slab may not overlap or touch.
- Slabs with holes require the `architecture.slab_hole.1` capability.

### Stair check

Each stair reports its total rise, riser height, going, step measure (2R + G)
and run. Notes flag figures outside common comfort ranges. These are
geometric notes only; they are not a building-code, accessibility or
structural approval.

### What is refused

The resolver reports these cases instead of building invalid geometry:

- a level with a non-positive height or slab thickness;
- a slab outline with fewer than three points, no area or a
  self-intersection;
- a pedestal without a height;
- a stair whose upper level is not above its base level;
- a stair with fewer than two risers, a turning stair with fewer than four,
  or a non-positive width or going;
- an envelope on anything other than a placed Part or Instance;
- an envelope box with no volume, reported when the clearances are checked.

## Service envelopes

The envelope box is the target's bounds in its own local frame, expanded by
the clearances. It moves and rotates with the target's placement.

- **Clearance check.** `Model::envelopes` intersects each box with every
  other final solid and reports overlaps above 1 mm³ by entity and volume.
- **Viewport.** After a full evaluation, the model snapshot carries each
  box's world corners. The viewport draws them as dashed orange boxes over
  the model, and the camera tools menu has a **Service envelopes** toggle.
- **Plan.** A drawing view looking straight down or up draws envelope
  outlines as dashed hidden lines. Stairs get a walking line with an arrow.

## Access

- **Window.** Model → Architecture → **Levels & stairs**. The Room tab
  entry opens the same tool. The Inspector chooses Level, Floor slab,
  Pedestal, Stair, Service envelope or Drawing guide.
  - An existing level or envelope can be chosen to edit it.
  - Applied slabs and stairs reopen from the model tree as feature edits
    that keep their Part.
  - The stair form shows its geometric check live.
  - **Slab outlines** use the same plan outline editor as wall paths (see
    [walls](walls.md#access)): draw on the level floor plane with snapping,
    rectangle by dimensions, corner and segment adjustment with live
    lengths, exact corner fields and red self-crossing diagnostics. A slab
    outline always closes; **Finish** closes it.
  - **Room boundary.** A floor can take its outline from exactly one closed
    wall chosen by name: its **Inside faces** (default), its **Wall path**
    (keeps the path's expressions) or its **Outside faces**. **Use room
    boundary** copies those corners into the outline and takes the wall's
    level. The chosen room's boundary is drawn as green dashes in the
    viewport; no other room is included. The copy is not linked: a later
    wall edit does not move the slab.
- **MCP:** `set_level`, `create_slab`, `set_stair`, `create_envelope` and
  `check_clearances`. Mutations are evaluated natively before commit.

Floor plans, sections and elevations linked to levels are authored as
[drawing guides](drawing-guides.md).

## Limits

- **Shapes.** Slab outlines are straight-segment polygons. Slab holes are
  rectangles.
  Stairs have straight flights only; there are no winders, spirals or
  railings.
- **Envelopes.**
  - Envelope boxes follow their target's local bounds and are axis-aligned
    in that frame.
  - A clash only reports an overlap; nothing moves.
  - Rotated Instances are covered by the pose math but were only exercised
    natively with placed Parts.
- **Window editing.** A stair start stored as an expression must be edited
  through the API, and there is no envelope edit from the model tree, only
  through the panel's existing-envelope choice.
- **Not certified.** These are geometric tools only; they carry no building
  code, accessibility or structural certification.
