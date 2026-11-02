# Shared Project materials and stock

CAD-224 places shared definition authoring in **Project → Materials**, as
explicitly selected by the product owner on 2026-11-02. It supersedes the
Model-only authoring location. Project keeps its five left-side actions,
parameter table, ribbon-free layout and one file-metadata sidebar. Parameters
and Materials are sections of that workspace. Model retains contextual
definition selectors and Part assignments.

The editor uses the existing canonical manufacturing materials and
`materials.library` stock, appearance, hatch and material-default entities.
There is one catalog in the document. New panel, sheet and profile buttons
initialize the appropriate stock specification; edits retain the existing
entity ID. An empty project can draft a companion manufacturing material with
its first stock entry and commit both in one transaction.

Rectangular and round profiles expose hollow or solid sections. Round stock
uses one outside diameter; angle stock exposes leg thickness. These controls
author the existing canonical profile fields and use the same native validation.

`prepare_library` is the shared typed source transaction used by UI and MCP.
The Project UI evaluates that prepared document in a separate native worker
before enabling Apply. Its candidate retains the originating session preview,
draft, project resources, request stamp, canonical content hash and control
channel. A matching full native scene with source-bound evaluation metadata is
required. Changed drafts, source, resources, foreign replies and missing
metadata cannot authorize Apply. New broken material/stock associations and
native dependent-feature failures remain errors without committing source.

Preview switches to a read-only native model view. The committed document is
unchanged until Apply; Discard drops the candidate and draft. Pending definition
edits block workspace/parameter changes, history and saving, and temporarily
disable file-metadata editing. Apply commits one undoable source transaction.
Clean selected definitions refresh from the current source after Undo/Redo.
Cancelling unrelated tools preserves the idle material worker.
The status bar reports completion or the native failure when evaluation ends.

Panel, sheet-metal and weld features already reference stock by identity and
read its live thickness or section. A stock assignment to an arbitrary Part is
manufacturing metadata and does not resize that Part. Existing changed-content
warnings remain visible for material defaults and Part assignments; live linked
appearance and drawing hatch definitions retain their IDs and behavior.

Legacy Model → Materials commands and recent-view records route to the Project
section. The old one-tool Model workbench is removed. Existing parameter-workspace
`Common` records remain compatible; the visible section is named Parameters.
