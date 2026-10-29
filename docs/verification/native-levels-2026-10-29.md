# Native levels, slabs, stairs and service envelopes, 2026-10-29

These checks record agent verification, not human acceptance.

## Source and build

The checks cover these commits:

| Commit | Content |
| --- | --- |
| `6019ee6` | Core definitions, native slabs, stairs and envelope evaluation |
| `f943a82` | MCP tools |
| `b03f430` | Viewport envelope boxes |
| `369e8d6` | The window's Levels & stairs tool |
| `e5e1845` | Documentation |

They used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline` | 1,391 passed, 0 failed, 2 existing ignored |
| `npm test` | 25 of 25 passed |
| `./build` | Passed |

The build wrote `artifacts/development/build-TK7M56/MultiCAD.app`. The SHA-256
of its executable is `7918bfe94111d4d535e4ce23ddf82f49533c9a91ad8dcb35dc40c4cd12ea417b`.

## Evidence

### Native model: `crates/multicad-model/tests/levels.rs`

- **Slabs and pedestals.** A 6000 × 4000 slab has exactly 6000 × 4000 × 200
  mm³ and spans Z −200 to 0. A pedestal rises to Z 300.
- **Stairs.** A straight 16-riser stair from a level at 0 to one at 3000 runs
  from X 500 to 500 + 16 × 280 and tops out at Z 3000. Quarter-turn and
  half-turn stairs are each one solid with their exact computed volume and
  the same top. The step measure equals 2R + G with no notes for comfortable
  figures.
- **Level edit.** Raising the upper level to 3300 rebuilds the stair with a
  riser of 3300 / 16 and moves a wall on that level to Z 3300. Undo restores
  the exact document.
- **Refusals.** A stair down to a lower level ("above its base level") and a
  self-intersecting slab outline are refused with a reason.
- **Save and reopen.** The project declares the level capabilities, and the
  reopened document equals the saved one.
- **Envelope.** A machine with front 600, back 200, left and right 300
  clearances gives a box from (1700, 1400, 0) to (3300, 3000, 1500) of
  1600 × 1600 × 1500 mm³. The model snapshot carries the same corners for
  display. A wall it reaches is reported with an overlap of
  1600 × 200 × 1500 mm³. Narrowing the front clearance clears the conflict.
- **Plan.** A top view of the machine draws the envelope as four dashed
  hidden lines.

### MCP: `crates/multicad-mcp/tests/levels.rs`

`set_level`, `create_slab`, `set_stair`, `create_envelope` and
`check_clearances` build the same objects through the protocol and report
volumes, the scoped geometric stair check and clearance conflicts.

### Window: `crates/multicad-ui`

- **Panel test.** `modeling::architecture` creates a level, a slab on it, a
  stair and an envelope around the slab, each evaluated natively and
  committed. The stair reopens as a feature edit that keeps its Part, and a
  level edit runs as an ordinary transaction whose rebuilt model still
  carries the envelope box. A malformed outline line is reported.
- **Scene test.** `envelope_boxes_and_guide_outlines_are_checked_and_kept_with_the_scene`
  keeps valid boxes with the scene and refuses non-finite corners.

## Not verified

- **Interactive check.** The new bundle was not launched; that needs the
  user's approval. The dashed viewport boxes and the panel are covered by
  tests only.
- **Rotated Instances.** Envelopes around rotated Instances follow the
  tested pose math, but only placed Parts were exercised natively.
- **Other platforms.** Windows and Linux were not run.
- **Not certified.** Stair and envelope figures are geometric notes, not
  building-code, accessibility or structural approval.
