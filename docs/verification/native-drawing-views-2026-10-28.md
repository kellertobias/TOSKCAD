# Native drawing views checkpoint, 2026-10-28

The production macOS application authored, saved and freshly reopened two pages
containing four source-linked native views. The production reader regenerated
all four views as Ready from the saved canonical project. These results record
agent verification; human acceptance remains separate.

## Executable evidence

| Check | Source | Review bundle |
| --- | --- | --- |
| Authoring and Save As | `4d2c0b2` | `artifacts/development/build-RBdBvS/MultiCAD.app` |
| Fresh reopen and preview controls | `edf925b` | `artifacts/development/build-1VdX8T/MultiCAD.app` |

The authoring executable SHA-256 was
`8e8ba67ec8bc30cb39ce5c985500cda522669ce3268dfc807ade6b21c8d329c1`.
The fresh-reopen executable SHA-256 was
`89d25c55383f3a8393190f9025ad2b45fe31759eff2fd2eeac260df627d3d6c0`.
Both hashes identify `Contents/MacOS/multicad` inside the corresponding bundle.

## Authored pages and native vectors

The source was Cabinet side, a real 600 × 400 × 18 mm Birch plywood part.
Both pages used landscape paper, first-angle projection, a 10 mm margin,
author Tobias and project MakerSpace.

| Page | Paper | Title |
| --- | --- | --- |
| D101 | A3 | Cabinet panel views |
| D102 | A4 | Panel isometric and detail |

The Inspector authored the following views. Positions are paper coordinates in
millimeters; origins, section offsets and crop dimensions are model millimeters.
Each scale is paper:model. Counts come from the saved native vector report.

| Page and view | Configuration | Ready vector counts |
| --- | --- | --- |
| D101, Panel top | Top frame, origin (300, 200, 0); scale 1:5; position (130, 80) | 4 visible |
| D101, Panel section | Same top frame and origin; section offset 10; scale 1:5; position (130, 200) | 4 visible, 4 section, 25 hatch |
| D102, Panel isometric | Isometric frame; scale 1:6; position (90, 70); hidden edges enabled | 9 visible, 3 hidden |
| D102, Corner detail | Top frame, origin (0, 0, 0); crop center (0, 0), radius 25; scale 1:1; position (220, 135) | 2 finite clipped lines |

The section used the material's 30° hatch with 5 mm spacing on paper.
Undo removed Corner detail; Redo restored it. Save As wrote
`artifacts/model-review/drawing-views-native.multicad` at revision 10.

The reader in commit `853859c` loaded that project through the production store,
compiled its exact B-reps and checked every page against the current source stamp:

```sh
cargo run -p multicad-app --example drawing_file_review --locked --offline -- \
  artifacts/model-review/drawing-views-native.multicad \
  artifacts/model-review/drawing-views-native-report.json
```

The report retains exact curve records separately from preview paths. Repeating
the command requires a new output path because the helper refuses overwrites.

## Fresh reopen and failure guards

A fresh native session reopened the saved project at revision 0. Page settings,
titles and all view controls persisted. The section, isometric and detail views
visually displayed their genuine hatch, hidden edges and clipped geometry.
Their native vector totals were 33, 12 and 2 respectively.

A temporary name edit enabled Apply after native evaluation completed, without
another input event. Discard restored the source while retaining revision 0.
Commit `a8e0937` has the asynchronous control-refresh regression and an automated
Apply/Undo regression for removing the final view. Final-view removal was not
physically retested. The oblique section regression in `617435e` verifies hatch
segments inside a real triangular cut face and outside its circular bore.

## Automated evidence and limits

The integrated checkpoint passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`: 865 passed, zero failed, two ignored.
Logs are `/private/tmp/multicad-native-workflow-integrated-{fmt,clippy,tests}.log`.
The native vendor deprecation warning remains outside the Rust lint scope.

The [drawing contract](../architecture/drawing-views.md) describes exact vector
production and registered source providers. Flat-pattern and exploded providers
have native contract fixtures; their producing workspaces are not established
by this check. PDF/DXF writers and annotations remain separate work. Detail
captions and crop borders are not automatic. Windows/Linux interaction,
low-power performance and human acceptance were not established. Review
applications remained open at the end of these checks.
