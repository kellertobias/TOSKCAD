# Mixed-material table fabrication slice, 2026-10-29

These checks record agent verification, not human acceptance. The macOS
interactive review below is still pending. Geometry and output validation
here is not a structural, welding or fabrication certification.

## Scenario

`crates/multicad-app/examples/cabinetry_table_review.rs` builds one editable
native project through the same transactional API the window and the MCP
server use. Every transaction is evaluated by the native kernel before it
commits. Nothing in it is placeholder geometry.

- **Parameters.** `table_width` (1600), `table_depth` (800) and
  `table_height` (900) drive the frame, the worktop, the cabinet depth, the
  back upstand and the feet. `cabinet_width` (500) drives the cabinet.
- **Steel frame.** SHS 40×40×2 S235 tube, 6000 mm bars, 13 separate member
  Parts:
  - a base frame from a constrained sketch rectangle whose two sides are
    bound to `table_width` and `table_depth`, standing on the floor
    (section anchored at its bottom face), with four mitered corners;
  - a top frame on parameter-driven point pairs with four mitered corners;
  - four legs butted between the base and top rails, under the front and
    back rails;
  - a cross rail butted between the front and back top rails.

  The 18 connections each carry one fillet weld (W1–W18, 3 mm, all around,
  135 MAG). The sequence "Table frame sequence" orders them: top corners,
  cross rail, leg tops, base corners, leg feet, with step notes.
- **Worktop.** A stack "Worktop" (PVAc D3) of two layers on the frame:
  - "Worktop core", MDF 19, grain free, a 3 mm trim oversize;
  - "Worktop face", beech multiplex 12, grain along the length, 2 mm bands
    on both ends and the front, the front band with a 1 mm allowance.
- **Cabinet.** A birch ply 18 carcass "Cabinet" hung under the top frame:
  full-height sides, top, bottom and one shelf, a 1 mm front band with a
  0.5 mm allowance, and four Ø5 × 10 shelf-pin holes drilled from the inside
  face of the left side.
- **Upstands.** "Back upstand" and "Left upstand", birch ply 18, on the
  worktop, joined at the back left corner by a five-finger joint with a
  0.2 mm fit. Their outer faces are fixed, so a thickness change keeps the
  corner block one full thickness of each panel.
- **Purchased hardware.** "Leveling foot M10", tagged `purchased`, placed
  under each leg: one Part, quantity 4.
- **Drawings.** One book, four A3 pages at 1:10:
  - T-101: front and top views with three associative dimensions (width
    over the worktop, floor to worktop, depth) and notes.
  - T-102: an isometric frame view with the weld plan (one weld symbol per
    joint and the "Weld sequence" table from `weld_plan_annotations`).
  - T-103: the fabrication cut list and the machining schedule.
  - T-104: the member cut list and the BOM.
- **Export sets.**
  - "Table frame and drawings": the `frame` tag and every page, with PDF,
    drawing DXF and STEP recipes.
  - "Panel cutting": the `panel` tag, with a 1:1 planar DXF recipe (one
    saved face per panel; the drilled face where there is one) and STEP.
- **CSV schedules.** Written by `Schedule::csv` from the shared builders:
  fabrication cut list, panel cut list, machining, member cut list, weld
  schedule and BOM.

The fabrication cut list is assembled in the example from the panel cut
records, the member cut groups and the BOM's purchased rows, as one
`Schedule`. It has three sections:

- panels and stack layers with quantity, stock, thickness, finished and
  core size, grain and bands with allowances;
- members by mark, with quantity, overall length and both end cuts with
  their long-point face;
- purchased Parts with their quantity.

## Shared-code changes

Two small changes in shared code, each in its own commit with tests:

- **Long points.** Commit `042c098`. Member end records now
  carry the long point of an angled plane cut: its world direction and
  section face (`+x`, `-x`, `+y`, `-y`). Cut groups also require the same
  long-point relation ("same face", "opposite faces" or "adjacent faces"),
  so picture-frame and parallelogram miters no longer group together. The
  member cut schedule prints both and MCP reports them. Without this the
  cut list could not state end-cut orientation unambiguously.
- **Dimension-bound frame paths.** Commit `0e01612`. Sketch-line members
  read their sketch with the raw-geometry reader, which refuses sketches
  whose dimensions are solved or bound to parameters. A parameter-driven
  frame path could not become members. The structural checks now read only
  curve identities and kinds.

## Source and build

| Commit | Content |
| --- | --- |
| `042c098` | Long-point orientation in member records, cut groups, the member cut schedule and MCP |
| `0e01612` | Frame members from dimension-bound sketches |
| `0b3ad3c` | The scenario and its app test |

These commits are on the worktree branch `worktree-agent-a1688732253b64a20`
and are not merged.

The checks used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline --no-fail-fast` | 1,463 passed, 2 failed, 3 ignored |

The two failures are the unitless-DXF import tests
(`unitless_binary_and_malformed_dxf_fail_without_guessing` and
`artwork_import::tests::unitless_dxf_requires_an_explicit_unit_layers_filter_and_cancel_changes_nothing`).
In this worktree the fixture `crates/multicad-core/tests/fixtures/sketch_import/plate.dxf`
is checked out with LF line endings instead of CRLF. They pass on main and
are unrelated to this slice. The new `cabinetry_table` app test passes all
three tests.

## Evidence

### App test: `crates/multicad-app/tests/cabinetry_table.rs`

- **Frame.**
  - 13 member records. Top and base front rails are 1640 mm long point to
    long point, mitered 45° at both ends with the long point toward −Y
    (outside), and have the exact tube volume over their 1600 mm
    centerline.
  - Legs are 820 mm with square butt ends, starting 40 mm above the floor
    on the base rail's top face; the cross rail is 760 mm.
  - Cut groups: 4 × 1640 and 4 × 840 (both "same face"), 4 × 820 and
    1 × 760.
- **Welds.** 18 welds in authored order W1–W18 with steps 1–18. W5 sits
  at (800, 0, 880) and W15 at (60, 0, 0).
- **Panels.**
  - Worktop face: finished 1640 × 840, core 1636 × 839. Worktop core: core
    1643 × 843, grain free.
  - The stack touches face to face, 31 mm total.
  - Cabinet top: 18 mm, finished 464 × 760, core 759.5 across.
  - Every panel fits its sheet and has no review issue.
- **Finger joint.** The two upstand solids do not intersect, and their
  volumes equal both full panels minus one corner block and the four
  0.2 mm finger gaps.
- **Shelf pins.** Four blind holes centered on the inside face (x = 118),
  drilled toward −X.
- **Drawings.** The dimensions read 1640, 931 and 840. Every page passes
  `checked_page`. T-101, T-103 and T-104 have no layout conflicts. On T-102
  the weld symbols overlap their view and one another near the legs, by
  design; nothing on any page leaves the printable area.
- **Width edit.** `table_width` 1800:
  - the front rails are 1840 and the end rails stay 840;
  - W5 moves to (900, 0, 880) and W16 to (1740, 0, 0);
  - the worktop face and back upstand are 1840 long, and the finger joint
    still checks;
  - the width dimension reads 1840;
  - the fabrication cut list, member cut list and weld plan tables are
    reported stale; after `refresh_tables` none are, and the member list
    shows 4 × 1840.

  Undo restores the exact document and the 1640 dimension.
- **Thickness edit.** Birch ply 18 → 15:
  - the cabinet top is 15 mm and 470 × 760, the left upstand 15 mm, and the
    worktop face stays 12 mm;
  - the drilled left side has the 400 × 760 × 15 volume minus its holes;
  - the finger joint checks at 15 mm, and the legs stay 820;
  - the fabrication cut list is reported stale.

  Undo restores the exact document.
- **Refusal.** `table_depth` 30 leaves no cabinet depth and is refused
  before commit; the revision and document are unchanged.
- **Save and reopen.** The project reopens equal to the saved document,
  and its member cut list equals the live one.
- **Exports from the reopened project.**
  - The PDF has four pages.
  - The four drawing DXFs carry `$INSUNITS`, the 1640 and 931 dimension
    texts, the weld plan with "W18 135 MAG" and its sequence table, the cut
    list with the purchased feet, and the member list with long points.
  - 13 member STEP files read back with the members' exact volumes; no
    panel or foot is exported in that set.
  - 9 panel DXFs: each has one outer `CUT` loop whose extent equals the
    panel's finished size. The left side has four `CUT_INNER` hole loops.
    The back upstand outline has 12 vertices: the two notches left by
    fingers 2 and 4.
  - 9 panel STEP files read back with the panels' exact volumes.
- **CSV.**
  - The fabrication cut list has 7 panel rows, 2 worktop layer rows, 4
    member rows and the purchased feet with quantity 4.
  - The worktop face row carries its finished and core sizes.
  - The 1640 rail row reads `miter 45° -x / miter 45° -x, same face`.
  - The member cut list, BOM and weld schedule CSVs agree.
- **MCP.**
  - Through an owner that reviews every client, `set_parameter` on
    `table_width` stays pending.
  - After acceptance, the `schedule` tool's member cut list has 4 × 1840
    with "same face", `weld_schedule` puts W5 at x = 900, the panel cut
    list has the 1840 back upstand, and `history` names the
    `mcp:workshop` actor.

### Generated deliverables

`cargo run -p multicad-app --example cabinetry_table_review --locked --offline -- artifacts/acceptance/cabinetry-table-2026-10-29`
wrote these files. They are kept locally under that `artifacts/` path,
which Git ignores. The PDF and panel DXF files are inside `exports/`
bundles named after their export set and source hash.

| File | SHA-256 |
| --- | --- |
| `workshop-table.multicad` | `7c4db338a2ceae044278de49f57edac45d56318e35798cedea92290f56a5ac87` |
| `manifest.json` | `668762bfd583a8ee1b3a14870b03df4ac29bd0003f52957cfe50e08668ae4cbe` |
| Four-page drawings PDF | `ccdd7c65c8a56b60709f06f743d0782ee4791d1883402be8c591b7fb5ec8e718` |
| T-101 assembly DXF | `3c0b6006ae6c9d3a521a8c02ddce12fdc200a9aa2bbc3b1800bf97fea3c450db` |
| T-102 weld plan DXF | `627cef15044f947b45b95af63e486c23d98b6fc75b49b4a865c21e27febfdb62` |
| T-103 cut lists DXF | `fa08bb5d28f0fa0c6358dd816c5806ef1acbf3b6a3dd10f23f42f54043930021` |
| T-104 members and BOM DXF | `c4677161bfb1b0a2599da6cbde3eab7c0d0c46cf9578e1e80de5fc54bc7820dd` |
| `schedules/fabrication-cut-list.csv` | `50bc1e5e73cef85d9885078de77b14aae6819d1343a0826a15feb48dd5f765e0` |
| `schedules/panel-cut-list.csv` | `f7f83a2708371239cbbe4697163f0c928dcba8847decce81607918db1c28be41` |
| `schedules/machining.csv` | `052f0887010c1147c998d5655b16474cc0fcb1493be867c90ab8413eae0beed8` |
| `schedules/member-cut-list.csv` | `9aafd1cf80314f9fb73d43e8af6fd0bcc8a2e5a28477fa1355872ff77af88a61` |
| `schedules/weld-schedule.csv` | `e39fba7df44321c9649c0e30b895b034e3ffe4eb9195a71658b41243dfe62ba1` |
| `schedules/bom.csv` | `f4a5b7f215b7d56c423a29e71bbe0dd8d8f1a145bf98d87ade2ad88925312147` |
| Left side panel DXF (four holes) | `a9a3260d0d6911d79def911a554174725aef0946fb536b9c833cf22469a5b837` |
| Back upstand panel DXF (fingers) | `7da2a9d57f19b5d175d2448a3e15a9e5540f03e14ca9914204e6648a8f2bb7ff` |
| Member and panel STEP, other panel DXFs | 13 member STEP, 9 panel STEP and 7 more panel DXF files; every hash is in `manifest.json` |

Project and export file names contain fresh entity IDs, so a new run
writes different names and hashes.

The PDF was rendered with `pdftoppm` and inspected:

- T-101 shows the front view (frame, cabinet with its shelf, worktop,
  upstand, feet) and the top view, with the three dimensions;
- T-102 shows the isometric frame with 18 weld symbols and the sequence
  table. Near the legs, symbols 60 mm apart in the model are 6 mm apart on
  paper at 1:10, so their tails overlap and are hard to read there;
- T-103 and T-104 show the four tables and the certification note.

## Operator review path (macOS, pending)

1. Generate the project with the command above into a new directory, then
   open `workshop-table.multicad` in the MultiCAD window.
2. **Parameters.** Change `table_width` to 1800 and Apply/Rebuild. The
   frame, worktop, back upstand, feet and the T-101 width dimension should
   follow. Undo.
3. **Model.**
   - Model → Materials: change "Birch ply 18" to 15 mm and check the
     cabinet and upstands. Undo.
   - Open a top rail and the "Top corner 1" connection (Miter / cope) and a
     weld (Weld joint), and the sequence order.
   - Open "Upstand corner" (Panel joint) and "Shelf pins" (Drill / pocket).
4. **Drawings.** Open the four pages and compare them with the exported
   PDF.
5. **Export.** Run "Table frame and drawings" and "Panel cutting", and open
   the PDF, DXF and STEP files in other applications.
6. **MCP.** Connect a client with `multicad --mcp --app`, call
   `set_parameter` on `table_width`, accept it in the review queue, then
   call `schedule` with `member_cut_list` and `weld_schedule`.

## Not verified

- **Interactive check.** None of the operator steps above has been
  performed by a human on macOS.
- **Tables are not live.** Placed schedule tables and weld plans are page
  content. After an edit they are reported stale by `check_table` and the
  example's `stale_tables` until they are placed again (`refresh_tables`
  in the example). No window command places or refreshes them.
- **Fabrication cut list.** The combined cut list is assembled in the
  example from the shared records; there is no shared builder or MCP kind
  for it. Export sets have no CSV recipe; CSV files are written by the
  example (or the MCP `schedule` tool).
- **Construction assumptions.** The cabinet hangs under the top frame
  without modeled fixings; there are no screws, brackets, worktop
  fasteners or weld beads. SHS corners are sharp (no corner radius), so
  steel volumes are slightly high. Finger joint fit and band allowances are
  nominal shop values, not validated tolerances.
- **External applications.** The PDF, DXF and STEP files were checked by
  the test's readers and one PDF rendering, not in external CAD, CAM or
  print applications.
- **Other platforms.** Windows and Linux were not run.
- **Not certified.** Member lengths, welds, the weld order and the cut
  lists are geometric and documentation aids. They are not a structural,
  load, welding-procedure, distortion or fabrication approval.
- **Acceptance.** Agent completion is not the user's workflow acceptance.
