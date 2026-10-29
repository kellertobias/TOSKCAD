# PCB-driven control desk slice, 2026-10-30

These checks record agent verification for CAD-101, not human acceptance.
The macOS interactive review below is still pending.

## Scenario

`crates/multicad-app/examples/control_desk_review.rs` builds one editable
native project through the same typed transactions the window and the MCP
server use. Each transaction is compiled natively before it commits; there is
no placeholder geometry.

- **Board.** `crates/multicad-app/tests/fixtures/control_desk/` holds a
  hand-written KiCad 8 board, `control-panel.kicad_pcb`: a 160 × 100 mm
  outline with R3 corners, 1.6 mm thick, with four M3 mounting holes (H1–H4),
  an OLED module (DS1), two 12 mm push buttons (SW1, SW2), an EC11 encoder
  turned 90° (ENC1), a 5 mm LED (D1), a USB-C receptacle on the back with oval
  drills (J1, not imported), two vias, a track and silkscreen text.
  `control-panel-revB.kicad_pcb` moves SW1 by 6 mm and H4 by 5 mm, removes D1
  and adds SW3.
- **Models.** The example writes STEP models for the display, the buttons and
  the LED beside the board (`models/`, as `${KIPRJMOD}` resolves). The
  encoder's model, `rotary_encoder_ec11.step`, is deliberately never written.
- **Panel.** "Control panel" is a sheet part from "AlMg3 1.5 sheet" stock
  (EN AW-5754 H22, 1.5 mm, inside radius 1.5 mm, K 0.44). Its base is
  `$desk_width` × `$panel_depth` (240 × 160) on XY. The front flange (50 mm)
  and the back flange (30 mm) are bent 90° down, each inset 14 mm from both
  ends to clear the cheeks, with 2 × 4 mm reliefs.
- **Board-driven features.** The board origin lands at
  (`$board_x`, `$board_y`), both parameters centered on the panel. Each board
  mounting hole becomes a standoff point, and each imported component with a
  panel rule becomes a cutout, written as expressions such as
  `=$board_x + 5 mm`. DS1 gets a window (its courtyard shrunk by 6 mm per side,
  30 × 16 mm), the buttons 12.5 mm holes, ENC1 a 7.5 mm shaft hole and D1 a
  5.2 mm hole.
- **Hardware.** Three mounting patterns on the panel, each purchased item its
  own Part tagged `purchased`:
  - four M3 × 10 hex standoffs (AF 5.5) on the base side at the board holes;
  - four ISO 4762 M4 × 20 screws with ISO 7089 washers from above into the
    cheeks;
  - two self-clinching M3 nuts on the back flange (user-entered catalogue
    values, sheet range 1.0–2.3 mm).
- **Cheeks.** Two 12 mm birch plywood cheeks under the side strips follow
  `$desk_width` and `$panel_depth`.
- **Printed guide frame.** A PETG frame fills the gap between the board top
  and the panel: 160 × 100 × 10 mm with a 140 × 80 mm window cut by a Boolean.
  It receives the board's mounting holes through the shared transfer engine
  (`FaceHoles` on the board top, 1.8 mm radial clearance) inside the "Board
  stack" assembly. It carries the `printed` tag.
- **Board-drive check.** `Scenario::findings` compares the panel with the
  board record and reports:
  - `panel_stale`: a derived standoff point or cutout that no longer matches
    its source, or that is missing or extra, with the distance;
  - `component_missing`: a component gone from the board, whose Part and
    opening are kept for review;
  - `panel_unmapped`: a component without a panel rule;
  - `model_missing`: a component without a linked STEP model, with the
    path the board names;
  - `guide_interference`: a component reaching into the guide frame,
    measured as an exact intersection volume.
- **Repair.** `link_model` imports the model the board names and links it to
  the component, then removes the unplaced STEP body that the import always
  creates. `prepare_sync` writes the board-driven points and cutouts into the
  panel in one transaction; hardware Parts keep their identities.
- **Drawings.** Four A3 pages in the "Control desk" book:
  - CD-101 at 1:2: front, top and side views of the panel, cheeks, hardware
    and board stack; the panel width as an associative dimension; and a note
    with the process assumptions.
  - CD-102 at 1:2: the panel top view and the bend schedule table, with a note
    naming the developed DXF.
  - CD-103: the BOM and purchased hardware tables.
  - CD-104: the board-driven features with their findings.
- **Delivery.** Three export sets:
  - "Control desk drawings": PDF and drawing DXF;
  - "Fabricated parts STEP": the `fabricated` tag, which covers the panel and
    the cheeks;
  - "Printed guide 3MF": the `printed` tag, in millimeters.

  The example also writes the developed panel DXF through
  `Exporter::developed_sheet_dxf`, after `check_current`. It writes BOM,
  bend, hardware and board-drive CSV files and a manifest with SHA-256
  hashes.

## Source and build

| Commit | Content |
| --- | --- |
| `e890090` | Fixtures and the `control_desk_review` scenario |
| `85bd219` | The app test `tests/control_desk.rs` and `dxf` 0.6.2 as a dev-dependency of `multicad-app` (already locked for `multicad-model`) |
| `fd74e6d` | Folded views placed clear of the title block |

The commits are on branch `worktree-agent-a8260466c36c5b2bb` and are not
merged. No shared library code changed.

The checks used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Exit 0 |
| `cargo test -p multicad-app --test control_desk --locked --offline` | 4 passed |
| `cargo test --workspace --locked --offline --no-fail-fast` | 1,476 passed, 2 failed, 3 ignored. The two failures are the unitless-DXF tests (`unitless_binary_and_malformed_dxf_fail_without_guessing`, `artwork_import::tests::unitless_dxf_requires_an_explicit_unit_layers_filter_and_cancel_changes_nothing`); in this worktree `crates/multicad-core/tests/fixtures/sketch_import/plate.dxf` is checked out with LF line endings instead of CRLF. They are unrelated and pass on main. |

## Evidence

### App test: `crates/multicad-app/tests/control_desk.rs`

- **Import.**
  - ENC1's model is reported missing with the path the board names.
  - Vias and oval drills are reported as unsupported.
  - The board is the exact rounded outline minus the four holes, times
    1.6 mm.
  - DS1, SW1, SW2 and D1 have exactly their STEP model volumes.
  - ENC1 is its 14 × 12 × 5 mm courtyard envelope.
  - At revision A the only finding is `model_missing` for ENC1.
- **Panel against its inputs.**
  - Thickness, radius and K-factor are 1.5, 1.5 and 0.44, both bend values
    from the stock. The measured thickness is 1.5 mm.
  - The flat area equals base plus both developed flanges (leg plus
    π/2 · (r + K·t) = 3.393 mm), minus the four relief notches and every
    opening.
  - The folded volume equals the same area times t with each allowance strip
    replaced by the exact bend sector.
  - Both bends are 90° down, 212 mm long, with that allowance.
  - Unfolding reports only the `tooling` diagnostic: no hole near a bend,
    no short leg and no missing relief at these sizes.
- **Hardware placement.**
  - The four standoffs span z −10 … 0 under the base face, centered on the
    panel. The placed board top is at z −10.
  - Standoff drill points are the board holes plus the board origin (40, 130).
  - The inserts' drill points lie 15 mm past the back bend in the blank.
  - No hardware record has a finding.
- **Clearance.**
  - Every component's exact intersection with the panel is zero. SW1 rises to
    z 3, through its opening and 1.5 mm above the panel top.
  - The guide frame has its exact volume with four 6.8 mm holes and does not
    touch the standoffs.
- **Reimport (revision B).**
  - The plan reports SW1 moved, SW3 added and D1 missing; the holes changed
    and the outline did not.
  - The check reports `panel_stale` for `fastener.0.3` (H4, 5 mm away),
    `cutout.1` (SW1, 6 mm away) and `cutout.5` (SW3, not on the panel). It
    also reports `component_missing` for D1 and `model_missing` for ENC1 and
    for SW3, which the reimport added without a model link.
  - The moved SW1 now intersects the unsynchronized panel by more than
    1 mm³.
  - The guide frame's H4 hole moved to (190, 35) through the transfer engine
    alone. A probe there is clear; a probe at the old (195, 35) hits the
    frame.
- **Repair and synchronization.**
  - After linking SW3's model and synchronizing, only `component_missing`
    for D1 and `model_missing` for ENC1 remain.
  - The drill points are at the board's new positions.
  - Every original hardware Part identity remains.
  - No component touches the panel or the guide frame.
  - D1's opening is retained, and the flat area includes it.
  - Undo returns to the reimported document with the same findings.
- **Sheet edits.**
  - `desk_width` 280 rebuilds the panel to its exact flat area and folded
    volume, with 252 mm bends.
  - The standoffs move to the new board origin (drill `fastener.0.0` at
    (65, 125), standoffs centered on x 140). The right cheek screws follow
    `$desk_width - 6 mm`, and the first insert follows the flange start.
  - The same hardware Parts remain, without findings.
  - The right cheek ends at x 280, and no component touches the panel.
  - The width dimension reads 280 on CD-101.
  - The placed bend table is reported stale by `Schedule::check_table`.
    Undo restores the exact document; the dimension reads 240 and every placed
    table is current again.
  - Moving the left cheek screws keeps every hardware Part and moves the
    drill points.
- **Deliverables, read back independently.** The test runs `generate`:
  - The manifest's SHA-256 values match the files. Its verification flags
    for interactive macOS, human acceptance, press-brake tooling,
    physical fit and a real KiCad project are all false.
  - The project file reopens and compiles to the same panel area and volume.
  - The developed DXF reopens with the `dxf` crate:
    - millimeters;
    - one `CUT` contour and 16 `CUT_INNER` openings, whose areas give the
      flat area and the opening area exactly;
    - two 212 mm `BEND_DOWN` lines and no `BEND_UP`;
    - 15 `DRILL` points;
    - labels such as `flange.0.bend.0 DOWN 90.000 R1.500 #1` and
      `fastener.2.1 D4.200`.
  - The bend CSV holds both bends with allowance 3.393 and length 212.
  - The BOM CSV lists the panel with its material and stock, the cheeks, the
    guide frame, the four hardware Parts as `purchased` and the components as
    `imported`, with no loose STEP body.
  - The hardware CSV counts 4 standoffs, 4 screws, 4 washers and 2 inserts
    from the solids.
  - The board-drive CSV carries the D1 and ENC1 findings.
  - The PDF has four pages. `pdfinfo` reports four A3 pages. `pdftotext`
    finds:
    - the width 240 and the assumptions note on CD-101;
    - the bend table on CD-102;
    - the BOM and hardware on CD-103;
    - the findings on CD-104.
  - Each page also has a drawing DXF.
  - Exactly three STEP files (the panel and the two cheeks) read back with
    their exact volumes.
  - The one 3MF passes `unzip -t`. Its mesh volume, in millimeters, is within
    0.2 % of the guide frame's exact volume.
- **MCP.**
  - Through an owner that reviews every client, `set_parameter` on
    `desk_width` stays `pending_review`.
  - The owner compiles the queued candidate as a preview: drill
    `fastener.0.0` at (65, 125) and 252 mm bends, while the document stays at
    the base revision.
  - After acceptance:
    - `unfold_sheet` reports 252 mm bends;
    - `sheet_hardware` lists the same standoff Part;
    - `history` names `mcp:control-desk`.
  - A reviewed `undo`, once accepted, restores the original document.
  - `activity` shows the applied `set_parameter` with its arguments, actor and
    base revision, and the `undone` entry.

### Generated deliverables

`cargo run -p multicad-app --example control_desk_review --locked --offline -- artifacts/acceptance/control-desk-2026-10-30`
wrote these files at revision 46. They are kept locally under that
`artifacts/` path, which Git ignores:

| File | SHA-256 |
| --- | --- |
| `control-desk.multicad` | `526954ae347b230a59c5e7ef4ec80f4ea5ec26b050ac822b0a26aaae40cbe630` |
| Four-page PDF (CD-101 to CD-104) | `59ed8719d45f3bd4659fc5e04333284455b302218692cdc1400a1475ead2ca8c` |
| Drawing DXF CD-101 (folded views) | `b5050c3bbfd4ccf5038e715849f901a593b882a6a52c258683f757f8ca237a70` |
| Drawing DXF CD-102 (top view, bends) | `72ccb33521de1835b3b2a26ab7fad04a02a0dbfa82f16a2ae31fde567efcb424` |
| Drawing DXF CD-103 (BOM, hardware) | `a836b50abc4da00a5068fd8f3b9a2fdfa41ebdd7f7b7fc0f3fc673a291b7957b` |
| Drawing DXF CD-104 (board-driven features) | `4ba79769f5b6fc6da7aef583c7765cb77944bff0383dbc4592e693ec819ff1c8` |
| Control panel STEP | `3cf16c8bdd52dc0e767d8551effce3d9a6fa9a9b1cea2a8db9a5a54974bc301f` |
| Left cheek STEP | `b9ea1a8bca0979f769594c52178930a555190171c28fb58357cb25f4ca4da46f` |
| Right cheek STEP | `72a96d3598a12371a037bb296e3166f974229995a395f01e4f8dc937fd99bf56` |
| Printed guide frame 3MF | `71d0dc84cb24084cf1aba4a929d2d221da736bff40efafdc942d4b2e8566a0ee` |
| `control-panel-developed.dxf` | `fb430b4c1f6dc0ead5bdd393fe3cf62af7a6e05e255241ce51e6b81493dd444a` |
| `bom.csv` | `c03036d005e4bbee8ca3dafca8e30a8d0cb55c18a30f8b8bd428ecd575ed9fe2` |
| `bend-schedule.csv` | `ba5f400041a664286d84fb0c038e14b161d23d0b030927780ae410da73a9d804` |
| `hardware.csv` | `3fb1d5ea8d8fb5371b100c427d8af67d31c7e8c0a701c97eee2fa4fcabeacf11` |
| `board-drive.csv` | `27797d15ac3acbccc6e5eb44256b8ee60a0242839f6957fbe5442630554e800a` |

The PDF was rendered with `pdftoppm` and inspected:

- CD-101 shows the top, front and side views clear of the title block, with
  the 240 width, the openings, standoffs, screws and cheeks;
- CD-102 shows the panel top view and the bend table.

Project and export file names contain fresh entity IDs, so a new run writes
different names and hashes. `manifest.json` records the reimport plan, the
findings at revision A, after the reimport and after the repair, the panel
values and diagnostics, and every file hash.

The final state deliberately keeps two findings visible:

- `component_missing` for D1, whose opening is kept for review;
- `model_missing` for ENC1, whose opening comes from its footprint.

## Operator review path (macOS, pending)

1. Generate the project with the command above into a new directory, then
   open `control-desk.multicad` in the MultiCAD window.
2. **Parameters.** Change `desk_width` to 280 and Apply/Rebuild. The panel,
   cheeks, board stack, standoffs and the CD-101 width dimension should
   follow. Undo.
3. **Model.** Inspect the panel's flanges, reliefs, openings and hardware.
   Check that the buttons pass through their openings, and look at the
   guide frame and its standoff holes.
4. **Drawings.** Open CD-101 to CD-104 and compare them with the exported
   PDF.
5. **Export.** Open the developed DXF, the STEP files and the 3MF in external
   CAD, CAM and slicer applications. Check the CSV files in a spreadsheet.
6. **MCP.** Connect a client with `multicad --mcp --app`:
   - call `set_parameter` on `desk_width`, preview and accept it in the
     review queue;
   - call `unfold_sheet` and `sheet_hardware`;
   - undo through the queue.

## Not verified

- **Interactive check.** No human has performed any of the operator steps
  above. The window has no KiCad import, board-drive check or hardware form.
  Those workflows run through the core API (and MCP for imports and sheet
  hardware) only.
- **Panel association.** The panel's board-driven features are not
  associative in the engine:
  - The panel follows `$board_x`, `$board_y`, `$desk_width` and
    `$panel_depth` through parameter expressions.
  - A board reimport does not move its standoff points or cutouts. The
    scenario's board-drive check reports them as stale, and an explicit
    synchronization transaction rewrites them.
  - A transfer cut on the sheet Part would make it a later feature that
    unfolding refuses, so the engine's transfer path is used for the guide
    frame instead.
  - The panel rules (which reference gets which opening) live in the
    example, not in the document.
- **Model faces.** Faces of linked STEP component models have no source
  provenance, so they cannot drive a `FaceBoundary` transfer. Only envelope
  components and board faces can.
- **KiCad lookup.** `${KIPRJMOD}` model paths resolve only when the board
  folder is absolute. A relative board path joins the folder twice and
  reports the models missing, so the example canonicalizes its source folder.
- **Reimport.** Components that a reimport adds get no model link, even when
  the model file exists. The scenario reports them as `model_missing` and
  links SW3's model explicitly.
- **BOM quantities.** The BOM counts Part occurrences, so each hardware Part
  is quantity 1. The hardware schedule counts pieces from the solids (4, 4,
  4 and 2).
- **PDF flat pattern.** The PDF export has no flat-pattern view provider, so
  the developed blank is delivered as the 1:1 DXF. The PDF carries the
  folded views, the panel top view and the bend schedule.
- **Drawing tables.** Tables are placed content. They go stale after an edit
  (as the test shows) until the scenario refreshes them.
- **Manufacturing and process.**
  - Bend allowance uses the stock K-factor only. There are no bend tables,
    press-brake, die, punch or springback data, and the order is not a
    bending sequence.
  - The DXF has no kerf, tool offsets or nesting.
  - Threads are plain cylinders. Insert values are user-entered, not a
    supplier's data.
  - The cheeks have no modeled pilot holes or threaded inserts for the M4
    screws.
  - No part was made, fitted or assembled.
- **Sources.** The board is hand-written in KiCad 8 syntax, and the STEP
  models are OCCT-written boxes. No real KiCad project or vendor model was
  run.
- **External applications.** The outputs were checked by the test's readers,
  `pdfinfo`, `pdftotext` and `unzip`, not in external CAD, CAM, slicer or
  spreadsheet applications.
- **Other platforms.** Windows and Linux were not run.
