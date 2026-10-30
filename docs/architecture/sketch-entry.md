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
- Not checked in the running window.
