# Sketch entry and exit

| Command | Where | Effect |
| --- | --- | --- |
| **New sketch** | Model → Common → Create, Sketch → Common | With a planar face selected in the 3D view, creates a sketch attached to that face. Otherwise the user picks a datum, construction plane or planar face; Esc or **Abort** cancels without changing the document. |
| **Create engraving** | Model → Common → Create | Creates a sketch named "Engraving N" attached to the selected planar face. There is no plane-picking fallback: no selection, a stale selection or a curved face is refused with a message and nothing changes. |
| **Finish sketch** | Pinned right of the Sketch ribbon | Leaves sketch editing; refuses while sketch controls are unapplied. |
| **Abort** | Pinned right of the Sketch ribbon | Leaves sketch editing or plane picking and discards only what is not yet in the document: the active tool, previews, unapplied constraint values and pending repair or trim work. Sketch edits already applied stay, and Undo removes them. |

While a sketch is open, **New sketch** is hidden and **Finish sketch** and
**Abort** sit together in a "Sketch" group at the right end of every Sketch
tab, outside the horizontal scroll, so they stay visible in narrow windows.

All face-attached sketches use `multicad_core::sketch::prepare_new` with
`CreationPlane::Face`: the sketch plane stores a persistent face reference
to its source feature, so it follows that face through rebuilds and survives
save and reopen. Creating one is a single undoable transaction.

## Drawing tool guidance

While a drawing tool is active, the Inspector shows guidance derived from
the tool's real gesture state (`crates/multicad-ui/src/sketch_tool_guide.rs`):

- **Step**: the next viewport click, for example "Step 1 of 2 · Click the
  first corner" and then "Step 2 of 2 · Click the opposite corner" for
  Rectangle. The status line repeats it with a short Escape hint.
- **Pending input**: clicks that are not in the sketch yet ("Pending: first
  corner · not in the sketch yet"), or for Polyline the segments of the open
  chain that are already committed.
- **Commit**: how geometry enters the sketch. Line, Rectangle, Circle, Arc
  and the other rectangle and circle modes commit on their final click, with
  no Apply; the tool stays active and the status line reports "Rectangle
  committed · Undo removes it". Polyline commits one segment per click after
  the first. Fillet, Chamfer and Text apply on each click.
- **Escape**: Esc discards only uncommitted clicks and ends the tool.
  Geometry already committed stays; Undo removes it.

Only Spline has a completion action: the fixed action bar shows Apply
(accessibility label "Finish spline"), disabled until four control poles
exist, and Cancel. Enter finishes the same way, and clicking the first pole
commits a closed spline. Other drawing tools reserve no action bar; their
Inspector offers **Clear pending points** (keeps the tool active) and
**End <tool> (Esc)**. The command-line syntax remains available under the
collapsed **Command entry** section.

## MCP

`create_engraving_sketch` takes a source feature, a face ordinal from
`reference_candidates` at the same revision, and an optional name. It
compiles the source and calls the same `prepare_new`. Curved, missing and
ambiguous faces are refused as `invalid_request`, an old revision as
`stale_revision`; neither changes the document.

## Limits

- Engraving defines only the sketch. Depth, material removal, CNC
  toolpaths and engraving on curved surfaces are not specified.
- The Create engraving and Abort buttons use placeholder icons until the
  reviewed artwork is integrated (CAD-137).
- Not checked in the running window. Drawing-tool guidance is covered by
  headless Inspector and canvas tests only.
