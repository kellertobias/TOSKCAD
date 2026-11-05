# Associative Transfer in Model Common

CAD-241 exposes Transfer features in Model → Common, opens a receiving-Part row immediately, and refuses to turn known internal threads into ordinary holes. The implementation is commit `3c09255` on `main`. Human acceptance remains separate from this verification.

## Runtime and checks

The locally staged optimized macOS application is `artifacts/transfer-common-review-20261105/optimized/MultiCAD.app`. Its executable SHA-256 is `bb4abe6e73891556cdcfa14f816d3abfab12a40d21fe621166446034d5d54584`. This is a local verification bundle, without release provenance or installation. All runtime source changes are in the implementation commit; the final test-only changes do not affect this executable.

Formatting, strict locked all-target workspace Clippy, the optimized application build, five native Transfer integration tests, and ten registry tests passed. The broader Transfer filter subsequently passed all 18 controller, form, registry, and native host tests.

The locked workspace command completed 279 passing groups, then stopped at one outdated UI host test. That test still clicked Add receiving Part, adding an unintended empty second row after the new initial row. Test-only commit `78db4f8` instead asserts the initial row exists and selects it. Production source and the verified executable are unchanged. The complete UI library target subsequently passed 967 tests with two intentional ignores; all eight remaining UI integration targets and all workspace documentation targets passed. Together with the original unchanged native/core groups, this establishes **2598 passed, zero failed and four intentional ignores across 295 groups**. `checks/summary.json` records the coverage and individual exits. Native/core groups retain the original full-run evidence, rather than repeating their roughly 40-minute execution for a test-only interaction correction. The original exit 101 and corrected rerun exits 0 are retained separately; this is composite workspace coverage, not a claim that the original command exited successfully.

Checks use the pinned toolchain and committed lockfile. The original workspace run and documentation rerun use test optimization level 3 with debug assertions and overflow checks enabled; the complete UI rerun uses the ordinary test profile.

Subsequent full validation on 2026-11-05, after the metric-thread kernel/cache commits `dd50334` and `06f831f`, ran the entire locked workspace successfully: **2600 passed, zero failed, four intentional ignores across 296 groups, exit 0**. This includes the corrected Transfer host test and all Transfer production code, which remains unchanged. The later complete run is retained at `artifacts/thread-performance-followup-20261105/checks/workspace.log`; the original composite evidence above remains recorded accurately.

## Actual macOS authoring

The fixture was prepared through public typed Rust APIs and initially contained no Transfer features. Its source is a 130 × 20 × 4 mm plate with two 6 mm holes, placed above separate Wood, Printed, and Sheet receivers. The enclosing occurrence has a 30° rotation and translation [300, 500, 100] mm. Reproduction source, preparation receipts, native files, screenshots, accessibility snapshots, and their hashes are retained under `artifacts/transfer-common-review-20261105/`.

The actual application checks established:

| Captures | Observed behavior |
| --- | --- |
| 01–02 | Common contains Transfer features. Starting without a selection shows Source occurrence, source geometry mode, Receiving Part 1, and its options. Scrolling exposes lower options while the action footer stays fixed. |
| 03–07 | Source occurrence and its actual top face were selected in the viewer. Wood's face was selected independently, with diameter 5 mm and blind depth 12 mm. Native preview reported two profiles and enabled Apply. |
| 08–15 | Added Printed with radial clearance 0.2 mm and through cuts, then Sheet with diameter 6.5 mm and through cuts. The full native preview reported two profiles for each of the three receiving Parts. |
| 16–19 | Apply created all receiving features in one transaction. Toolbar Undo and Redo restored the corresponding document states. Save As wrote `native-transfer-created.tcd`. Saving also updated preview metadata. |
| 20–24 | Project changed source count 2 → 3, pitch 32 → 40 mm, and source occurrence offset 10 → 15 mm. Apply and Rebuild updated the same associative features. Save As wrote `native-transfer-associated.tcd`. |
| 25–27 | Reopened the existing Transfer form, changed its draft name, then clicked Cancel. The app reported “Modeling cancelled · document unchanged” at the same revision 6. |
| 28 | After quitting and cold-launching the app, opened the saved associated file from Recent. All three receiving Transfer features remained present at reopened revision 0. |
| 29–33 | Selected a source with a valid modeled internal M8 × 1.25 thread. Native Preview refused the mixed-hole face explicitly, named the thread source, and kept Apply disabled at revision 0. The complete diagnostic and Cancel remained readable at normal 1360 × 860 and compact 1100 × 700 logical sizes. Cancel restored the unchanged document. |

Receiver capture views reuse the existing “component-local source view” heading. Captures of those views establish receiver-face selection; they are not evidence that the source itself changed. Some scrolled inspector content is omitted by the accessibility bridge, so those actions use the corresponding visible screenshots as evidence.

The independent visual review inspected 17 specified captures and passed. The Printed receiver's top face is occluded in the assembly overview, so its updated hole count is established by native readback rather than that overview. `independent-native-transfer-visual-review.json` records the inspected files, hashes, and evidence limits.

All ToskCAD verification windows were closed. `closed-window-inventory.json` records the final empty running-app inventory.

## Independent native readback

`independent-native-transfer-readback.json` recompiles the actual GUI-saved `native-transfer-created.tcd` and checks its canonical requests. Each receiver has two holes at local [30, 10] and [62, 10] mm. Native volumes are 69728.76110196151 mm³ for Wood, 30685.281459635837 mm³ for Printed, and 7667.267710385827 mm³ for Sheet. The source remains 10173.80532894153 mm³. Only the three receiver definitions gained their cut features; the source and placements are unchanged. New records comprise the Transfer group, three receiving cuts, and file metadata.

`independent-native-associated-transfer-readback.json` checks the second actual GUI-saved file. Each receiver now has three holes at local [35, 10], [75, 10], and [115, 10] mm. Native volumes are 69493.14165294226, 30427.92218945376, and 7600.901565578743 mm³ respectively; the source is 10060.7079934123 mm³. Transfer group, feature and capture IDs, receiver settings, and occurrence identities are retained. Changes are limited to the three parameters, preview metadata, and parameter-table ordering metadata.

These receipts distinguish native exact-solid measurements from screenshots and retain the standalone Rust audit source and commands. The full fixture's translated and rotated occurrences exercise canonical relative-pose projection; no world-space display mesh is used to manufacture the cuts.

## Thread handling and limits

Transfer currently carries ordinary circular cuts and outer contours, with per-receiver diameter, clearance, depth, and direction. It does not carry thread metadata. The shared model compiler now inspects known internal-thread ancestry, including Hex Nut definitions, and rejects selected coaxial holes before producing receiving cuts. Circular mouths outside the authored threaded axial span are also refused, since a counterbore or lead-in must not erase thread intent. The native regression proves this for both modeled and cosmetic-envelope internal M8 threads and permits the separate noncoaxial ordinary hole on the same Part.

Coordinate-preserving operations retain that axis proof. An unproven downstream transform or deformation causes an explicit refusal rather than using an obsolete axis. This is conservative: separate coaxial holes and some complex ancestry can require an explicit thread workflow even when a particular selected region might be ordinary. Face-boundary transfer does not claim to transfer internal threads. Imported geometry without semantic thread metadata cannot recover a thread specification from its appearance.

Existing source-capture repair, full-preview receipts, stale-source rejection, atomic commit, and save/reopen contracts remain covered by the focused tests and native integration suite. This round establishes macOS interaction and native geometry behavior, not Windows/Linux runtime behavior, current standards compliance, physical fit, or a manufacturing-export validation.
