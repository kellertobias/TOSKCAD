# ADR-0032: Keep Wood Box faces as recipe-backed panels

| | |
| --- | --- |
| Status | Implementation contract; verification pending |
| Date | 2026-11-02 |
| Scope | CAD-249 |

The existing carcass has sides, top, bottom, back and shelves. It has no front
role and does not represent arbitrary missing faces. Keep that source contract
unchanged. Add one Wood Box recipe with six named, independently optional faces.
Its members use the ordinary panel feature and existing panel/joint/machining
infrastructure, with a versioned recipe-backed panel source.

The recipe owns outside width/depth/height, world placement, a shared panel-stock
reference, face presence and stable identities for each face. Stock supplies the
thickness and material. The document's shared stock/material definitions remain
authoritative; the assistant does not introduce a separate material library.
Allocate face identities before Preview and preserve retained identities through
edits. Generated panels resolve geometry directly from the recipe, rather than
storing detached rectangle copies.

Use an explicit butt-joint policy: present left/right sides span full depth and
height; front/back fit between present sides; top/bottom fit between the present
vertical faces. Missing faces remove their thickness allowance. This partitions
corner material without overlap. Positive outside and clear dimensions are
required. An empty face set is invalid. A nonempty subset remains a valid starting
shape, including a single panel.

Outside dimensions describe the design envelope. Clear dimensions subtract the
thickness of present opposing faces. Actual assembly bounds are derived from the
enabled panels and may span less than the design envelope when only parallel
faces remain. The preview and inspection must distinguish those facts.

Recipe edits update retained panels in place, including supported downstream
operations that reference their producers. Native preflight must reject a changed
recipe when a downstream operation becomes invalid. Removing a referenced face
requires explicit dependency repair, without partial mutation or stale copies.
Panel modifiers remain panel data; replacing the recipe-backed source with an
unrelated source is an ownership conflict.

Use one shared transactional API for graphical and MCP flows. Preview/Cancel
leave the document unchanged, Apply is atomic, and Undo/Redo/save/reopen retain
the complete source. New capabilities refuse unsupported readers. Existing
rectangle/profile/carcass panel files retain their version and behavior.

Verification must cover open-top five-panel output, an omitted side, multiple
omissions, dimensions/material/stock changes, retained identities, downstream
machining/joints, removal conflicts, native pair intersections and export
readback. Production UI captures and required Rust checks are separate from
actual macOS window interaction and human acceptance.
