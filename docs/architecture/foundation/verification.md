# Local foundation verification — 2026-10-26

Source implementation: `eda3d56`; GitHub proof wiring: `6e50c9c`. These are bounded CAD-49 results, not complete product acceptance. Full commands are in the [lab guide](../../../spikes/native-foundation/README.md).

Environment: Apple M5 Max, 64 GiB RAM, macOS 26.6.2 / aarch64, Rust 1.99.1, CMake 4.3.0 and Apple Clang 21. This is a powerful development machine; no low-power performance claim follows from these measurements. Rust lab code uses the dev profile; OCCT is source-built with its Release profile and optional external graphics dependencies disabled.

## Results

| Check | Observed |
| --- | --- |
| Formatting and Clippy | Root and proof workspaces passed; Rust warnings denied. OCCT headers emit an upstream deprecated `sprintf` warning under Apple Clang; it is not hidden. |
| Proof tests | 7 passed: solver/conflict diagnostics; geometry failure; strict bounded file loading/atomic save; revision and Undo/Redo; triangle picking; Parameters/sketch layout; asynchronous preview → commit → Undo. |
| Root regression | 5 CLI tests and 18 release contract tests passed. The HTML mockup is unchanged from its baseline. |
| GitHub workflow validation | actionlint passed both workflows. No hosted jobs have run; no push/remotes exist. |
| Geometry | A solved shared rectangle drives two independent extrusions/cuts. Exact volumes 172800 and 86400 mm³; 24 display triangles with signed mesh volume matching the exact solids. |
| STEP lifecycle | Actual `sketch-cut.step` exported, reimported and validated; volume matches. Process exits successfully, including after a deliberate invalid output path. |
| Alignment fixture | Exterior/contact/interior offsets of 0, ±1e-8, ±1e-6 and ±1e-4 mm remained valid. Analytic volumes checked with a fixture allowance of twice 1e-7 mm times the contact area. This does not establish a universal kernel tolerance policy. |
| Rebuild timing | 25 small-fixture rebuilds: approximately 4.3 ms median and 5.1 ms maximum. This is solve + two solids + cuts + validation + tessellation, excluding UI presentation. |
| Verification process resources | About 0.17 s wall, 0.13 s user CPU, 0.01 s system CPU; maximum RSS 24,248,320 bytes. This measures the headless verification process, not the running GPU window or large documents. |
| CPU UI benchmark | 360 layout/tessellation frames over scales 1.0, 1.5 and 2.0: medians 0.594–0.599 ms, p95 0.834–1.083 ms. Cold maximum 45.94 ms. No GPU/input-to-presentation latency claim. |

Native macOS interaction verified through the running window:

- Parameters occupies the content area without the parts/Inspector/ribbon.
- Numeric and text keyboard edits work with ordinary pointer focus; Apply rebuilds and Undo restores the committed definition.
- New Sketch retains the viewer, exposes projected XY-plane selection, then reveals sketch controls. Finish returns to Model.
- Part picking updates selection and Inspector volume. Docked panels render through the docking framework; the automated separator drag did not visibly resize them, so resizing, tear-off and multi-monitor behavior remain unverified.
- Saving produced the actual workspace-local proof JSON. Loading it exercised unsaved-change discard confirmation, candidate rebuild, restored width 135 mm and Saved state.
- An invalid 999 mm cut was rejected without replacing the valid model. Preview changed displayed geometry while document revision remained zero; Apply/Undo isolation is also covered by the worker test.
- After focus left text editing, the viewport frame counter remained unchanged across separate observations. Focused caret blinking can legitimately repaint.
- AccessKit exposes controls. Accessibility activation did not reliably focus text fields; coordinate focus plus normal keyboard input worked. Full keyboard-only navigation and screen-reader behavior remain unverified.

The first candidate build using bundled OCCT 7.8.1 crashed during static type/map destruction after STEP I/O. That result was rejected. Official OCCT 7.9.3, with archive hash/source revision committed separately from Cargo, passed the same lifecycle and clean-exit check. The initial slvs 0.6.0 wrapper also failed locally because its generated bindings lacked required types; no global dependency source was patched to make it appear to pass.

## Remaining work

Windows/Linux execution, real display/DPI transitions, large scenes, physical low-power machines, general sketch/profile topology, stable geometric references and complete accessibility are unverified. The source-built C++ dependency currently rebuilds into separate Cargo check/build output directories; CAD-50 should establish shared, target-aware native caching and full runtime/source/notice packaging.

The laboratory's mode registry is only a descriptor experiment. The production command/module extension API, canonical feature document, migrations/recovery, native file dialogs/close protection and MCP are not implemented. CAD-50 and subsequent core/UI/file plans remain necessary. Generated reports, native test bundle and STEP data stay in ignored `artifacts/`; the lab is excluded from bootstrap release assets.
