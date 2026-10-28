# Native spline editing verification, 2026-10-28

Exact rational and polynomial spline edits passed native geometry, transactional
and macOS interaction checks. This records engineering verification; human
workflow acceptance and Windows/Linux runtime checks remain separate.

## Source and executable evidence

Production source through `3a3ec45` passed formatting, strict workspace/all-target
Clippy, and locked offline workspace tests: 1,224 passed, zero failed and two
existing ignored tests. `npm test` passed all 25 release contract tests.

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline
npm test
CARGO_INCREMENTAL=0 ./build
```

Logs are `/private/tmp/multicad-cad127-final-{tests,clippy,npm,build}.log`.
The known upstream OCCT `sprintf` deprecation remains outside the Rust lint scope.

| Native check | Local bundle | Executable SHA-256 |
| --- | --- | --- |
| Pointer trim, Cancel, Apply, history and Save As | `artifacts/development/build-2nXymG/MultiCAD.app` | `2870692522ef3ec8e1ace5c455a9eceb41c64940b8ba7e7cfcc8980cf597f071` |
| Fresh reopen, further exact preview and discarded-report controls | `artifacts/development/build-STXswX/MultiCAD.app` | `6c5e4738a9493fdac3ce28f68758cc51c90736a699fb1b51816654faf43098b8` |

The final bundle contains the production source cited above. Review instances
were closed through native controls. Nothing was installed, pushed or published.

## Geometry and transaction coverage

The [spline edit policy](../architecture/0022-exact-spline-edits.md) owns the
continuation rule and supported limits. Tests use real native curves, exact
segmentation and the production constrained solver.

| Behavior | Evidence |
| --- | --- |
| Rational and polynomial trims against line, arc, circle and spline boundaries | [Native model tests](../../crates/multicad-model/tests/spline_trim.rs), eight combinations with 41 retained-curve samples each |
| Both endpoint continuations preserve the original interval | Native model and [kernel tests](../../crates/multicad-kernel/tests/spline_edits.rs), including 101 original-interval samples |
| Multiple contacts require explicit choice | Native model tests, actual native form and macOS chooser |
| Missing, tangent, overlapping, degenerate and out-of-budget inputs cannot commit | Kernel, core, solver and native model failure fixtures |
| Parameter changes use the current boundary; affected constraints reject explicitly | Native model tests preserve source and session history on failure |
| Exact closed regions produce real solids and STEP geometry | Native model tests close trimmed profiles, extrude, and independently read STEP results |
| Native archive readback preserves editable source | Ten no-overwrite generated projects plus independent reads of three GUI-saved projects |
| Viewer clicks use exact projections under their captured camera | [Host tests](../../crates/multicad-ui/src/sketch_trim_host_tests.rs), including zoom before Preview and curve selection over a control pole |
| Cancel, latest-only queue, stale/foreign session and worker panic handling | [Worker tests](../../crates/multicad-ui/tests/exact_trim_worker.rs) and read-only preparation snapshot guards |
| Strict numeric input cannot acquire viewer snapping authority | Native worker regression, including a rejected point outside the eight-pixel bound |

Core and solver use optional query interfaces without native dependencies.
Native queries run on the background worker. The host adopts a checked draft only
while its source, session, evaluation and form remain current.

## Native interaction and saved-source checks

Inputs and results are under `artifacts/spline-trim-review-20261028-2153/`.
The production `spline_trim_review` example generates ten editable inputs/results
and a manifest; it refuses an existing output directory.

In the native app, selecting the sketch and opening it exposed the Edit ribbon.
Trim was activated before selecting a curve. A viewport click chose the target
span and a second click chose the finite vertical boundary at X = 10 mm.
Preview enabled Apply without changing canonical source. Cancel restored the
original source. Preview and Apply committed the polynomial trim at revision 1.
Header Undo restored the original curve at revision 2; Redo restored the trim at
revision 3. Save As wrote `polynomial-native-gui.multicad`.

The same pointer workflow trimmed the rational quadratic and saved
`rational-native-gui.multicad`. The production `model_file_review` reader rebuilt
both saved archives. Their target curve identities survived. The polynomial
retained native knots [0.5, 1] and began at (10, 7.5) mm. The rational result
retained weights [0.75, 0.75, 1] and began at (10, 3.3333333333333335) mm.
All three original constraints and the parameter-bound boundary survived.

For Extend, the native Inspector selected the rational spline and two vertical
boundaries. Preview displayed contacts (25, -5) mm and (30, -10) mm without
enabling Apply. Selecting the first contact and previewing produced a checked
five-pole result. Cancel discarded it. Setting the maximum to 5 mm produced
`No boundary lies within the bounded tangent continuation`, leaving Apply
disabled and revision 0 unchanged.

With a 40 mm maximum, selecting the first contact and applying committed one
transaction. Save As wrote `end-continuation-native-gui.multicad`. Independent
readback retained the target curve identity, original knot interval [0, 1],
original rational weights and the new endpoint (25, -5) mm.

A fresh final-build session reopened that file in Sketch mode. Opening its sketch
showed the five-pole source. A further preview toward X = 30 mm displayed
`Extend: (25.000, -5.000) → (30.000, -10.000) mm.` and enabled Apply.
Cancel restored the five-pole source, disabled Apply and removed the checked
operation report. This final preview was discarded and did not alter the saved
review artifact.

## Remaining boundaries

Supported edits operate on open clamped splines. Tangent continuation is G1 and
straight; it does not extrapolate curvature. Closed seam-aware spline trimming
and analytic extension against spline boundaries return explicit unsupported
diagnostics. These checks establish correctness for the named fixtures, not
universal CAD-kernel correctness or a measured low-power performance result.

Trim is a destructive source edit. Moving its boundary later does not implicitly
retrim the result. Old authored control points remain unchanged for reference
preservation and may remain unused. References to replaced curve identities need
explicit repair; constraints on changed target geometry may prevent Apply.
