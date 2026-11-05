# Native drawing navigation and PDF verification, 2026-11-05

Actual macOS interaction verified the project-wide drawing sidebar, draft-safe
navigation, structural updates and direct PDF export. The three exported scopes
contain the expected one, two and three pages. Independent checks confirm their
titles, numbering and native vector geometry. Human acceptance remains separate.

This supplies the window-interaction evidence missing from the
[historical report](drawing-navigation-2026-11-03.md).
The sidebar is implemented in
[`sidebar.rs`](../../crates/multicad-ui/src/drawings/sidebar.rs), with the shared
PDF dialog in
[`dialog.rs`](../../crates/multicad-ui/src/exchange/drawing_pdf/dialog.rs).
The owned evidence root is `artifacts/drawing-navigation-review-20261105/`.

## Sheet selection and native PDF export

The [baseline audit](../../artifacts/drawing-navigation-review-20261105/independent-baseline.json)
records Construction with A1 Plate overview and A2 Plate manufacturing, plus
Hardware with H1 Fastener schedule. Each saved Top projection references the
same native 60 × 40 × 8 mm plate, with volume 19,200 mm³.
An owned copy of the historical project supplied the native-window checks.

Drawings listed both books and all three sheets in the left sidebar.
Selecting each entry synchronized its highlight, canvas and Drawing Inspector.
Switching back to A1 preserved revision 0.
Export as PDF opened directly from the sidebar and showed the selected scope,
listed sheet titles and destination before writing.

Canceling Choose folder returned without writing.
A nonexistent absolute output path produced a readable missing-folder message
and disabled Export PDF. An existing owned folder enabled export.
The following three jobs completed through the native dialog:

| Selected scope from A1 | PDF pages | Included titles |
| --- | ---: | --- |
| Current sheet | 1 | Plate overview |
| All sheets in this drawing | 2 | Plate overview; Plate manufacturing |
| All project sheets | 3 | Plate overview; Plate manufacturing; Fastener schedule |

Each completion displayed its page count, source revision and full managed
output path. Escape closed the completed dialog.
The original copied project retained SHA-256
`2f87411c4c2b8e57badfcd724590ceae891d3e8dccda12a49ad7defb92dff918`
through navigation, exports and canceled drafts.

The [independent PDF review](../../artifacts/drawing-navigation-review-20261105/independent-native-pdf-review.json)
checks the actual GUI-written files, their selected title inclusion/exclusion,
page order and per-book numbering. All six exported pages have the four native
projection edges of a 60 × 40 mm rectangle at 1:1 scale.
Measured coordinates agree with the canonical view equations within
0.000500001 PDF points, the checked serialization allowance.
All six Poppler-rendered pages were inspected: notes, title blocks and full
frames remain readable without material clipping or overlap.
These checks distinguish the native vectors from the embedded logo image.

## Draft safety and structural updates

Typing Unapplied title review into A1 changed its local draft and marked its
sidebar row. Other sheet entries became disabled with an apply/discard message.
The direct PDF dialog refused export and explained that Cancel retains edits.
Escape closed that dialog while preserving the draft text.
Discard edits restored the saved title and enabled navigation at revision 0.

A separate owned project checked structural edits after the original exports.
New drawing created Drawing 1 and its first Untitled sheet at revision 1.
Add page created its second sheet at revision 2.
Applying the title Sidebar edit review updated that row at revision 3.
Undoing the title and page addition produced revisions 4 and 5; the added row
disappeared and the sidebar count returned to four sheets.
Redo restored the page and title at revisions 6 and 7.
Save retained three drawings and five sheets at revision 7.

The [structural audit](../../artifacts/drawing-navigation-review-20261105/independent-native-structural-readback.json)
confirms the added book and two pages, including their stable identities.
Every original entity, view binding and native plate remains unchanged.
Fresh launch and the exact Recent entry reopened the new sheet with its title
and correct Inspector at revision 0. The native check removed the added row
through Undo; the direct Remove page command retains automated coverage.

An empty new project showed 0 drawings, 0 sheets and No drawings yet, with
disabled sidebar PDF export and a New drawing action.
Save and Quit retained this review project in the owned folder before closing.
It supplied an empty-project check, rather than an empty book containing no
pages, which the public fixture API did not author.

## Long lists, compact layout and shared Export

A [typed fixture preparation](../../artifacts/drawing-navigation-review-20261105/independent-long-sidebar-preparation.json)
added 58 source-linked elevation pages to Construction, preserving its original
two sheets and the Hardware book. The resulting list contained 60 Construction
sheets and 61 project sheets. At 1100 × 700 logical size, actual scrolling reached
the final Construction entry and returned to A1 without source changes.

The final entry's title is Long sidebar sheet 60 followed by Plate elevation;
its generated page number is 58. Its selected row, canvas and Construction
Inspector agreed. The first two pages retain their original A1/A2 numbers.
Long titles wrapped within the sidebar. The source hash remained unchanged
after scrolling and selection.

The original three-sheet project also switched sheets at compact size.
The direct PDF modal kept scope, destination and action controls within the
window. Normal captures are 2720 × 1720 pixels; compact captures are
2200 × 1400, corresponding to the current minimum 1100 × 700 logical size.
The smaller 1024 × 600 target remains part of the final UI consistency work.

Export → Common → New export set still offered Multi-page PDF and All current
pages in its local recipe draft. Discard edits preserved source at revision 0.
No second saved export set or PDF implementation was introduced by verification.
The [capture manifest](../../artifacts/drawing-navigation-review-20261105/capture-manifest.json)
indexes 32 JPEG/accessibility pairs for these native checks.

The [independent visual review](../../artifacts/drawing-navigation-review-20261105/independent-drawing-navigation-visual-review.json)
inspected 12 actual frames and found no material layout defect.
Compact PDF actions remain visible; lower Inspector fields require scrolling,
and the long destination input scrolls horizontally.

## Checks and evidence limits

No Rust changes were required. The runtime is the optimized bundle built from
`3a9da7a`, executable SHA-256
`e2bc46957fd6365871d9137e9791bfa1899171e23f50de3ea057f795d1f073f9`, at
`artifacts/wood-navigation-review-20261105/optimized-final/MultiCAD.app`.
Diagnostics reports the source commit as not stamped; this is local evidence.

The unchanged Rust tree reuses the completed
[required-check receipts](../../artifacts/wood-navigation-review-20261105/checks/summary.json):
formatting, strict locked all-target workspace Clippy, release build, `./build`,
version and diagnostics passed. `cargo test --workspace --locked` exited zero:
2591 passed, zero failed and four intentional ignores across 295 result groups.
Tests used optimization level 3 with debug assertions and overflow checks.
The UI group passed 961 tests with two GPU ignores in 63.63 seconds.

Deterministic job cancellation, late publication attribution and checking-stage
close guards retain their controlled regression coverage.
Interactive cancellation during a native write was not tested on these small
jobs. No cancellation-race result or new timing guarantee is claimed here.
The final [running-window inventory](../../artifacts/drawing-navigation-review-20261105/closed-review-windows.json)
is empty. Windows/Linux interaction and human acceptance remain separate.
No installation, push or publication was performed. Work stays local on main,
preserving unrelated `.claude/` content.
