# Native feature editing checkpoint, 2026-10-28

The production macOS app edited an upstream Box and a consumed Cylinder,
rebuilt their downstream cut, rejected an empty result and changed Boolean tool
retention. Saved projects regenerated with their original identities. This is
implementation evidence, not human acceptance of the complete workspace.

## Reviewed executables

The local application was
`artifacts/development/build-dxiY6X/MultiCAD.app`. Its executable SHA-256 was:

```
8d279ea1f85a78a8b13c2569c73f797ce139a1528b19c8c99cce311b3d02d123
```

It includes the applied-feature UI, source-bound metadata adapter and preview
Fit correction. It precedes the interactive extrusion-operation and termination
controls. Native Split source was present during packaging, but no Split UI
acceptance is claimed.

Independent saved-file reports used the already compiled `model_file_review`
executable with SHA-256:

```
d8ee084f3d9997796892587dbaf8172d9047471c9aa4c0ba83f5abcab4fd9a6b
```

This reader supports the primitive, revolve and Boolean source in these fixtures.
It was executed afresh for each saved project. No installation or publication
occurred.

## Applied edits and failure handling

1. Open `direct-solids-and-revolve.multicad` through the native File dialog.
   Both final solids regenerate at revision 0.
2. Select the upstream Box feature and use Inspector's Edit feature. Set width
   from 80 to 90 mm. Preview succeeds and displays the downstream cut.
3. Change width to 91 mm. Apply becomes disabled. Restore 90 mm, Preview again
   and Apply at revision 1. Undo restores revision 2; Redo restores the edit at
   revision 3 without creating replacement Parts or Features.
4. Select the consumed Cylinder feature and Edit feature. Set radius from 5 to
   6 mm. Native Preview succeeds despite the tool having no independent final
   scene instance.
5. Set radius to 100 mm and Preview. The native compiler reports that the Boolean
   removed all material. Apply stays disabled and the document remains at
   revision 3. Restore 6 mm, Preview and Apply at revision 4.
6. Edit the Boolean feature. Its saved Box target and Cylinder producer versions
   appear read-only. Enable Keep tool bodies, Preview and Apply at revision 5.
   Save As writes `feature-edit-keep-tools.multicad`.
7. Undo that retention edit at revision 6, preserving both dimension edits.
   Save As writes `feature-edits.multicad`.

Both files and their independent reports are under `artifacts/model-review/`.
They preserve the original document, Box Part, Boolean producer and revolved Part
identities from the [solid tools checkpoint](native-solid-tools-2026-10-28.md).

| Saved state | Final bodies | Cut Box volume, mm³ | Tool state |
|---|---:|---:|---|
| `feature-edits.multicad` | 2 | 105,738.05328941533 | Consumed Cylinder retained as source |
| `feature-edit-keep-tools.multicad` | 3 | 105,738.05328941533 | Independent Cylinder, 4,523.893421169292 mm³ |

The Box bounds are (0, 0, 0) to (90, 60, 20) mm. Its volume matches
108,000 minus 720π mm³. The retained Cylinder bounds are (14, 14, -10) to
(26, 26, 30) mm, matching radius 6 and height 40. The revolved part remains
37,699.11184307742 mm³ in both reports. Each final body contains one native solid.

## Preview Fit and closure

With the 400 mm² sketch profile selected, a new -180 degree revolve Preview
succeeded. Home followed by Fit framed the displayed candidate and other final
Parts within the viewport, without zooming into the source rectangle. Cancel
returned to the saved revision 6; no new revolve was committed.

The initial empty tab was saved separately as `edit-startup-empty.multicad`.
The application quit, and inventory confirmed every MultiCAD review instance
was closed. Interactive reopen of the two new edit files was not performed;
their saved-source regeneration is the evidence above.

## Remaining evidence

Current Boolean editing preserves target/tool producer versions and exposes
operation and retention controls. Replacing those inputs requires a separate
workflow. Viewer-picked extrusion limits, interactive taper, automatic cut scope,
Split and manufacturing exports require subsequent verification. No physical
Windows/Linux or low-power performance result is claimed.
