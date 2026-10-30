# Workbench icon review assets

These editable SVG proposals follow the ToskLight icon family's 64 by 64 canvas, 3 px rounded strokes and `currentColor` artwork. Each source contains an accessible title. The shared finish check defaults to green. The native application still draws its existing Rust symbols; these assets are not connected to native commands.

`catalog.json` lists labels, workbench groups, proposal status and retained native motifs. An asset for a proposed tool does not establish that tool's availability.

Generate labeled review sheets with `node tools/icon-review.mjs`. Add `--png` when `rsvg-convert` is available to render PNG copies. Output goes to the ignored `artifacts/icon-review/` directory; open `index.html` to review all groups. Each card includes 64 px artwork and 24/16 px samples on dark and light surfaces. Edit the individual source SVGs, then regenerate the sheets.
