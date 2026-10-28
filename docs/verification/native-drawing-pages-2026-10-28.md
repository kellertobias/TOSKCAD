# Native drawing pages checkpoint, 2026-10-28

The production macOS app saved and reopened two independent drawing pages with
paper settings, title information and a page-specific text note. These checks
cover page authoring; model views and drawing exports remain separate work.

## Executable and automated evidence

The final review bundle was `artifacts/development/build-4gttxJ/MultiCAD.app`,
built from `704d1fb`. Its executable SHA-256 was
`bab90d55d151d126d3e4feae2bf2b0142f3787485783e6110ccd0dcf6ccb3279`.
The drawing implementation is pinned by `10a2d24`, `8bd00fd`, `063207e` and
`8b520fd`.

Formatting, all-target Clippy with warnings denied and locked offline workspace
tests passed: 751 passed, zero failed, two existing ignored. The commands were
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`.
Logs are `/private/tmp/multicad-materials-drawings-{fmt,clippy,tests}.log`.

## Native page authoring and reopen

Add page created an empty drawing book and its first page. The Inspector accepted
page A101 with A3 portrait paper, third-angle projection, scale 1:20 and a
12.5 mm margin. Its title was “MakerSpace ground floor and equipment layout”.
Author, project, revision and checker fields contained Tobias, MakerSpace, A
and TK respectively.

Duplicate page created an independent page, renamed A102 with the title
“Equipment connection details”. Switching pages preserved each title. Moving
A101 later changed their order; one Undo and Redo restored each ordering.
The note “All dimensions in mm. Model views will be added separately.” appeared
only on A101 at paper position (17.5, 17.5) mm.

Save As wrote `artifacts/model-review/drawing-pages-native.multicad` at revision
8. A fresh app session reopened A102 first and A101 second, with their independent
titles, settings and note intact. The checked source report is
`artifacts/model-review/drawing-pages-native-source-report.json`.

An oversized title and author produced an explicit title-block fit error.
Both the Inspector and ribbon Apply controls were disabled in the final bundle.
Discard restored the saved page without a source revision change. The shared
validation guard has an automated regression in
[`drawings/tests.rs`](../../crates/multicad-ui/src/drawings/tests.rs).

All open review documents were saved before Quit. Application inventory confirmed
that every MultiCAD review instance had stopped. This checkpoint does not establish
model projection, PDF/DXF output, Windows/Linux interaction, low-power performance
or human acceptance.
