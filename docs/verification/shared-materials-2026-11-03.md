# Shared Project materials and stock verification, 2026-11-03

CAD-224 moves shared material, sheet and profile stock authoring to Project.
The production UI, native representative checks, required workspace
verification and application build pass. Actual macOS OS-window
interaction is unverified, and no human acceptance is recorded.

## Source and authoring behavior

Work is local on `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Scope |
| --- | --- |
| `159e59f` | Embedded Project editor, canonical definitions, native candidate gate, navigation and draft guards |
| `6f0a079` | Hollow/solid, round-diameter and angle-thickness controls; accessible fields and completed preview status |
| `0b344c4` | Native stock/hatch fixtures, production captures and cross-workbench tests |
| `357e1b1` | Copy the actor channel directly; final verification source freeze |

The user explicitly selected Project for shared authoring. That decision is
recorded in CAD-224 and `AGENTS.md`. Project retains the five left-side actions
at their established top position, Parameters access, one metadata sidebar and
no ribbon. Model tools select the same canonical definitions by identity.
Legacy Model → Materials commands and recent views route to Project.

Apply requires a matching native evaluation of the prepared definition edit.
Changed source, draft, resources, session, channel or missing native metadata
cannot authorize it. Preview leaves the document unchanged. Discard drops the
draft; Apply commits one undoable transaction. Pending definition edits block
history, saving and conflicting mode/parameter/metadata edits. Clean selected
definitions reload after Undo. Cancelling unrelated tools retains the idle
native worker. Completion and native errors update the status bar.

New panel, sheet and profile stock controls use existing canonical types. The
first stock can create a companion manufacturing material in the same edit.
Rectangular and round profiles support hollow and solid sections; round stock
uses one diameter, while angle stock exposes leg thickness.

## Production UI and native tests

The final UI crate run passed **905 tests, zero failures and two ignored** in
24.09s. Three Project tests use the actual painted controls to create a material,
sheet stock and profile stock through Preview/Apply, and check history, Discard,
draft guards, legacy recall and source neutrality. Eleven library tests include
native dependent geometry, shared hatch changes, invalid/missing/stale inputs,
foreign session/resources/channel, missing evaluation metadata and stock-material
association failures.

The actual Hollow section widget converts a tube into a native solid bar with
120000 mm³ volume, retaining the stock and Part IDs. The same editor changes
wood, sheet and profile stock and a shared hatch, then verifies Undo/Redo without
creating additional definitions. An invalid carcass thickness and a used-stock
material mismatch remain errors without source changes.

Early UI runs caught action displacement, parameter-workspace naming
compatibility and an unnecessary worker reset on cancellation. These were
corrected before the final passing UI run. The first strict Clippy attempt
reported `clone_on_copy` for the actor channel; `357e1b1` corrects it.

## Representative native artifacts

`crates/multicad-app/examples/shared_materials_review.rs` generates
`artifacts/shared-materials-review-20261102/`. It uses one shared catalog for a
Wood panel, Sheet-metal plate and Weld member. The two metal features retain
the same steel material identity. Stock and Part identities remain unchanged.

| Feature | Before | After | Native volume before / after |
| --- | --- | --- | --- |
| Wood panel | 100×60×18 mm | 100×60×24 mm | 108000 / 144000 mm³ |
| Sheet-metal plate | 100×60×2 mm | 100×60×3 mm | 12000 / 18000 mm³ |
| Weld tube, 200 mm long | 30×20×2 mm section | 40×30×3 mm section | 36800 / 76800 mm³ |

Three native section views use the same linked hatch. Its paper line width
changes from 0.18 to 0.28 mm, angle from 0° to 45° and spacing from 2 to 3 mm.
Hatch counts change from 31/31/22 to 37/37/30; every hatch vector has the expected
paper width. Both `before.tcd` and `after.tcd` reopen with the same geometry,
associations and drawing results. All six STEP exports read back their native
volumes within 0.0001 mm³. `native-report.json` records identities, bounds,
volumes and section results.

The five changed-content warnings visible in the edited fixture are retained
fingerprint review/acknowledgement notices. Live linked definitions resolve;
these are not missing-link or unresolved-geometry results. A metadata-only stock
assignment to an arbitrary Part continues to leave its geometry unchanged.

## Production offscreen review

All nine final PNGs in `captures-final/` are visually verified: sheet inputs,
profile inputs and the native preview at 1100×700 logical points (1×),
1360×860 (1×) and 1360×860 (2×, 2720×1720 physical pixels). The helper uses the
production Application, egui and native GPU renderer. It checks the absent
ribbon, retained metadata/sidebar actions, uncut stock inputs and native scene
containing the three Parts.

The preview capture creates a new unused material, clicks the actual Preview
control and waits for native Apply readiness, while asserting unchanged source.
Stock-edit geometry and hatch effects are proven separately by the native
editor tests and artifacts above. The initial capture timeout was a helper
timing defect: it queued Preview from the frame painted disabled before draft
creation. Waiting for the painted enabled control corrected the capture flow;
the application validation gate was retained.

Final profile and preview captures were inspected after the last layout/status
changes. The three sheet PNGs were byte-identical to their inspected versions.
Earlier `captures/` are superseded. No OS window is created by this helper.

## Required checks and build

Formatting and strict `cargo clippy --workspace --all-targets --locked --
-D warnings` passed on `357e1b1` (Clippy 2m33s).
`cargo test --workspace --locked` passed: 2327 tests, zero failures and four
ignored across 261 result groups. The UI group passed 905 tests, zero failures
and two ignored in 25.26s. Test compilation took 3m05s. `./build` passed in
24.45s and produced `artifacts/development/build-CVkmlz/MultiCAD.app`.
Its executable returned zero for `--version` and `--diagnostics`: development
version 0.1.0-dev, macOS/aarch64, static OCCT 7.9.3, eframe 0.36.2/wgpu and a
compiled native runtime. The development source commit is not stamped; this
is not release provenance. The diagnostics workspace summary is legacy scaffold
text and is used only for binary/platform/runtime identification here.

## Verification limits

These checks use compiled native geometry and production components with
offscreen pointer input. They do not prove actual macOS OS-window interaction.
The native CUA controller is unavailable; no substitute private input mechanism
was used. Windows/Linux runtime/CI and human acceptance remain separate.
The item is submitted for human Review/test, not recorded as accepted or Done.

No installation, push or publication is performed. No OS windows were opened;
completed QA processes exited. Unrelated `.claude/` is preserved.
