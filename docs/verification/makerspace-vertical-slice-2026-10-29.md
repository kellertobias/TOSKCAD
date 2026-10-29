# Makerspace vertical slice, 2026-10-29

These checks record agent verification, not human acceptance. The macOS
interactive review below is still pending.

## Scenario

`crates/multicad-app/examples/makerspace_review.rs` builds one editable
native project through the same transactional API the window and the MCP
server use. Nothing in it is placeholder geometry: every solid is evaluated
by the native kernel before its transaction commits.

- **Room.** Project parameters `room_width` (9000) and `room_depth` (6000)
  drive one closed loop of the reusable wall type "Block 200" (200 mm, 3600 mm
  high, concrete). The loop has a 1000 mm door on the south wall and two
  1500 mm windows of reusable opening types.
- **Levels.** "Ground floor" at 0 and "Mezzanine" at 2600. A ground slab
  under the whole room and a 3000 × 2500 mezzanine deck follow the room
  parameters. A straight 14-riser stair rises from the ground floor onto the
  deck edge.
- **Equipment.** One dynamic symbol, "Equipment footprint", has exposed
  width, depth and height and its footprint sketch as its plan symbol. Its
  body carries the `equipment` tag. It is placed three times with its own
  values: a laser cutter (1400 × 900 × 1000), a CNC router
  (2500 × 1500 × 1100) and a workbench (1800 × 800 × 900). Each placement has
  a service envelope with front, back, side and top clearances.
- **Drawings.** A floor plan guide on the ground floor (cut 1200, depth 1400)
  and a section guide through the stair generate two linked A3 pages at 1:50
  in one book:
  - A-101 adds the room's inner width and the workbench width as associative
    dimensions, a note, an equipment table and the title block.
  - A-201 is section A-A.
- **Delivery.** The export set "Makerspace delivery" selects the
  `equipment` tag and every page, with PDF, drawing DXF and STEP recipes.

## Source and build

| Commit | Content |
| --- | --- |
| `2f39fd1` | The scenario and its app test |

The checks used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline` | 1,397 passed, 0 failed, 2 existing ignored |
| `npm test` | 25 of 25 passed |

## Evidence

### App test: `crates/multicad-app/tests/makerspace.rs`

- **Initial model.**
  - The walls equal the 2 × (9000 + 6000) × 200 × 3600 mm³ loop minus the
    door and window openings.
  - The slab is 9200 × 6200 × 200 mm³, and each machine has exactly its own
    width × depth × height.
  - All three envelopes are clear of every final solid. The router box is
    3200 × 2800 mm in plan.
  - The plan's room and workbench dimensions read 8800 and 1800.
- **Room edit.**
  - `room_width` 9600 rebuilds the walls and slab to their new exact volumes,
    and the room dimension reads 9400 without regenerating the page.
  - Undo restores the exact document.
- **Clearance conflict.** `room_width` 8700 moves the east wall into the
  router's right clearance. The router envelope reports exactly the walls as
  a conflict, and Undo restores the document.
- **Equipment edit.**
  - A workbench width override of 2000 resizes only that placement: the
    laser cutter's volume is unchanged.
  - The workbench envelope becomes 2000 × 2300, and its dimension reads 2000.
- **Rotation.** A quarter-turn instance move of the workbench turns its
  envelope to 2300 × 2000 in plan, still clear and with the same volume.
- **Pages.** Both pages are checked with `checked_page` after every edit.
- **Save and reopen.** The edited project reopens equal to the saved
  document and evaluates.
- **Exports.**
  - The PDF has two pages.
  - The two drawing DXFs include the plan with its 8800 and 1800 dimension
    text and `$INSUNITS`.
  - Exactly three STEP files are written, one per tagged machine, and they
    read back with the machines' exact volumes. Walls, slabs and the stair
    are not exported.
- **MCP.**
  - Through an owner that reviews every client, as the window does by
    default, `set_parameter` on `room_width` stays `pending_review` without
    changing the revision.
  - After acceptance, `check_clearances` reports three clear envelopes and
    `history` names the `mcp:makerspace` actor.
  - The `activity` audit shows the applied `set_parameter` with its
    arguments, and the walls have the 9600 mm volume.

### Generated deliverables

`cargo run -p multicad-app --example makerspace_review -- artifacts/acceptance/makerspace-2026-10-29`
wrote these files. They are kept locally under that `artifacts/` path (ignored by Git) and recorded on CAD-98:

| File | SHA-256 |
| --- | --- |
| `makerspace.multicad` | `eb4ca352f6b891025aac7105688562ac4bd8d3bae13240a8958e8481df93f677` |
| Plan and section PDF | `028f8eb15db8f64e7c88c2be6096a13a75606a07f3d42b457f9159d4167d87ae` |
| Floor plan DXF | `3bccb450b02d9d1f65b516d83f37642ac3d6731db5dcbfa8c89f08cf9840d467` |
| Section DXF | `2c43f321ee3230c97cdc495e10ec2483b8f8a5da580d152b27f206beb5467a88` |
| Equipment STEP files | Three files, one per machine |

Project and export file names contain fresh entity IDs, so a new run writes
different names and hashes. Reopening `makerspace.multicad` with the app's
project loader (`drawing_file_review`) regenerated both pages and their two
dimensions.

The PDF was rendered and inspected. The plan shows:

- hatched walls with the door swing and both windows;
- the cut stair with its treads below the cut;
- the three equipment symbols with dashed service areas;
- the dimensions, the table and the title block.

The section shows the walls, the slab, the hatched deck and stair, and the
north window beyond.

## Operator review path (macOS, pending)

1. Generate the project with the command above into a new directory, then
   open `makerspace.multicad` in the MultiCAD window.
2. **Parameters.** Change `room_width` to 9600 and Apply/Rebuild. The walls,
   slab and plan dimension should follow. Undo.
3. **Model.**
   - Turn on **Service envelopes** in the camera tools menu to see the
     dashed clearance boxes.
   - Edit the workbench's symbol values and move or rotate it.
   - **Levels & stairs** shows the levels, the stair and the drawing
     guides.
4. **Drawings.** Open the two pages and compare them with the exported PDF.
5. **Export.** Run "Makerspace delivery" and open the PDF, DXF and STEP
   files in other applications.
6. **MCP.** Connect a client with `multicad --mcp --app`, call
   `set_parameter` on `room_width`, accept it in the review queue and call
   `check_clearances`.

## Not verified

- **Interactive check.** None of the operator steps above has been performed
  by a human. The window has no clearance-conflict report; conflicts are
  reported by the model API and MCP `check_clearances` only.
- **Placements.** Equipment placements are literal positions; they do not
  follow `room_width`, so a smaller room can produce conflicts as shown.
- **External applications.** The PDF, DXF and STEP files were checked by the
  test's readers and one PDF rendering, not in external CAD or print
  applications.
- **Other platforms.** Windows and Linux were not run.
- **Not certified.** Envelopes, the stair and the layout are geometric aids,
  not building-code, egress, accessibility or machine-safety approval.
