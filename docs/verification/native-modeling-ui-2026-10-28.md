# Native modeling UI checkpoint, 2026-10-28

The macOS application restored a saved workspace, created a sketch directly on a
selected face, and edited exact solids through Scale and Fillet/Chamfer. Saved
projects regenerated successfully through the production compiler. These are
agent-run checks; product acceptance and physical trackpad testing remain open.

## Reviewed source and artifacts

The interaction checks below used frozen source `b4c1283` in
`artifacts/development/build-s0UeZd/MultiCAD.app`. The executable SHA-256 was:

```
31cee60c51c7f7c5fa454a74e5059dbe2c743f3abcfc6820a358053055408b67
```

This executable predates the user's spelling correction to **ToskCAD**, committed
as `86cbe81`. Internal executable names, bundle identity and `.multicad` files
retain compatibility. No installation, push or publication occurred.

The corrected native UI was checked at `1a6eeb4` in
`artifacts/development/build-6Miicu/MultiCAD.app`, executable SHA-256:

```
cf8706ad5d457c83ae714a895e8553ccd43fcbe681677d44c810750fafd71f67
```

Welcome, the application menu, window title and native Save As all displayed
ToskCAD. A fresh-opened Chamfer project restored Model/Common at revision zero.

The initial frozen snapshot `b4c1283` passed pinned formatting, strict all-target
Clippy and locked offline workspace tests: 1,037 passed, zero failed, two existing
ignored. Logs are
`/private/tmp/multicad-refine-{fmt,clippy,tests}.log`. The branding correction also
passed those gates and `npm test`, with 25 release-contract tests passing.

The final integrated snapshot `899a206` passed formatting, strict all-target
Clippy, locked offline workspace tests (1,043 passed, zero failed, two existing
ignored) and all 25 release-contract tests. Logs are
`/private/tmp/multicad-toskcad-verified-{fmt,clippy,tests,node-tests}.log`.
The final native bundle is `artifacts/development/build-QM2alR/MultiCAD.app`;
its executable SHA-256 is:

```
2d831c2a84e48ec546a4de36e348af164d92eb62e6102eea6727d27065c00423
```

The final commands were:

```
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline
npm test
CARGO_INCREMENTAL=0 ./build
```

The preceding integrated test run exposed an intermittent recovery-file lock
failure. Deterministic native-save regressions proved that a retained duplicate
descriptor kept the lock after the writer returned. An explicit unlock guard
fixed success and error exits. The original failure's precise fork timing remains
an inference. The failed run remains in
`/private/tmp/multicad-toskcad-final-tests.log`; the final successful run is above.

## Workspace, ribbon and face entry

Clean quit, relaunch and recent-file open restored Model/Wood at revision zero
with Undo disabled. Navigation changed recent-project metadata independently of
canonical source. The original input was
`artifacts/model-review/move-copy-gizmo-native.multicad`, with SHA-256:

```
3b59e1fb992ecbfe736865313190e8bfef591ad39104b70e41bd217341cd62a1
```

Returning from Wood to Common displayed the grouped ribbon, including the
Box/Cylinder dropdown and the Scale and Fillet/Chamfer commands. Empty Wood
displayed centered guidance. Welcome displayed the existing approved application
logo. A Cylinder native preview enabled Apply; Cancel retained revision one.

Selecting the Source block's top face, choosing Sketch, then New sketch entered
Sketch 1 immediately. Its frame origin was (46.99215228426331, 0, 12) mm, with
normal +Z and sketch editing tools visible. No second plane prompt appeared.
Native Save As wrote a new `face-entry-native.multicad`.

## Scale and edge treatment

Tool-first Scale accepted the Source block from the tree. Dragging its X handle
changed X to 1.4986013939741702 while Y and Z remained one. Release committed one
native operation at revision two. Undo and Redo restored the preceding and scaled
solids at revisions three and four. Native Save As wrote `scale-native.multicad`.

Fillet/Chamfer isolated the upstream Part for explicit edge selection. A selected
edge and 2 mm radius produced a ready native Fillet preview; Apply reached
revision five. Edit latest feature switched the same feature to a symmetric
2 mm Chamfer at revision six, retaining its captured edge and identity.

Editing that distance to 1,000 mm failed native validation. The Inspector retained
the editable value and disabled Apply. Cancel preserved revision six and the
last valid Chamfer. Undo and Redo restored Fillet and Chamfer; separate Save As
operations wrote `fillet-native.multicad` and `chamfer-native.multicad`.

In the corrected build, tool-first Scale accepted the Source Part, then camera
Fit and another independent repaint retained the fixed Inspector footer.
Clicking the white Cancel button left revision zero unchanged. Its actions use
the non-scrolling Inspector's UI context; a regression verifies that culling the
body cannot hide or disable the footer.

Shift-selecting both existing Instances and enabling Create copy allowed a
uniform center-handle drag. Release created two scaled copies in one operation
at revision one. Undo removed both at revision two; Redo restored them at revision
three. Native Save As wrote `scale-copy-native.multicad`.

## Independent geometry reports

All new projects and reports are under `artifacts/ui-refine-20261028/`.
The [`model_file_review`](../../crates/multicad-app/examples/model_file_review.rs)
example reopened each saved project independently and invoked the production
compiler. Each report used a new destination:

```
CARGO_INCREMENTAL=0 cargo run -p multicad-app --example model_file_review --locked --offline -- INPUT.multicad NEW_REPORT.json
```

| Project prefix | Source Part volume, mm³ | Target Part volume, mm³ | Resolved references |
| --- | ---: | ---: | --- |
| `face-entry-native` | 28,800.000000 | 96,000.000000 | All |
| `scale-native` | 43,159.720146 | 96,000.000000 | All |
| `fillet-native` | 43,082.535520 | 96,000.000000 | All |
| `chamfer-native` | 42,979.887979 | 96,000.000000 | All |

Each Part contained one solid. Both linked Instances followed the Source Part's
volume in each report. Fillet and Chamfer retained feature
`7c80fca3-a799-4582-84c0-fd76e25aa5dd` and edge capture
`9f2efd6f-cc49-4524-814e-853e5aeb722f`. Independent regeneration also confirmed the
saved sketch frame. The original input's SHA-256 remained unchanged.

The copy report contains exactly two independent Part outputs and four physical
Instances. Both original Instances retain the Chamfer volume from the table.
The two copies each contain one solid with volume 350,981.0680607122 mm³ and
uniform factors 2.0137527074704766. All references resolve, while the Source Part
and target remain unchanged. Private occurrence geometry contributes through
its Instance rather than duplicating independent physical output.

The final native bundle reopened `scale-copy-native.multicad` after clean quit,
restoring Model/Wood at revision zero with Undo disabled. Opening a copied
Instance's latest feature selected Scale and retained its factors and pivot.
Native Preview succeeded; Cancel preserved revision zero. The application was
left in Model/Common with its saved solids visible. Native application inventory
showed only this final ToskCAD instance running.

## Limits

The current edge operation accepts Part definitions, constant fillet radius and
symmetric chamfer distance. Instance-only treatment, unequal distances, angle
chamfers and variable radii remain unsupported. Other advanced shape features
remain separate work. Nonuniform Scale uses exact native affine geometry;
unsupported NURBS contour export reports a diagnostic.

Physical Option-held copy gestures, pinch and three-finger orbit remain
unverified. Automated native geometry and pointer tests cover Scale copying,
uniform/per-axis handles, stale results, cancellation and save/reopen.
[Trackpad evidence](trackpad-navigation-2026-10-28.md) records input classification
and the macOS system-gesture limitation. Windows/Linux runtime, low-power
performance and manufacturing acceptance have separate verification requirements.
