# Native measurements checkpoint, 2026-10-28

The production macOS app measured finite faces, retained an associative pin and
updated its value after a parameter edit. Undo and Redo restored the measured
clearance. Independent reads of the saved containers rebuilt exact solids and
re-evaluated retained queries. These checks record agent verification; human
acceptance remains separate.

## Executable and automated evidence

The native review used source checkpoint `b251414` and this bundle:

```text
artifacts/development/build-CTvRKD/MultiCAD.app
```

Its `Contents/MacOS/multicad` SHA-256 was
`0dd49a091f142d972f709e1e33b6db86dce30dd514eb5a2ca69ed953aa12f060`.
The integrated checkpoint passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`: 945 passed, zero failed, two ignored.
Logs are `/private/tmp/multicad-native-transform-final-{fmt,clippy,tests}.log`.

## Face selection, retained values and history

The input was `artifacts/measurement-ui-20261028/01-ready.multicad`, generated
by the production `measurement_review` example. It contains exact Bench and
Saw envelopes with a parameter-driven gap. The reviewer opened global Measure
from Parameters and selected Face 2 on each envelope in the viewport.
The native finite minimum was 400 mm.

The selected faces share a supporting plane, so their independently labeled
supporting-plane separation was 0 mm and their angle was 0°.
The service withheld axis components and displayed a nonunique-direction
warning. It did not substitute supporting-plane separation for finite distance.

Pin measurement committed revision 1 with the default name “Measurement”.
Closing Measure restored Parameters. Changing `clearance` from 400 to 650 mm
and using Apply committed revision 2. Returning to Measure displayed 650 mm
for both the current query and retained pin. Undo restored 400 mm at revision 3;
Redo restored 650 mm at revision 4. Save As wrote
`artifacts/model-review/measurement-native.multicad`.

## Saved-source regeneration and missing sources

The `measurement_file_review` example opens the production native container,
rebuilds the complete model and evaluates retained queries through
`multicad_model::inspection::measure_model`. This run read the saved GUI file:

```text
cargo run -p multicad-app --example measurement_file_review --locked --offline -- /Users/keller/repos/_apps/multicad/artifacts/model-review/measurement-native.multicad /private/tmp/multicad-measurement-native-independent.json
```

The report confirms one visible pin with canonical face references, a 650 mm
finite minimum, zero supporting-plane separation and zero angle.
Both references resolve. Axis components remain unasserted.
The exposed `clearance` expression remains `650` in millimeters.
Bench bounds are (0, 0, 0) to (1200, 600, 900) mm; Saw bounds are
(1850, 0, 0) to (2450, 600, 1500) mm. Each envelope contains one solid.
The reader starts a new session at revision zero; saved GUI history revisions
are observations from the interactive session.

The reviewer also opened `04-source-missing.multicad` from the same fixture
directory. Measure displayed a red “Measurement source is missing or has no
current solid” diagnostic, with no stale number. The Bench mesh remained
visible and the complete model reported no geometry failure. The source
remained at revision zero.

An independent native read of that container produced
`/private/tmp/multicad-measurement-missing-independent.json`.
It contains the Bench solid, one resolved reference and one `MissingSource`
reference. The retained query has a missing-source error instead of a value.
The helper refuses an existing report path; the overwrite check retained the
original report unchanged.

## Verification limits

A fresh interactive reopen of `measurement-native.multicad` remains pending.
Its independent saved-container read and native recompilation succeeded.
The equipment envelopes do not establish physical operating or safety clearance,
and this checkpoint does not implement Room tools.

The numerical and ownership contracts, including unsupported pairs and
ambiguous directions, are described in
[measurements](../architecture/measurements.md). Automated pointer and native
regressions cover additional pin actions and reference kinds. This physical
review covers the face-selection workflow above, not every supported pair.
Windows/Linux interaction, low-power performance, manufacturing output and
subsequent Move/Copy changes are outside this checkpoint.
