# Native material properties checkpoint, 2026-10-28

The production macOS app edited reusable plywood hatch defaults, assigned an
independent appearance override and added tags to two genuine solid parts.
Fresh native regeneration preserved both parts' geometry.

## Executable and automated evidence

The bundle and combined automated checks are recorded in the
[drawing pages checkpoint](native-drawing-pages-2026-10-28.md).
Material behavior is pinned by `c7dfb92`, `7c585b8`, `3a59999`, `793d284` and
`704d1fb`. The resolution and invalidation contracts are described in
[manufacturing properties](../architecture/manufacturing-properties.md).

The input was `artifacts/model-review/materials-native.multicad`, containing
Shelf and Cabinet side. Both parts used Birch plywood, a reusable 18 mm panel
stock definition and tags `cabinet` and `cnc`. The Inspector showed stock size
2440 × 1220 mm with grain along its length.

## Linked hatch, appearance and tag edits

The material library changed the Plywood section hatch from 45° and 3 mm spacing
to 30° and 5 mm spacing on paper. Apply updated the selected part's Inspector
and displayed a changed-definition warning. Acknowledge current defaults on
Birch plywood, followed by Apply, removed that warning while retaining the
linked definitions.

A reusable appearance named “Cool preview finish” was created and assigned only
to Shelf. Selecting Cabinet side showed the unselected Shelf with its cool
appearance. Cabinet side retained its warm inherited appearance. Manufacturing
material, stock and section hatch stayed assigned independently.

Selecting both parts and using Add tags appended `manufacturing-review` and
`maker-space` in one transaction. Shared tags included both additions.
One Undo removed both additions from both parts; Redo restored them. Pending
property edits disabled Save, document history and conflicting modeling tools.

Save As wrote `artifacts/model-review/materials-edited-native.multicad` at
revision 7. The production reader rebuilt both the input and edited file with:

```
cargo run -p multicad-app --example model_file_review --locked --offline -- INPUT.multicad OUTPUT-report.json
```

The corresponding `materials-native-report.json` and
`materials-edited-native-report.json` contain identical body identities,
producers, bounds and volumes. Rounded volumes are 3,654,000 mm³ for Shelf and
4,320,000 mm³ for Cabinet side. These reports use exact native solids; meshes
are not manufacturing input.

The review files and startup tab were explicitly saved before Quit, and inventory
confirmed every MultiCAD review instance stopped. Interactive reopen of the edited
material file is still pending; the native reader successfully loaded it.
Drawing hatch rendering, export tag filtering, physical Windows/Linux behavior
and human acceptance are outside this checkpoint.
