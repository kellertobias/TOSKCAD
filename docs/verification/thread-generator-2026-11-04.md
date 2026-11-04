# Thread generator integration verification, 2026-11-04

CAD-198 integrates the native thread generator and the bounded family increments
CAD-212 through CAD-219. The final compatibility fix is committed on main as
3f461dc. Its seven focused saved-reader tests pass. The new native package,
self-test and four independent saved-thread sessions pass. Required workspace
checks passed, including 2,553 tests with zero failures and four ignored.
Human acceptance remains separate.

## Supported family matrix

The [thread reference](../architecture/threads.md#family-coverage-and-sources)
owns the supported sizes, editions, geometry, fit conventions and exclusions.
Its matrix covers ISO metric, UNC, UNF, BSW, BSF, BSPP, BSPT, NPT,
Trapezoidal, Acme, Buttress, Round and explicit Custom profiles.
The ambiguous legacy Unified and Whitworth selectors remain unavailable.

Metric basic profiles include the listed coarse/fine pairs. The fitted subset
is M3x0.5 and M6x1, single-start 6g external/6H internal, within the documented
normal engagement bounds. It keeps source class limits, target dimensions and
fabrication allowances distinct. Other classes and sizes require additional
source and implementation work.

| Increment | Reviewed implementation evidence |
| --- | --- |
| CAD-212, UNC/UNF | [Unified report](unified-threads-2026-11-03.md) |
| CAD-213, BSW/BSF/BSPP | [Rounded 55-degree report](rounded-threads-2026-11-03.md) |
| CAD-214, BSPT | [Tapered pipe report](tapered-pipe-threads-2026-11-03.md) |
| CAD-215, NPT | [NPT report](npt-pipe-threads-2026-11-03.md) |
| CAD-216, Trapezoidal/Acme | [Lead-screw report](lead-screw-threads-2026-11-03.md) |
| CAD-217, Buttress | [Buttress report](buttress-threads-2026-11-04.md) |
| CAD-218, Round | [Round report](round-threads-2026-11-04.md) |
| CAD-219, metric fits/endpoints | [Metric report](metric-thread-fits-2026-11-04.md) |

These reports record agent verification and Review/test disposition. They do
not establish human acceptance or conformity to every current standard edition.
Natural helical runouts, DIN 76 shoulder relief, nested hardware endpoint
junctions and sourced wood-screw hardware presets remain outside supported
coverage. Explicit custom entry/terminal relief is described in the metric
report and is not labeled as a standard shoulder treatment.

## Acceptance evidence

| CAD-198 criterion | Observable implementation and verification |
| --- | --- |
| Discoverable Mechanical Thread tool and explicit conventions | [Registry](../../crates/multicad-ui/src/registry.rs), shared inspector and family-specific painted production-egui tests; explicit diameter, pitch, lead, hand, starts and receiving modes |
| Sourced matrix and unsupported rejection | Canonical [profile registry](../../crates/multicad-core/src/model/threads/profiles.rs), thread reference and family source tests; separate Custom parameters and explicit unavailable combinations |
| Native metric mating, left hand, multiple starts and tapered pipe | [Metric model tests](../../crates/multicad-model/tests/threads.rs), [lead-screw tests](../../crates/multicad-model/tests/lead_screw_threads.rs), [BSPT tests](../../crates/multicad-model/tests/pipe_threads.rs), [NPT tests](../../crates/multicad-model/tests/npt_threads.rs) and family reports; actual solids, sections, pitch/lead, receiving cuts and exchange readback |
| Independent fit/allowances and atomic failures | [Metric fit source tests](../../crates/multicad-core/tests/model_metric_fits.rs), [native fit tests](../../crates/multicad-model/tests/metric_fit_threads.rs) and family regressions; invalid walls, incomplete receivers, engagement and treatment dimensions fail before commit |
| Modeled versus cosmetic export | Shared definition and [model tests](../../crates/multicad-model/tests/threads.rs); cosmetic recipes are refused by STEP/print paths; retained native STEP/STL/3MF fixtures contain measured helical geometry |
| Stable history and shared API | Canonical transactions, model/UI/MCP regressions and saved-session fixtures exercise Preview/Cancel/Apply, Undo/Redo, parameter edits, stable identities, independent reopen and explicit repair |
| Required checks and separate OS evidence | Current checks below; historical actual macOS foundation evidence below; current expanded-family UI checks are offscreen |

## Saved-reader integration fix

The aggregate audit found that template-only Thread and Mechanical Part source
could omit their base reader requirements. Unified source also omitted its
additional family reader when saved only inside a reusable symbol, or solely
as retained hardware reference geometry. An older reader could consequently
accept a container containing source it did not support.

[Capability detection](../../crates/multicad-core/src/model/source.rs) now uses
the existing saved-feature walker for these gates. It covers active and
suppressed features plus supported symbol templates and retained hardware
reference paths. Nested thread parameters require their Mechanical Part
provider rather than a standalone Thread feature reader. Other provider
inventories are unchanged; nested symbol definitions remain unsupported.

Four [regressions](../../crates/multicad-core/tests/model_unified_threads.rs)
construct actual template-only imports and retained references, write real
project containers, prove refusal with the relevant reader removed and reopen
with the supported reader. The two Unified regressions failed before their
fix; both base-provider regressions failed before theirs. All seven tests in
the final test binary pass in 0.04 seconds. Independent read-only code review
found no further thread capability gaps.

Before/after logs: [Unified before](/private/tmp/cad198-hidden-reader-before.log),
[base providers before](/private/tmp/cad198-base-reader-before.log),
[final seven tests](/private/tmp/cad198-readers-after.log).

## Actual macOS evidence and current limits

The historical [foundation report](../../artifacts/thread-review-20261101/review.json)
is pinned to d787a08 and the build-Ajvhtb executable. It records actual macOS
metric internal/external creation, left/two-start pitch versus lead,
Preview/Cancel/Apply/Undo/Redo, fresh-process save/reopen/edit, incomplete
receiver refusal and cosmetic manufacturing refusal. An explicit Custom
left/three-start tapered example was also saved and independently read back.
Real STEP/3MF/STL output retained the measured native/mesh geometry. Four owned
GUI processes exited normally.

That evidence predates the expanded family and fit increments. Their reports
separately record current painted production UI and packaged native MCP checks.
No fresh OS-window interaction, Windows/Linux runtime, physical gage/fit/load,
printer acceptance or human acceptance is claimed for this integration.

The [metric performance report](metric-thread-fits-2026-11-04.md#metric-cache-and-required-verification)
records an isolated packaged default M3 first creation of 537.102 ms and a
second identical feature of 9.718 ms. First fitted 6g M3 creation improved from
18.528 s to 6.232 s through parallel native joins; the unchanged fitted edit
was 7.515 ms and a second identical fitted feature 20.377 ms. These single-run
native observations use the prior package and remain attributable to that
source. A first fitted construction is still noticeably slow. The reader-only
integration fix changes no native geometry or performance path.

## Final integration checks

Required `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` and
`cargo test --workspace --locked` passed on frozen 3f461dc. The suite passed
2,553 tests across 291 result groups with zero failures and four ignored in
2,972.051 seconds. Formatting took 2.918 seconds and Clippy 96.990 seconds.
[Runner result](/private/tmp/cad198-required-checks-result.json).

The native development package built successfully at
artifacts/development/build-BkYnha/MultiCAD.app. Executable SHA256:
4c7b8e2e151f3caa768899a484f9adf8706e9f8dc57eb9efa56a9281876b0eb8.
[Diagnostics and native self-test](/private/tmp/cad198-runtime-result.json)
passed. [Four independent stdio sessions](/private/tmp/cad198-runtime-fixtures-result.json)
reopened both fitted metric sizes and hands, compared complete canonical source
and native volumes, then exported both members with STEP readback. Each process
exited normally with code zero. These runs overlap workspace compilation and
are not isolated latency measurements.

Agent verification supports requesting Review/test for this bounded generator
outcome. Human acceptance remains outstanding. No installation, push or
publication occurred.
