# Level-linked drawing guides, 2026-10-29

These checks record agent verification, not human acceptance.

## Source and build

The checks cover these commits:

| Commit | Content |
| --- | --- |
| `99cc01d` | Guide source, generation, live resolution and viewport outlines |
| `559d630` | MCP `set_drawing_guide` |
| `d11c011` | Viewport display and the window's Drawing guide choice |

They used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline` | 1,394 passed, 0 failed, 2 existing ignored |
| `npm test` | 25 of 25 passed |
| `./build` | Passed |

The build wrote `artifacts/development/build-i0vhqR/MultiCAD.app`. The SHA-256
of its executable is `fb978971737866392eec202e573e439802116cc041c6db1b2d341439a801370d`.

## Evidence

### Native model: `crates/multicad-model/tests/guides.rs`

The fixture has two levels at 0 and 3000, each with a closed 5000 × 4000
concrete room that has a hatch style.

- **Floor plans.** One guide with a 1000 mm cut and 1200 mm view depth
  creates two linked views on new pages:
  - "Floor plans – Ground floor" cuts at Z 1000 and "– First floor" at Z 4000;
  - both are section views with a 1200 mm depth range;
  - each draws hatched section outlines of only its own level's room;
  - the snapshot holds two cut-plane outlines at Z 1000 and 4000.
- **Live level edit.** Raising the first floor to 3300 moves its linked plan
  cut to Z 4300 without regenerating, and the view still evaluates.
- **Rerun.** A cut of 1100 with only the ground level:
  - updates the ground view in place, in the same book with no new views;
  - reports the first-floor view as stale;
  - evaluation reports that view as "no longer part" of the guide.
- **Save and reopen.** The project declares `drawings.guide.1` and
  `drawings.guided_views.1`, and the reopened document is equal.
- **Section.** A line from (−500, 2000) to (5500, 2000) with a 3000 mm depth
  gives:
  - frame u = +X, v = +Z and normal −Y;
  - window u 0–6000 and v −200 to 6000, from the ground slab bottom to the
    top of the first floor;
  - hatched cut vectors and a matching viewport outline.

  Raising the first floor widens the window to 6300.
- **Elevation.** A guide looking toward +Y adds a projection view with
  normal −Y to the named existing book, and it evaluates.
- **Refusals.** A zero-length section line and a plan without levels are
  refused with a reason.
- **Deletion.** After its guide is deleted, the elevation view is reported
  as "deleted" instead of being drawn.

### MCP: `crates/multicad-mcp/tests/guides.rs`

`set_drawing_guide` creates a two-level plan guide with two views, and none
are unresolved. A rerun with a new cut updates both views in the same book
and creates none. A stale revision and a plan without levels are refused.

### Window: `crates/multicad-ui`

- **Panel test.** The `modeling::architecture` test adds a Drawing guide on
  the single level. It generates one view; reopening the guide with another
  cut keeps one view. Clearing the levels is refused.
- **Scene test.** `envelope_boxes_and_guide_outlines_are_checked_and_kept_with_the_scene`
  keeps guide loops with the scene and refuses non-finite corners.

## Not verified

- **Interactive check.** The new bundle was not launched; that needs the
  user's approval. The dashed guide outlines and the panel are covered by
  tests only.
- **Exports.** PDF and DXF output of guided views uses the same evaluated
  vectors as other views but was not exercised separately here.
- **Other platforms.** Windows and Linux were not run.
