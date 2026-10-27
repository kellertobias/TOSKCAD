# Native sketch entry verification — 2026-10-27

CAD-60 implements sketch entry and attachment. General sketch tools and solid
feature authoring remain separate work in CAD-62 and CAD-64. The native review
fixture uses genuine OCCT geometry with a deliberately limited feature compiler.
It does not establish that arbitrary production models can already rebuild.

## Automated checks

- `multicad-core/tests/sketch_planes.rs` exercises one-transaction prepared
  creation, source-neutral cancellation, stable Undo/Redo identities, datum and
  existing-plane offsets, face attachment, changed parent normals, reference
  repair, reload and rejection of stale or unsupported selections.
- `multicad-ui/tests/sketch_context.rs` uses actual named OCCT extrusions to check
  component-local source coordinates against translated/rotated/shared displayed
  occurrences, unconsumed face sketches, offset chains and explicit world anchors.
- `sketch_plane` tests check Inspector and pointer selection, generation/revision
  guards, missing placement context, curved or unnamed surface rejection and
  section-clipped face picking. A clipped view selects retained real faces; it
  does not create manufacturing section geometry.
- `sketch_editor` tests cover selecting without committing, cancellation, stale
  acceptance rejection, explicit creation, immediate editing, local/world inverse
  coordinates, re-entry and one-step Undo for creation and geometry edits.
- The `reference_review` example regression rebuilds a new sketch attached to the
  final drilling feature, including subsequent dimensional and sketch edits.
  Upstream ambiguity still retains the previous valid model.

## macOS native checks

The physical review used the compiled `reference_review` application at
`artifacts/development/MultiCAD-reference-review.app` in two builds:

| Scope | Executable SHA-256 |
|---|---|
| Entry, face selection, cancel, offset/angle and creation (steps 1–3) | `2ea70a95c53e8e571ea6734bc4f014adc028494c6d90b8939782d44f7f88fb34` |
| Fixed fixture rebuild, editing, native save and fresh-process reopen (steps 4–8) | `1d129cb6feb3aeb48c24fb2f8f2bb1d84e8322889d279e73b21f91ff1d6442ca` |

Later section-context guards have automated evidence above, not a separate
physical interaction claim.

1. Selecting Sketch retained the real 3D model. Drawing and constraint tools were
   absent before creating or opening a sketch.
2. New Sketch displayed datum/construction guides and contextual plane controls.
   Clicking a real top face selected a preview at Z=18 mm. Cancellation kept
   revision 0 and created no sketch.
3. A 3 mm normal offset and 30° in-plane angle previewed origin Z=21 mm,
   U=(0.866, 0.500, 0) and V=(-0.500, 0.866, 0). Use plane committed the attachment
   before drawing tools appeared.
4. The saved source checkpoint was imported after fixing the fixture's dependency
   ordering. The actual drilled panel rebuilt without a repair error. Opening
   Sketch 2 restored its plane at Z=21 mm.
5. `rect 10 15 30 25` produced four editable constrained lines with lengths
   20/10/20/10 mm. The displayed rectangle followed the rotated sketch axes.
   Finish sketch returned to Model while preserving the sketch.
6. Save As wrote `artifacts/reference-review/face-sketch-native.multicad` through
   the native project container. A fresh process opened that file with a clean
   saved baseline.
7. Changing panel height from 18 to 24 mm, applying and reopening Sketch 2 moved
   its origin from Z=21 to Z=27 mm. Four curves and their constraints remained.
   Native Save replaced the project successfully.
8. The checked project reader decoded before/after native containers. All saved
   geometry anchors, their intent IDs, sketch geometry IDs and constraints were
   equal; `panel_height` was the only changed source entity.

The initial fixture rebuild rejected an unrelated new attachment before its
producer had been generated. Commit `503e21d` fixes that ordering and adds the
regression above. The successful checks followed that fix.

## Limits and retained artifacts

Only macOS interaction was checked. Windows/Linux runtime, low-power hardware and
sketch-creation latency were not measured here. Curved, unnameable or ambiguous
planar-frame selections produce diagnostics instead of a guessed attachment.
Broader geometry transforms, projected references and region tools remain later
work. Native macOS Quit and recovery behavior are tracked separately under CAD-56;
this sketch result does not certify those paths.

Local, ignored verification artifacts include:

- `artifacts/reference-review/new-face-offset.multicad.json`: source checkpoint
  from the offset/angle creation check.
- `artifacts/reference-review/face-sketch-height18.multicad`: native baseline with
  editable geometry at the original height.
- `artifacts/reference-review/face-sketch-native.multicad`: native project after
  changing the supporting height to 24 mm.
- `artifacts/reference-review/face-sketch-height18-source.json` and
  `face-sketch-height24-source.json`: checked-reader reports used for comparison.

[Producer-local attachment decision](../architecture/0009-sketch-plane-occurrences.md)
and [reference repair verification](reference-repair-2026-10-27.md) describe the
shared attachment and repair contract.
