# Native reference repair verification, 2026-10-27

The `reference_review` example exercises real OCCT geometry through the native
application and shared reference API. Its narrowly supported feature definitions
are acceptance fixtures. They do not replace the production feature compiler or
implement the general solid-modeling tools.

## Native geometry and scene checks

The fixture contains a named panel, a circle on its top-face plane, a Boolean
through drilling, a part and an instance. Parameters control width, thickness and
topology: intact, a full-width slot splitting the top, or a shave removing it.
The scene retains the upstream panel as a hidden geometry asset so reference
diagnostics inspect the actual producer separately from the drilled result.

Seven example tests cover dimension changes and drilled volume, split/deleted
references, explicit fragment repair, undo/redo, source save/reopen and the actual
asynchronous `SceneFeed` publication gate. They also check a translated instance
rotated 90 degrees: the local attachment remains at 20,25,18 mm, its placed frame
resolves to -15,40,48 mm, a world ray passes through the real hole and a neighboring
ray hits the top at Z48. Isolating that instance preserves its placement.
Additional tests duplicate instances transactionally, preserve three canonical
identities sharing one drilled geometry asset, and reject unsupported components
instead of silently omitting them.

The UI repair controller has tests for preview/cancel/accept, one-command undo,
stale evaluation and source rejection, replacement sources, unnameable candidates
and a pointer-driven Preview → Accept sequence. Current resolved reports are
required for attached-sketch opening and constraint edits. Tests reject missing,
stale or unresolved attachment evaluation rather than using the saved plane datum.

## Native Mac interaction checkpoint

Checked on the available Mac with Metal in a 1360 × 860 logical window:

1. Changed panel thickness from 18 to 24 mm in Parameters and applied it as one
   source transaction. The model rebuilt with the real drilling; selecting the
   attached circle showed its plane origin at 0,0,24 mm.
2. Changed topology mode to split. Inspector reported two possible faces and the
   previous valid drilled panel stayed visible. The sketch selection remained
   available for inspection; its unresolved frame was explicitly reported.
3. Selected the original panel source and a current top fragment. The explicit
   repair view showed the split geometry and highlighted the chosen left top
   face. Preparing the replacement listed five affected objects and left the
   document at revision 2. Cancel returned without changing source.
4. Prepared the same replacement again and accepted it. Revision 3 contained one
   repair transaction; the split model and dependent drilling became visible,
   with the circle still on Z24. One Undo restored the ambiguous reference at
   revision 4; Redo restored the valid replacement at revision 5.
5. Saved the repaired source atomically. The local checkpoint
   `artifacts/reference-review/repaired-radius3-checkpoint.multicad.json` retains thickness
   24, topology mode 1, the reference UUID, an eight-origin boundary discriminator
   and the authored attachment offset. The source was preserved before closing
   that review instance.

## Combined-build reopen and editing

The combined native build at `95c214a` reopened the repaired checkpoint in a fresh
process. It retained the split, resolved the repaired plane and showed the circle
on Z24. Its initial Inspector was empty. The file-open check deliberately tried
a missing test path first; the successful retry immediately returned the status
to gray, before changing modes.

Opening the attached sketch exposed its editing tools. With the circle selected,
the real command line accepted `dim 5 mm`: radius changed from 3 to 5 mm, its plane
stayed at Z24 and the solver reported two remaining degrees of freedom. Expanded
history showed the manually entered command and the `CommandLine` transaction.
The native model displayed the enlarged Boolean drilling after finishing the
sketch. The saved final source's complete geometry intent equals that in the
radius-3 checkpoint: reference UUID, provenance, boundary and offset all match.

The same native window verified selected-part properties and linked duplication.
One duplication added an instance in one revision; native Shift-click selected
both rows. Duplicating both added two rows in one revision, and one Undo removed
both. Search accepted Down/Enter to open Parameters without changing source;
Escape released search focus so Tab switched Model/Common to Wood. The local-user
popover exposed the local actor and shortcut/history controls. A Cmd-click attempt
produced single selection; modifier delivery was not resolved. Native Shift
selection and headless Cmd/Ctrl cases are verified separately.

The final guarded build at `510642d` was launched after closing the prior saved
instance. The unsupported transform control was disabled without a selected
sketch. Pressing X still allowed a subsequent native face pick at Z18. It reopened
`artifacts/reference-review/reference-final.multicad.json`, showing both saved
instance rows, the repaired split and the enlarged drilling with the new default
solid/visible-edge style. Its unselected Inspector stayed empty. The final binary
SHA-256 was `52317464458639f99df81bcc3e23d553d6d6040127444832467105326902ab9b`.

Final automated checks passed: formatting, full workspace Clippy with warnings
denied, 156 workspace tests including two compile-fail checks, seven reference
example tests and two viewport example tests. The physical Metal test ran
separately and passed multiple-part highlighting plus all previous projection,
style, section and cache checks. The upstream native deprecated `sprintf` warning
remains.

The main production executable still needs its general feature evaluator.
General face-sketch creation and projection remain CAD-63 work; manufacturing
exports remain later work. Native project containers, recovery and persisted
session history remain separate from these interim canonical JSON snapshots.

## Reproduction

```sh
cargo test -p multicad-app --example reference_review --locked --offline
cargo run -p multicad-app --example reference_review --locked --offline -- --headless
cargo run -p multicad-app --example reference_review --locked --offline
```

The interactive example creates its initial source under `artifacts/reference-review/`
on each launch. Preserve edited checkpoints under a different name before
relaunching. Headless execution computes geometry without opening a window.
macOS evidence does not establish Windows/Linux runtime behavior or performance
on weaker machines. No push, publication, installation or human acceptance is
implied by these checks.
