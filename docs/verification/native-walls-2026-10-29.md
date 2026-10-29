# Native walls, doors and windows, 2026-10-29

These checks record agent verification, not human acceptance.

## Source and build

The checks cover these commits:

| Commit | Content |
| --- | --- |
| `0863e90` | Core types, walls and the native evaluator |
| `4cff999` | MCP tools |
| `826d3a3` | One transaction for new types and the wall |
| `1fba83c` | The window's Wall tool |

They used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline` | 1,386 passed, 0 failed, 2 existing ignored |
| `npm test` | 25 of 25 passed |
| `./build` | Passed |

The build wrote `artifacts/development/build-2SyDhF/MultiCAD.app`. The SHA-256
of its executable is `62fad0eac4e4283e27e8c9b2f190a720c0959b75a00c9cd68a0dcf9302b277f6`.

## Evidence

### Core planning: `crates/multicad-core/src/model/wall.rs`

These are unit tests of the planner.

- **Mitered L wall.** The outline has six edges with stable roles. Its area is
  7000 × 200 mm², and its corners are at (4100, −100) and (3900, 100).
- **Closed rooms.** Both windings give the same outer area (5200 × 4200 mm²)
  and inner area (4800 × 3800 mm²). A left-justified room built inward has an
  outer area of 5000 × 4000 mm².
- **Refused paths.** Reversing paths, a 3000 → 100 mm hairpin miter,
  self-intersections and zero-length segments are all diagnosed.
- **Refused openings.** Openings that don't fit, reach the wall top or overlap
  are refused.
- **Door symbol.** The door leaf runs from (500, 100) to (500, 1000), with a
  900 mm quarter arc.
- **Single transaction.** A wall and its new wall and door types are created in
  one transaction. A later edit that no longer fits is refused.

### Native model: `crates/multicad-model/tests/walls.rs`

- **L wall with openings.** An L-shaped concrete wall with a 900 × 2100 door
  and a 1200 × 1200 window at sill 900:
  - is one solid of 7000 × 200 × 2500 mm³ minus both openings;
  - has bounds (0, −100, 0) to (4100, 3000, 2500);
  - has face roles `wall.seg.0.left`, `wall.start`, `opening.<id>.head` and
    `opening.<id>.jamb.start`;
  - carries the type's material on its Part.
- **Type edit.** Changing the type to 250 × 2700 rebuilds the L wall, and Undo
  restores the exact document.
- **Override kept.** A second wall overriding its height at 3000 keeps that
  height through the type edit, while its thickness follows the type.
- **Opening edit.** Widening the door to 1000 rebuilds the cut.
- **Refused edits.** An opening that doesn't fit, reaches the top or overlaps
  is refused, and the source is unchanged.
- **Material follows the type.** After the type's material changes to Brick,
  the wall Part carries Brick.
- **Save and reopen.** The `architecture.wall.1` capability is recorded. The
  reopened document is identical and has an identical volume.
- **Closed room.** A room with a door and a window has a volume of
  (5200 × 4200 − 4800 × 3800) × 2500 mm³ minus both openings.
- **Plan section.** A plan section cut at 1 m:
  - has section and hatch vectors;
  - has a 900 mm quarter swing arc at (1000, 100);
  - has two 1200 mm window pane lines on segment 2.

### MCP: `crates/multicad-mcp/tests/walls.rs`

- **Creation.** The tests create a wall type, a door type and a wall with a
  door. The wall is evaluated natively: one solid of the expected volume at
  revision 3.
- **Refused edit.** An opening that doesn't fit is refused with "does not fit"
  and the document is unchanged.
- **Edits.** Lengthening the wall rebuilds it. A type edit to 250 mm rebuilds
  the wall, and `evaluate_model` confirms the volume.
- **History.** The type edit is attributed to `mcp:walls`.

### Window: `crates/multicad-ui/src/modeling/wall.rs`

These are unit tests of the panel.

- **New project.** On an empty project, the panel defines a new wall type and
  new door and window types. Its command creates three types and the wall in
  one transaction, and the native volume matches.
- **Edit.** Reopening the applied wall reuses the saved types. A height
  override of 3000 rebuilds natively.
- **Bad input.** A malformed path line is reported.

## Limits

- **No interactive macOS check.** The new bundle was not launched; launching a
  newly built GUI needs the user's approval.
- **Joints and shapes.** There are no arc walls, and no T or L cleanup between
  separate walls.
- **Openings.** Openings are rectangular. Door and window frames and leaves are
  plan graphics, not solids.
- **Path editing in the window.** The Inspector edits numeric path points only.
- **Not certified.** There is no building-code, accessibility or structural
  certification.
