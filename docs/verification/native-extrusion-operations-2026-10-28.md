# Native extrusion operations checkpoint, 2026-10-28

The production macOS UI created exact tapered, scoped-cut, plane-limited and
face-limited features, then split a panel into two independent Parts. Native
saved-file regeneration confirms their geometry and retained identities. This
records implementation verification, not human acceptance of the workspace.

## Executable and fixture

The reviewed application was `artifacts/development/build-5CnaLJ/MultiCAD.app`,
built after the integrated modeling UI commit `632e3a7`. Its executable SHA-256:

```
2f90ead5eca01de67c9b5822ddce964311d7c9ed81d197fdc6cc33b6b25f05d6
```

At that source checkpoint, workspace formatting, all-target Clippy with warnings
denied and locked offline tests passed: **678 passed, two existing ignored**.
Logs are `/private/tmp/multicad-modeling-integrated-{fmt,clippy,tests}.log`.
The reviewed bundle predates captured-operation group editing.

[`model_scope_review`](../../crates/multicad-app/examples/model_scope_review.rs)
creates the editable fixture and verifies its six native solids before saving.
It also checks exact default and overridden cut queries without committing the
test extrusion. It refuses to overwrite an existing file; the refusal was tested
and the existing file's checksum stayed unchanged.

The generated `artifacts/model-review/cut-scope-native.multicad` contains six
80 x 60 x 10 mm panels at Z offsets 0, 20, 40, 60, 80 and 100 mm:

| Part | Ownership and visibility |
|---|---|
| A visible panel | Visible in active Cabinet A |
| A grouped panel | Visible in an organizational group inside Cabinet A |
| A hidden panel | Hidden in Cabinet A |
| Nested panel | Visible in a nested component |
| B foreign panel | Visible in sibling Cabinet B |
| Document root panel | Visible at document root |

A shared XY sketch contains the rectangle (10, 10) to (30, 30) mm. A separate
construction plane lies at Z = 110 mm. Each initial panel has volume 48,000 mm³.

## Taper and affected-body scope

1. Select the sketch's 400 mm² profile in Model and start Extrude. New body with
   distance 10 mm and taper +5 degrees previews and applies at revision 1.
   Save As writes `taper-native.multicad`; Undo restores the base at revision 2.
2. Reuse the profile, select Cut, start offset -5 mm and distance 120 mm. Exact
   discovery lists only A visible panel and A grouped panel. The final preview
   shows both cuts. Apply changes both targets at revision 3; one Undo removes
   both at revision 4; Redo restores them at revision 5. Save As writes
   `cut-default-native.multicad`.
3. Undo to the base at revision 6 and start another Cut. Exclude A grouped panel
   and explicitly include Cabinet B. Apply remains disabled until a fresh Preview.
   The exact list changes to A visible panel and B foreign panel. Apply reaches
   revision 7; Save As writes `cut-override-native.multicad`.

Independent saved-source reports give the following rounded volumes, in mm³:

| Part | Default Cut | Override Cut |
|---|---:|---:|
| A visible panel | 44,000 | 44,000 |
| A grouped panel | 44,000 | 48,000 |
| B foreign panel | 48,000 | 44,000 |
| A hidden, Nested and Document root panels, each | 48,000 | 48,000 |

The cut removes 400 x 10 = 4,000 mm³ from each chosen panel. The generated tool
Part stays in canonical source and is consumed. Ownership does not change.
The separate tapered Part regenerates with volume 4,360.160342431098 mm³; all six
stock panels remain unchanged in that file.

## Viewer-picked associative limits

After Undo restores the base at revision 8, select To plane and Pick plane in
viewer. Clicking the upper plane outline selects **Limit at Z 110 mm** by its
canonical identity. Start offset -5 mm, plane offset +2 mm and zero taper preview
and apply at revision 9. Save As writes `plane-end-native.multicad`. Reopening
the applied feature's controls preserves the plane and offsets; unchanged Preview
succeeds and Cancel leaves revision 9 intact.

Entering nonzero taper with this associative end displays the unsupported
combination and disables Preview and Apply. Restoring zero enables Preview.

Undo restores the base at revision 10. Hide the root panel at revision 11, use
the +Z view, then select To face and Pick face in viewer. Clicking Cabinet B's
exposed top face records **Face on B foreign panel feature**. A one-sided Preview
with start -5 mm and face offset -2 mm succeeds. Switch to Two sides, retain the
forward face end, and set the backward distance to 7 mm. Preview and Apply reach
revision 12; Save As writes `mixed-ends-native.multicad`.

Edit B's upstream primitive height from 10 to 20 mm. Full native Preview rebuilds
the dependent extrusion; Apply reaches revision 13. Save As writes
`face-rebuilt-native.multicad`. Reports preserve the dependent Part and Feature
IDs while its upper cap moves with B's face:

| Saved result | Z bounds, mm | Rounded volume, mm³ |
|---|---|---:|
| Plane end | -5 to 112 | 46,800 |
| Mixed face/distance ends | -12 to 88 | 40,000 |
| Mixed ends after B's height edit | -12 to 98 | 44,000 |

The common profile remains 20 x 20 mm. The backward end stays fixed; the forward
end follows the current named face and its outward-normal offset. No face frame
was baked into source as a replacement for the reference.

## Independent Split outputs

Create Box 1 at (40, 0, 0) mm with size (40, 60, 10) mm and Apply at revision 14.
Select A visible panel, start Split, and first choose the disjoint B panel as
tool. Preview reports no positive-volume InsideTool partition and Apply stays
disabled, with revision 14 unchanged.

Change the tool to Box 1 in that same form. Preview validates both sides; Apply
creates Split inside 1 and Split outside 1 at revision 15. One Undo restores the
original target and tool at revision 16. Redo restores the two outputs at
revision 17. Save As writes `split-native.multicad`.

The saved report contains one solid per output, each with rounded volume
24,000 mm³. Inside bounds are (40, 0, 0) to (80, 60, 10) mm; outside bounds are
(0, 0, 0) to (40, 60, 10) mm. Their sum matches the original target volume.
The InsideTool producer is `46fb5e0d-0ff1-4bf0-8e76-429d30abd410`, matching the
producer reported during the earlier failed Preview. Target and tool remain
canonical source and are consumed by their producer versions.

## Evidence boundaries

Every named file and its corresponding `*-report.json` is an ignored local
artifact under `artifacts/model-review/`. Reports run the production native
compiler in a fresh process through
[`model_file_review`](../../crates/multicad-app/examples/model_file_review.rs).
Rounded table values are supported by those raw reports and the analytical
volumes above; display triangles are never manufacturing input.

Both open tabs were saved before Quit. Application inventory confirmed every
MultiCAD review instance was closed. Interactive reopen of these new operation
files remains a subsequent check; fresh saved-source regeneration passed.
Captured operation-group editing, general multi-fragment Split, Windows/Linux
physical runtime and low-power performance are outside this checkpoint.
