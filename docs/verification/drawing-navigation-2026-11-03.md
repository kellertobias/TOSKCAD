# Drawing navigation and direct PDF verification, 2026-11-03

> Historical implementation evidence. The [2026-11-05 native verification](drawing-navigation-2026-11-05.md) supplies the later actual macOS window checks.

CAD-226 adds a project-wide drawing/sheet sidebar and scoped PDF export inside
Drawings. Automated UI, native PDF and offscreen layout evidence is recorded
below. The required actual macOS window interaction check remains unverified;
no human acceptance is recorded.

## Source and behavior

Work is local on `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Scope |
| --- | --- |
| `d94d72b` | Drawing sidebar, shared PDF controller/dialog, source and draft guards, automated UI coverage and architecture |
| `aa3f910` | Native PDF scope fixture and production UI capture tools |
| `deb7623` | Borrowed destination comparison for strict Clippy |
| `1e10dcb` | Keep close guards through accepted PDF checking; final runtime verification source |

The scrolling sidebar lists every drawing book and its sheets, with the current
book/sheet highlighted. Entries show sheet number and title, wrap long names,
refresh after structural edits and describe empty drawings. New drawings receive
distinct names. Navigation clears obsolete request/selection state while
retaining completed page cache entries. Pending edits or placements block sheet
changes, so their content cannot move to a different sheet.

Export as PDF is available in the sidebar, Drawings ribbon and tool finder.
The dialog lists the chosen sheets and destination before writing. Current
sheet, current drawing and all-project scopes produce one PDF. It shares
Export's native backend, destination picker, publisher and output history;
saved Export sets still support PDF. A transient prepared output-set overlay
does not become canonical source. Apply/discard or finish/cancel remains
necessary before exporting drawing drafts. Cancelling retains those drafts.
Late successful publication is explicitly reported when it beats cancellation.

The controller validates the originating session/source, actor, selected pages
and destination before writing. Publication receipts must match the accepted
preview and selected page scope. Failures retain an actionable result.

## Automated UI evidence

The whole UI crate passed **917 tests, zero failures and two ignored** in
25.06 seconds on the final close-guard change. Coverage includes
painted selection across books, scrolling through 60 sheets to select the last,
structural refresh, empty states, retained preview cache and pending placement
guards. A production Application test opens the modal through its actual
sidebar button, checks scope/location controls, closes it, then presses Escape
with an unapplied note and verifies that the retained note can still be applied.

Controller tests exercise all three scopes, unchanged source and saved-set
drafts, cancellation before writing, cancellation with late publication, stale
source/session, foreign actors, changed destinations, mismatched page scope and
incorrect publication receipts. A close-event regression holds the checking
reply on a clean saved drawing, requires CancelClose, and keeps cancellation
busy until acknowledgement. The cancelled job cannot start a write; closing
then becomes available without source changes. These controlled adapters prove controller
behavior; the following checks use the production native backend.

## Native PDF evidence

`crates/multicad-app/examples/drawing_navigation_review.rs` produces
`artifacts/drawing-navigation-review-20261103/`. Its saved `drawings.tcd` defines two
books, three titled sheets and top views of a 60 × 40 × 8 mm plate. Native
export generates their OCCT projections. The public ExportWorkspace controller uses the production ExportWorker,
Exporter and existing PDF publisher. Canonical document and revision stay
unchanged through all exports.

| Selected scope | PDF pages | End-to-end time on this small fixture |
| --- | ---: | ---: |
| Current sheet | 1 | 72.37 ms |
| Current drawing | 2 | 48.16 ms |
| All project sheets | 3 | 39.22 ms |

Independent Poppler `pdfinfo` confirms those page counts. `pdftotext -layout`
confirms Plate overview, Plate manufacturing and Fastener schedule appear only
in their selected scopes. `independent-pdf-checks.json`, per-scope info/text
files and `native-report.json` retain the results. All three project-PDF pages
were rendered with Poppler and visually reviewed: native projection, authored
note, title block and per-book sheet numbering are visible and unclipped.
These timings describe this fixture, not a large-model performance guarantee.

## Production layout evidence

Nine GPU-rendered production captures in `captures-final/` cover the first book,
the other book and the PDF dialog at 1100 × 700 / 1× and 1360 × 860 / 1× and 2×.
All nine were visually reviewed. Sidebar actions and current selection fit;
the selected sheet and inspector agree; the PDF scope/location controls and
buttons remain within the dialog. The helper explicitly selects the first book
and waits for the opening animation to settle. Navigation leaves source hash
and revision unchanged. These offscreen checks create no OS windows and do not
prove actual macOS window interaction.

## Required checks and build

Final verification on `1e10dcb` passes:

- `cargo fmt --all -- --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`, 2m 51s.
- `cargo test --workspace --locked`: **2339 passed, zero failures, four ignored**,
  across 261 result groups. Compilation took 3m 17s. The UI group passed
  **917 tests, zero failures, two ignored** in 25.19s.
- `./build`, 27.44s. Native bundle:
  `artifacts/development/build-8TeUDj/MultiCAD.app`.
- The bundled `Contents/MacOS/multicad --version` and `--diagnostics` both
  exited zero. They report ToskCAD 0.1.0-dev, macOS/aarch64, static OCCT 7.9.3,
  eframe 0.36.2/wgpu and a compiled Rust/native runtime. The source commit is
  not stamped; the diagnostics workspace summary is legacy descriptive text,
  not workflow acceptance evidence.

Final logs are `/tmp/multicad-cad226-{fmt,clippy,workspace,build}.log`, with
separate version/diagnostics logs under the same prefix. The pipeline exited
zero on 2026-11-03 at 01:48 UTC.

The first strict Clippy run rejected an unnecessary owned path comparison;
`deb7623` corrected it. Its initial workspace pass was intentionally stopped
after an independent review found the checking-stage close gap. That
interrupted pass is not final verification evidence. `1e10dcb` fixes the gap
and adds the close-event regression; all required checks above were repeated
on that source. An independent evidence review also reran read-only Poppler
page-count and title checks successfully.

## Remaining acceptance

An actual native macOS check must switch among multiple drawings and export
through Drawings. Desktop interaction tools are unavailable, so this explicit
CAD-226 criterion is still open. Keep the item Ready after automated verification
until that check is recorded; do not infer Review/test, Done or human acceptance
from the tests, native PDFs or GPU captures. Windows/Linux runtime behavior is
also unverified here. No application installation, push or publication was
performed. Existing user windows and unrelated `.claude/` content are preserved.
