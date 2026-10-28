# Native sketch references checkpoint, 2026-10-28

The production macOS application projected a linked model edge into a sketch,
updated it after a source edit and selected a finite plane intersection.
It also repaired a reference while its dependent solid could not rebuild.
Independent reads of the saved native projects regenerated the solved geometry
and exact solids. These checks record agent verification, not human acceptance.

## Executable and automated evidence

The review used the frozen source checkpoint `19f7f7a` and this bundle:

```text
artifacts/development/source-19f7f7a/artifacts/development/build-tOzBvB/MultiCAD.app
```

Its `Contents/MacOS/multicad` SHA-256 was
`f76583aff80c5783bc4d7da193156268b2368e05d3ee1f5ce693f05ef49aeaea`.
The checkpoint passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`: 869 passed, zero failed, two ignored.
Logs are `/private/tmp/multicad-frozen-19f7f7a-{fmt,clippy,tests}.log`.

## Projection, source edit and intersection

The initial input was
`artifacts/external-reference-review/reference-start.multicad`.
Opening Top projection at Z10 exposed the Sketch reference tools. Project with
Derived geometry selected the native source edge at Y0, Z10. Preview solved a
30 mm line; Apply committed revision 1. Undo and Redo restored both geometry
and its source link at revisions 2 and 3.

Editing the source primitive width from 30 to 40 mm produced a current native
preview. Apply committed revision 4. Reopening the sketch displayed a 40 mm
line and reported native source geometry available. The saved solved line ran
from (0, 0) to (40, 0) in its plane and remained non-construction geometry.
Its original authored point guess remained 30 mm; linked evaluation supplied
the current endpoint.

Wall intersection at Z5 used Intersect plane and selected the visible vertical
edge at X40, Y20. Preview and Apply committed revision 5; Undo and Redo reached
revisions 6 and 7. The solved intersection was (40, 20) in a plane whose world
origin was (0, 0, 5). Save As wrote
`artifacts/model-review/sketch-references-native.multicad`.

## Repair with a broken dependent solid

The separate input was
`artifacts/external-recovery-review/repair-broken.multicad`.
Its original wall height was 2 mm, so the referenced finite edge did not reach
the sketch plane. The complete model reported that failure.

Repair reference selected the explicit replacement-wall Feature. Load selected
source geometry displayed an incomplete source view. The review selected its
vertical edge at X50, Y0 from the minus-Y view. Preview checked the complete
repaired candidate before enabling Apply. The repair committed revision 1,
cleared the full-model failure and retained one fully constrained 5 × 7 mm
profile. Undo restored the original failure; Redo restored the repair.

Save As wrote `artifacts/model-review/sketch-repair-native.multicad` at revision 3.
The original wall remained unchanged. The independent native report confirmed
these results:

| Result | Saved-source regeneration |
| --- | --- |
| Replacement wall | 50 × 20 × 10 mm |
| Linked intersection and coincident own corner | (50, 0) in the Z5 sketch plane |
| Other profile corners | (55, 0), (55, 7), (50, 7) |
| Dependent extruded Part | One solid, 70 mm³ |
| Dependent world bounds | (50, 0, 5) to (55, 7, 7) mm |
| Reference report | Every recorded reference resolved |

## Saved artifacts and limits

The production store, Compiler and checked sketch context in reader checkpoint
`b3908dc` generated `artifacts/model-review/sketch-references-native-report.json`
and `artifacts/model-review/sketch-repair-native-report.json` from the saved
containers. The reports retain canonical guesses separately from solved points,
source provenance, reference diagnostics and exact native body measurements.
Both saved projects were subsequently reopened in fresh Sessions in macOS build
`build-Fd6A5g`, source `e1ce67f`, executable SHA-256
`2bb6d06193518d80feec245a6383e00ff74abc49214ae3405a173c1d86893c1d`.
Top projection at Z10 reopened with its 40 mm line, Derived geometry link and
native source available. The repaired Wall intersection at Z5 reopened with
its replacement-wall reference, four 5/7/5/7 mm edges, one closed area and
Fully constrained state. Its dependent Part remained in the regenerated model.
The current integrated workspace checks passed 948 tests, zero failures and two
existing ignored tests; formatting and strict all-target Clippy also passed.
Logs are `/private/tmp/multicad-coalesced-drag-{fmt,clippy,tests}.log`.

The current UI supports Feature sources in an identity target context.
Arbitrary placed-occurrence projection remains unsupported. Intersections are
limited to finite straight edges; other analytic families and ambiguous results
need explicit diagnostics. Incomplete source views cannot authorize model or
export results. Repair handles one broken binding per transaction, and other
candidate failures block Apply.

All review tabs were saved before Quit, and application inventory confirmed
that every review instance stopped. These checks do not establish subsequent
Move/Copy workflow changes, general manufacturing readiness, Windows/Linux
interaction or low-power performance.
