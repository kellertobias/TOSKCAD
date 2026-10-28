# Native operation conversion checkpoint, 2026-10-28

The production macOS app converted a saved two-target Cut into an independent
extrusion, reopened it, and converted it back using explicitly selected targets.
Native regeneration confirms stock restoration and the resulting cuts.

## Executable and automated evidence

The reviewed bundle was `artifacts/development/build-mcZuvX/MultiCAD.app`, with
the conversion implementation pinned by `1d17cc1`, `6666926` and `b9ed7f4`.
Its executable SHA-256:

```
fa98c110292207512fa10365e9f8df2086dac45d0cee53f820be02ee5f8c407e
```

Formatting, all-target Clippy with warnings denied and locked offline workspace
tests passed: **718 passed, two existing ignored**. Commands were
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`.
Logs are `/private/tmp/multicad-conversion-integrated-{fmt,clippy,tests}.log`.

The [native conversion tests](../../crates/multicad-model/tests/extrude_group_conversion.rs)
cover save/reopen, parameter changes, identity preservation, Undo/Redo and explicit
adoption of one legacy producer without guessing its same-sketch siblings.
The UI controller tests cover plain legacy edits, pinned creation identities and
operation conversion. Those cases establish automated behavior; the interactive
checks below cover the saved multi-target group.

## Cut to New body, then reopen

Open `group-edited-native.multicad` from the
[captured-group checkpoint](native-extrusion-group-2026-10-28.md) in a fresh
session. Both edited cuts regenerate. Select A visible panel and Edit latest
feature, then choose New body. Preview validates the restored stocks and
independent extrusion; Apply reaches revision 1.

Save As writes `group-newbody-native.multicad`. One Undo restores both cuts at
revision 2; Redo returns to the independent body at revision 3. Explicit Save
persists that state. Save the empty startup tab separately and Quit. Inventory
confirms every MultiCAD review instance stopped.

Relaunch the same bundle and open the saved New body file. Model shows the stock
panels and extrusion. Edit latest feature on the extrusion opens the same
operation with start offset 2 mm and distance 85 mm intact.

## Explicit conversion back to Cut

Choose Cut. The affected-body count starts at zero; every candidate target is
unchecked. Explicitly select A visible panel and B foreign panel. Preview
validates the candidate, and Apply creates their new Boolean results at revision
1 in this fresh session. Save As writes `group-recut-native.multicad`.

Fresh reports give these rounded volumes, in mm³:

| Part | New body | Reconverted Cut |
|---|---:|---:|
| A visible panel | 48,000 | 44,800 |
| B foreign panel | 48,000 | 45,200 |
| A grouped, A hidden, Nested and Document root panels, each | 48,000 | 48,000 |
| Extruded part 1 | 34,000 | Consumed source body |

The independent extrusion bounds are (10, 10, 2) to (30, 30, 87) mm.
Its producer remains `447a773e-2f99-43b0-b86d-1314a35bab19` throughout.
Source inspection confirms operation identity
`56598823-feed-47b7-8825-ed80b360eb29` is unchanged. Its schema changes from 1 to
2 for New body and stays 2 when converted back. Removed Boolean identities are
absent; the explicitly added targets receive new result features.

## Artifacts and limits

All files and `*-report.json` outputs are ignored local artifacts under
`artifacts/model-review/`. Fresh production-native regeneration uses
[`model_file_review`](../../crates/multicad-app/examples/model_file_review.rs),
rebuilt against the conversion core. Its executable SHA-256 was
`bbcdc873f70aaed38206ea4ace7e94803547a9f098566da5427e5ae93455441b`.
Source-schema inspection follows the native project codec's checked manifest and
canonical source layout. Reports establish native solids, bounds and volumes;
display meshes are not manufacturing input.

Both tabs were explicitly saved before the final Quit. Inventory confirms every
review instance stopped. Interactive legacy adoption and interactive reopen of
the final reconverted Cut were not performed; their automated roundtrip coverage
is separate. Human acceptance, manufacturing exports, physical Windows/Linux
behavior and low-power performance remain outside this checkpoint.
