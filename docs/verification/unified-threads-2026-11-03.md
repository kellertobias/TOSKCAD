# Unified thread verification, 2026-11-03

The bounded UNC/UNF implementation passes the targeted canonical, native,
UI and MCP checks below. Native mating pairs and manufacturing files have
independent mesh evidence. Formatting, strict Clippy and the full workspace
suite and native build pass after repairing a metric nut STEP-volume
regression. Actual OS-window
interaction and human acceptance are unverified.

## Source and supported scope

Work is local on `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Scope |
| --- | --- |
| `43a9ed5` | Sourced profiles, fit resolution, canonical guards and reader capability |
| `be2f8cd` | Preserve physical crests when trimming fractional-length helices |
| `b72bc80` | Shared UI/MCP fit control and preservation of authored nut heights |
| `f42939f` | Native mating, STEP/print artifacts and repeatable review fixture; initial verification source |
| `b4642bb` | Preserve metric helix geometry; retry segmented construction only on physical ridge loss |

The supported rows are 1/4-20 UNC, 1/4-28 UNF, 5/16-18 UNC and 5/16-24 UNF,
with 2A external and 2B internal profiles. They use uncoated, single-start UN
flat roots and threaded length up to 1.5 nominal diameters. Ambiguous Unified,
UNR rounded roots, other sizes and other fit classes remain unavailable.

The dimensional basis is NBS Handbook H28 (1969), issued March 1970,
USA B1.1-1960: profile Table 2.1/Figure 2.3 and limits Table 2.21.
The [architecture reference](../architecture/threads.md) links the exact scans,
page numbers, registry and limited OSG 2024 cross-check. Fit targets use the
published pitch-diameter midpoint as an engineering choice. Basic dimensions
remain separate from actual dimensions after fit selection and allowances.
External minor diameter is reference information; internal major diameter has
a one-sided minimum. Applicable bounds are checked after allowances.

Metric and Custom saved definitions retain their serialized schema, canonical
parameters and nominal/basic fit semantics. Native behavior still awaits the
metric regression repair and fresh full verification below.
Metric profiles remain nominal/basic; ISO 965 fit classes are unimplemented.
These checks do not certify current ASME compliance, calibrated-gage acceptance
or physical fit. The external test shafts have no screw heads or drives.

## Targeted checks

| Check | Result | Runtime |
| --- | --- | --- |
| Existing and Unified core tests | 8 passed, zero failures | Two groups, 0.01 s each |
| Native kernel thread tests, final repair | 6 passed, zero failures | 17.53 s |
| Native mechanical-part tests, final repair | 4 passed, zero failures | 4.24 s |
| Native model thread tests, final repair | 7 passed, zero failures | 38.93 s |
| Whole UI crate, final repair | 920 passed, zero failures, two ignored GPU tests | 24.94 s |
| MCP thread tests, final repair | 10 passed, zero failures | 20.05 s |

Core checks independently transcribe all four rows, distinguish nominal and
fitted pitch diameters, check mode-specific depths and reject unsupported
series, size, fit, mode, starts, length and allowances without partial commit.
Tested reader capability coverage includes direct definitions, nested nut
recipes and suppressed nested source. Independent-session persistence retains
the definitions. This does not establish every Library workflow.

UI checks use painted controls for both series and native Preview/Cancel/Apply,
Undo/Redo and independent project reopen. They retain exact inch/TPI values,
give new default threads a valid preset length and preserve authored nut
heights. MCP checks use the same canonical recipe, mode/class guards and
native compile-before-commit path. Native tests check pitch sections, mating
clearance, fitted-radius receiving walls and cosmetic manufacturing refusal.

Two failures were corrected before the initial full run. A valid
2.5 mm UNF 2B cutter lost its ridge during top trimming despite valid inputs.
The established continuous helix is retained when it passes physical checks;
only detected ridge loss triggers rebuilding with per-turn edges on the same
helix. Both paths check quarter-pitch height bands, validity, solid count and
face ancestry, with unchanged tolerances and exact authored length.
A reopened M8 nut whose recipe
equalled the defaults incorrectly acquired the new-preset height. Forwarding
the outer editing state retains its authored 12.5 mm height and requires an
explicit correction when that height exceeds Unified coverage.

Targeted logs are `/tmp/multicad-cad212-core.log`,
`/tmp/cad212-guarded-kernel-final.log`,
`/tmp/cad212-guarded-mechanical-final.log`,
`/tmp/cad212-guarded-model-threads-final.log`,
`/tmp/multicad-cad212-ui-final.log` and `/tmp/multicad-cad212-mcp-final.log`.

## Native artifacts and readback

`crates/multicad-app/examples/unified_thread_review.rs` generated
`artifacts/unified-thread-review-20261103-final/`: two TCD projects, four STEP files,
four 3MF files, four STL files and `native-report.json`. Each project contains
a 1/4-inch external shaft and a cylindrical receiving nut at the same frame.
The UNC pitch is 1.27 mm and length 5.08 mm; UNF pitch is 25.4/28 mm and length
3.628571429 mm. Both pairs have zero native material intersection.

| Part | Actual pitch diameter (mm) | Native volume (mm³) | Mesh triangles | Maximum mesh volume error |
| --- | ---: | ---: | ---: | ---: |
| UNC external shaft | 5.449570 | 120.344812 | 2000 | 0.207% |
| UNC receiving nut | 5.586730 | 271.074474 | 2224 | 0.327% |
| UNF external shaft | 5.693410 | 93.152720 | 1768 | 0.242% |
| UNF receiving nut | 5.815330 | 186.995608 | 1996 | 0.313% |

The native fixture reopens each saved project into an independent session,
checks identical canonical definitions and rebuilds volume within 0.001 mm³.
It reads each STEP back through OCCT with the same volume bound. Print files
come from the production exporter and retain attributable output receipts.

`independent-mesh-checks.json` separately parses actual STL and 3MF triangles.
All coordinates are finite, no triangles meet its degeneracy threshold, and
every welded edge has exactly two incident triangles on a 0.00001 mm grid.
Raw mesh volumes differ from native reference volumes by 0.2068-0.3273%.
STL coordinates agree with transformed, explicit-millimeter 3MF coordinates
within 0.000000238 mm. STL itself does not encode units.

Five interior transverse sections per file sample 360 angular positions at
the actual pitch radius. The independent checker inverts the nut void phase
to compare it with the external crest phase. Maximum right-hand phase error
is 0.647 degrees at a one-degree sampling step; one-pitch repetition error is
zero and quarter-pitch advance error is at most 0.5 degrees. This check reads
mesh triangles without the writer/native classifier. Native JSON supplies
reference dimensions and volumes; this independent pass does not re-evaluate
STEP B-reps or TCD contents.

## Required workspace checks and build

The first full verification on `f42939f` stopped with one failure. `cargo fmt --all -- --check`
passes. `cargo clippy --workspace --all-targets --locked -- -D warnings`
passes in 12m 01s. Workspace compilation took 17m 24s; 1126 tests passed
before the existing metric M6 nut STEP readback test failed its 0.00001 mm³
volume-difference bound. Native thread probes passed. Independent integration
methods converged to each shape's respective volume; tighter integration,
construction precision, healing and shorter sweep segments did not resolve
the discrepancy and were removed. Preserving the established continuous
metric helix restores the M6 native volume to 330.9557489284212 mm³ against
the independent analytic 330.9557489178954 mm³. STEP reads back as
330.9557489692525 mm³: the round-trip difference is 0.0000000408313 mm³,
below the unchanged 0.00001 mm³ bound. The regression now also checks native
and STEP volumes independently against the analytic volume, and probes the
readback thread material and voids.

On `b4642bb`, formatting and strict workspace Clippy pass (Clippy: 8m 44s).
Full workspace compilation took 10m 02s. The complete suite passes: 2,353
tests, zero failures, four ignored across 262 groups, including documentation
tests. The M6 nut's four-test mechanical-part group passes in 4.26s. `./build`
passes in 54.17s and produces
`artifacts/development/build-15Jvth/MultiCAD.app/Contents/MacOS/multicad`.
The bundled `--version` and `--diagnostics` both exit zero, reporting
`ToskCAD 0.1.0-dev`, macOS/aarch64 and statically linked OCCT 7.9.3. The binary
reports its source commit as unstamped; `b4642bb` is the recorded tested source
freeze, rather than a claim of embedded commit metadata.
Initial logs use `/tmp/multicad-cad212-{fmt,clippy,workspace,build}.log`;
the fresh pass uses the same names with `-final` before `.log`.

## Remaining acceptance

Actual macOS OS-window interaction, Windows/Linux CI/runtime evidence and human
acceptance are unverified. No new OS windows, installation, push or publication
were performed for these checks. Metric washer catalog facts and headed metric
screws remain separate work, with [washers](https://plans.tokenet.de/CAD/233)
preceding [headed screws](https://plans.tokenet.de/CAD/235); other queued families
remain outside this report's tested scope.
