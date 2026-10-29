# Rolling tool case vertical slice, 2026-10-29

These checks record agent verification, not human acceptance. The macOS
interactive review below has not been performed.

## Scenario

`crates/multicad-app/examples/tool_case_review.rs` builds one editable
native project through the same transactional API the window and the MCP
server use. Every geometry transaction is evaluated by the native kernel
before it commits.

- **Carcass.** A 300 × 300 × 200 mm cabinetry carcass in 18 mm birch
  plywood stands on four casters, its bottom at z = 60. It has full-height
  sides, a top and a bottom, and no back or shelves.
- **Purchased hardware.** These Parts are tagged `purchased`:
  - the slide cabinet member and drawer member, each 6.5 × 250 × 35 mm, made
    of zinc-plated steel;
  - the 50 × 50 × 60 mm caster envelope.

  Each slide member carries Ø4.5 through holes. Their count, pitch and
  first offset come from the project parameters `rail_holes` (3),
  `rail_pitch` (96) and `rail_offset` (29).
- **Printed parts.** These Parts are tagged `printed` and assigned PETG:
  - a 238 × 250 mm open drawer with 4 mm walls and a 3 mm floor, its height
    set by `drawer_height` (80);
  - a 120 × 60 × 40 mm bit holder with a 80 × 20 mm tunnel under it.
- **Running gear.** One assembly, placed once, holds:
  - both slide cabinet members, as two Instances of one Part;
  - both runners (drawer members);
  - the drawer, the bit holder and the four casters.

  The drawer is a Slider joint on the left slide, pulling out along −Y with
  limits of 0–250 mm. The runners and the holder are Fixed joints on the
  drawer. Named poses "Drawer closed" and "Drawer open" exist; the closed
  pose is applied.
- **Source-driven drilling.** Four transfer groups take the slides' face
  holes, selected as all inner loops of the mounting face:
  - the left and right slide drill Ø3 × 12 mm pilots into the inside faces
    of the carcass sides;
  - the left and right runner drill Ø3 × 3 mm pilots into the outside faces
    of the printed drawer walls. The second drawer group builds on the
    first.

  The sides are world Parts. The drawer is an assembly component, so its
  cuts change the drawer definition.
- **FDM aids.** These are linked to the process record "PETG 0.2 / 0.45":
  - four 10 × 8 × 0.6 mm mouse-ear tabs, with a 2 mm overlap, at the
    drawer's bottom corners;
  - a removable support in the holder's tunnel. Its top is derived 0.2 mm
    below the tunnel ceiling.
- **Panel drawing.** Page TC-101 on A3 shows both sides from the inside at
  1:2. It has associative dimensions for the 300 mm length, the 200 mm
  height and a Ø3 pilot, a note, the panel cut list table and the title
  block.
- **Delivery.** Two export sets:
  - "Panel drawing" writes a PDF and a millimeter drawing DXF.
  - "Print package" selects the `printed` tag and writes one 3MF with the
    removable aids and one without them.

  The example also writes the BOM and the panel cut list as CSV, and a
  manifest.

## Shared-code change

The aid inspection measured targets only through world bodies. A printed
Part that is only an assembly component, such as this drawer, was
therefore reported as having "no evaluated solid", even though the
evaluator builds its aids against that same definition solid. Commit
`60ba57b` measures such Parts in their definition frame. Consumed Parts
stay unavailable. A focused test was added to
`crates/multicad-model/tests/fdm_aids.rs`, and
[FDM aids](../architecture/fdm-aids.md) describes the rule and its limits.

## Source and build

| Commit | Content |
| --- | --- |
| `60ba57b` | `fix(fdm)`: inspect aids of assembly component Parts against their definition |
| `e74f096` | The scenario and its app test |

The checks used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy (workspace) | Exit 0 |
| `cargo test --workspace --locked --offline --no-fail-fast` | 1,465 passed, 2 failed, 3 ignored. The two failures are the known worktree-only unitless-DXF tests (`plate.dxf` checked out with LF line endings); they pass on main |
| `cargo test -p multicad-app --test tool_case` | 6 of 6 passed, about 33 s |

## Evidence

### App test: `crates/multicad-app/tests/tool_case.rs`

- **Drilling.**
  - All four entry faces carry exactly three pilots at y = 29, 125 and 221,
    at z = 120. The positions are read from the exact circular edges of each
    transfer output.
  - Each transfer count is 3, and the transfer evaluation validates.
  - The sides measure 18 × 300 × 200 minus three Ø3 × 12 pilots. The drawer
    measures its shell minus six Ø3 × 3 pilots. The holder measures
    192,000 mm³.
  - The slides keep their own Ø4.5 holes; no transfer edits the hardware.
  - All four panels are cut-list records without issues.
- **Pitch and count edits.**
  - `rail_pitch` 80 moves every pilot on both sides and both drawer walls
    to 29, 109 and 189. The Ø3 drawing dimension stays attached, and Undo
    restores the exact document.
  - `rail_pitch` 64 with `rail_holes` 4 adds a pilot to every receiver, at
    29, 93, 157 and 221, with matching volumes.
- **Hardware replacement.**
  - The left slide Instance is switched to a different heavy-duty member,
    with two Ø5.5 holes at 50 and 200, and that member is added to the
    assembly definition.
  - The model then refuses to evaluate with "Transfer occurrence binding
    needs repair: Transfer path resolves to a different source/receiving
    Part". The old holes are never kept silently.
  - The mounting face is picked again from a source-only inspection, and a
    transfer-group edit is applied. The left side then has two pilots at 50
    and 200, and the right side still follows the original slide.
  - Undo of every step restores the exact document.
- **Poses and limits.**
  - The open pose moves the drawer, both runners and the holder exactly
    250 mm toward −Y. The slides stay in place, and the drawer keeps its
    pilots because transfers follow relative poses.
  - Undo restores the document.
  - A joint value of 251 mm does not evaluate.
- **Interference on the actual assembly.** Exact occurrence-pair checks
  give these results:
  - drawer to each side: 13 mm;
  - drawer to the bottom: 2 mm;
  - drawer to the top: 82 mm;
  - runner to slide and holder to drawer floor: contact, with no overlap.
- **Sampled travel.** `sample_assembly_range` on the drawer slider:
  - It samples 0, 50, 100, 150, 200, 250 and 275 mm, with coverage
    `SamplesOnly`. The six samples within the limits reproduce the closed
    clearances.
  - The 275 mm sample is refused as outside the inclusive [0, 250] limits.
  - The source document and revision are unchanged.
  - The report summary states that motion between samples is not checked.
- **Real overlap.** With `drawer_height` 170, the drawer–top pair reports
  `Overlapping` with exactly the 4 mm wall ring × 8 mm, 30,720 mm³. Undo
  restores the document.
- **FDM aids.**
  - All five aids inspect without findings against the process record.
  - The tabs touch the drawer from z = 80 and are 0.6 mm thick.
  - The support stands 0.2 mm below the tunnel ceiling.
  - The printed Parts keep their exact volumes.
- **Independent readers.** The test's own readers share no code with the
  writers.
  - **Project.** The saved project reopens equal and evaluates to the same
    transfer counts.
  - **PDF.** The PDF has one page, which a byte scan and `pdfinfo` confirm
    as A3. `pdftotext` finds the cut list, the panel names, TC-101, Ø3 and
    300.
  - **DXF.** A group-code parser confirms `$INSUNITS` = 4 (millimeters) and
    the dimension texts 300, 200 and `%%c3`. Six clusters of short visible
    lines have a 0.75 mm radius, which is Ø3 at 1:2. Their centers match the
    hole positions projected by hand onto both views to 0.001 mm.
  - **BOM CSV.** An RFC 4180 parser reads:
    - casters ×4, slide cabinet members ×2 and drawer members ×2, all
      `purchased`;
    - six manufactured Parts;
    - no aid rows. With aids included, the model BOM has five aid rows.
  - **Cut-list CSV.** The parser reads four 18 mm panels, with the sides at
    200 × 300.
  - **3MF.** A stored-ZIP and XML reader checks both packages.
    - Both declare millimeters.
    - The package with aids has 7 objects and 7 build items; the package
      without aids has 2 of each.
    - Drawer and holder mesh volumes, computed from the triangles, match
      the exact solids within the chord allowance.
    - Each tab is 10 × 10 × 0.6 mm and starts at z = 80. The support is
      76 × 56 × 19.8 mm.
- **MCP review, preview, apply and Undo.**
  - Through an owner that reviews every client, `set_parameter`
    `rail_pitch` = 80 returns `pending_review`, and the document stays at
    its base revision.
  - The owner compiles the queued candidate natively, as "Preview in
    viewport" does. Its left-side and right-drawer-wall pilots are at 29,
    109 and 189, while the session is unchanged.
  - After acceptance the model has those pilots, and `history` names
    `mcp:tool-case`.
  - An MCP `undo` is queued, then accepted. It restores the exact original
    document and pilots.
  - `activity` keeps the `previewed` and `applied` `set_parameter` entries,
    with their `rail_pitch` arguments, and the `undone` entry, all
    attributed to `mcp:tool-case`.

### Generated deliverables

`cargo run -p multicad-app --example tool_case_review -- artifacts/acceptance/tool-case-2026-10-29`
wrote these files. They are kept locally under that `artifacts/` path,
which Git ignores.

| File | SHA-256 |
| --- | --- |
| `tool-case.multicad` | `1b923d09d925af2b8f9bbe80aa71d6c6bc5039d19993947b2df4afb703520f37` |
| Panel drawing PDF (TC-101) | `265bfaca56d499459dda5926951d65bdd3cdc73df13723535d5d9b3ce819575b` |
| Panel drawing DXF | `536049709760a60a7b3f56afc3a2fc21b1edff60ed60979294009719227cb0e1` |
| 3MF with aids | `8a8881e226e3f7607fab197685d6fd3b7ad807795e44a5f46129258cbdce4ffc` |
| 3MF without aids | `234f4759598109ece7e8514df4ede9c4be5503a3be71d1a5b143bc2b1c0b2a4d` |
| `schedules/bom.csv` | `252323985321c21b69979a344104c7986a4dddc5b97f2f0c359a8f3dff5f172c` |
| `schedules/panel-cut-list.csv` | `fd1cd80b8ad5a9e0e5e1ae55e341a59c3a1eeef2b9ca63e738021069f6563863` |
| `manifest.json` | `62c41761834056924f0e538f3ebc80a5672663a2dfeb01b22b6c0b5093db96c9` |

Project and export file names contain fresh entity IDs, so each new run
writes different names and hashes. The manifest records the transfer
counts (3 per group), the closed interference pairs and the sampled travel
report with its coverage statement.

The PDF was rendered and inspected. It shows both side panels from the
inside with three pilot circles each, mirrored between the views, the
300 and 200 dimensions, the Ø3 diameter dimension, the note, the cut list
table and the title block.

## Operator review path (macOS, pending)

1. Generate the project with the command above into a new directory, then
   open `tool-case.multicad` in the MultiCAD window.
2. **Parameters.** Change `rail_pitch` to 80 and Apply/Rebuild.
   - The pilots in both sides and both drawer walls should move.
   - Page TC-101 should show the moved holes, with the Ø3 dimension still
     attached.
   - Undo.
3. **Model.**
   - Apply the named poses "Drawer open" and "Drawer closed". The drawer,
     runners and holder should move together.
   - In **Interference / Clearance**, select the Running gear / Drawer and
     a carcass side. Run **Sample motion range** on the drawer slider from
     0 to 275 mm in 50 mm steps. Compare the result with the manifest's
     `travel` entry.
   - Open the transfer groups and the FDM aids, and check the support under
     the holder's tunnel.
4. **Drawings.** Compare TC-101 with the exported PDF.
5. **Export.**
   - Run "Panel drawing" and "Print package".
   - Open the PDF and DXF in other applications.
   - Load both 3MF files in a slicer. One should include the tabs and
     support; the other should not.
6. **MCP.** Connect a client with `multicad --mcp --app` and call
   `set_parameter` on `rail_pitch`. Preview and accept it in the review
   queue, call `undo`, accept again, then read `activity`.

## Not verified

- **Interactive check.** No operator step above has been performed by a
  human.
  - The window's Interference / Clearance panel has not been run on this
    project.
  - MCP has no transfer, interference or travel tool.
  - The MCP change shown is the rail-pitch parameter. Editing a transfer
    group itself over MCP is not available.
- **Sampling.** Travel is checked at discrete samples only. A collision
  between samples would not be found.
- **Layout.** Positions are fixed, not parametric:
  - The carcass size, slide positions and drawer width are literal values.
    A carcass edit does not move the slides.
  - Caster bolt holes are not transferred; the caster is an envelope.
  - The slide members are simplified bars with a hole pattern, not
    manufacturer geometry. They have no ball-bearing extension, so the
    drawer runner moves past the cabinet member.
- **Machining output.** Transferred pilots appear in the drawing and the
  solids. They are not in the cabinetry machining schedule, which lists
  only cabinetry machining operations. There is no CNC toolpath or
  drilling program.
- **Aids.** Aids are placed in the drawer's modeled frame and do not follow
  it to the open pose. Fit-clearance findings consider world Parts only.
- **External applications.** The deliverables were checked by the test's
  readers, `pdfinfo`/`pdftotext` and one PDF rendering. They were not
  checked in an external CAD tool, CNC software, slicer or printer.
- **Other platforms.** Windows and Linux were not run.
- **Drawing.** The title block reads "Scale 1:1", the page default; the
  views are 1:2. The placed cut list table is a snapshot and does not
  update itself.
- **Not certified.** The slide load, wood joinery, print strength and fit
  are not certified, and PETG and plywood properties are nominal.
