# Sourced washer catalog verification, 2026-11-03

CAD-233 adds ten sourced normal-series washer definitions to the existing
Library. Focused canonical, native, UI, supplier and packaging checks, full
workspace checks and the native application build pass. Actual
macOS OS-window interaction and human acceptance are unverified.

## Source and nominal coverage

Work stays on local `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Scope |
| --- | --- |
| `837514c` | Optional literal catalog facts, reader capability and compatibility tests |
| `03c436f` | Exact nominal search and source facts in installed, embedded and loaded Library views |
| `46f664e` | Curated data, ten native files/previews, supplier and packaging closure |
| `516acab` | Actual shipped geometry and configured-instance ownership regressions; final runtime source |
| `ed7abf7` | Update the existing Library integration count; final verification source |

[Curation](../../library/washers-curation.json) owns the supported rows and all
twelve requested nominal statuses. Dimensions below are millimeters; nominal
fastener diameter is separate from physical bore.

| Nominal | Bore | Outside | Thickness | Status |
| ---: | ---: | ---: | ---: | --- |
| 2 | 2.2 | 5 | 0.3 | Supported |
| 2.5 | 2.7 | 6 | 0.5 | Supported |
| 3 | 3.2 | 7 | 0.5 | Supported |
| 3.5 | 3.7 | 8 | 0.5 | Supported |
| 4 | 4.3 | 9 | 0.8 | Supported |
| 4.5 | — | — | — | No preset in this selected series; custom geometry remains available |
| 5 | 5.3 | 10 | 1 | Supported |
| 6 | 6.4 | 12 | 1.6 | Supported |
| 7 | 7.4 | 14 | 1.6 | Supported |
| 8 | 8.4 | 16 | 1.6 | Supported |
| 9 | — | — | — | No preset in this selected series; custom geometry remains available |
| 10 | 10.5 | 20 | 2 | Supported; declared upstream bore correction |

The source is BOLTS `plainwasher1`, commit
`5c83300a57761f70db87f50c9a1f647c7278a4c2`, authored by Johannes Reinhardt
in 2013 under LGPL-2.1-or-later. The original `washer.blt`, complete license,
adapted data and dated modification notice are bundled. Its source SHA256 is
`0f04b56108c958275b05bb1fcf915d6ba971bd2f20cdce70da9c1de75b93c326`.

The [source notice](../../library/sources/bolts/WASHER-NOTICES.md) records the
NBK JIS B 1256:2008 normal-series PDF page 2 cross-check, and the identified
RECA M7 product cross-check without an invented edition date. M10's upstream
10.0 mm bore is explicitly corrected to 10.5 mm. Normal M3 outside diameter is
7 mm; NBK's small-series 6 mm row is separate. Manufacturer artwork and prose
are not redistributed. Material and grade are unknown; no variants are
inferred from another supplier's coating or hardness. Sharp annular geometry
omits tolerances and edge breaks/chamfers. No current-standard, stock or
physical-fit certification is asserted.

## Canonical source and Library behavior

Optional `catalog_facts` live inside retained `MechanicalPartProvenance` and
propagate to source cards. Missing facts are unknown, and absent fields retain
legacy serialization. Nominal values must be finite and positive; present
text is bounded and nonempty. Physical edits retain source facts while
reference matching reports custom dimensions. UUID-looking descriptive facts
remain literal during symbol remapping.

Facts-bearing records require `model.mechanical_part.catalog_facts.1`, including
suppressed and template-only definitions. A reader without the capability
refuses the file. Existing capability inventories remain unchanged when no
facts are present. Bare `M3` searches require exact sourced nominal 3 mm, so
M3.5, M30, missing facts and misleading filenames cannot pass. Other terms
stay on the same card; ordinary physical `3.2` searches remain supported.
Cached typed indexes follow installed, embedded and loaded snapshots.

The generator wrote `/private/tmp/cad233-library-review`, preserving all 28
prior project bytes and manifest records and adding ten washer TCDs. Those
ten files and the 38-entry manifest were copied into `library/`; old bearing
revisions and pins remain unchanged. Manifest/source closure validation passes
for 38 projects and 46 packaged files. The supplier reads checked project
cards without native evaluation. Definitions expose bore, outside diameter
and thickness through the existing dynamic-symbol configuration path.

All ten embedded native preview PNGs were extracted and visually inspected in
`artifacts/washer-review-20261103/native-previews.png`. They show recognizable
annular solids. This is native model-render evidence, not OS-window discovery
or placement evidence.

## Focused verification

| Check | Result |
| --- | --- |
| Canonical catalog/legacy hardware | Nine passed; zero failed |
| Actual shipped native washers | Two passed; zero failed; 0.50 s |
| Mechanical-part MCP/ownership | Four passed; zero failed; 3.39 s |
| Whole UI crate | 922 passed; zero failed; two ignored GPU tests; 24.90 s |
| Shared Library supplier | Three passed; zero failed; 0.52 s |
| Complete application target after integration-count correction | 70 passed; zero failed; one ignored; 23.64 s |
| `npm test` packaging/release contracts | 32 passed; zero failed; 4.769 s |

Native tests validate each actual shipped file's manifest hash, compile both
its authoring document and immutable template, and independently measure
outside/thickness bounds, bore/outside cylindrical areas and analytic annular
volume. All twenty solids undergo STEP export/readback with the same checks.
Independent expected rows also verify the complete coverage matrix and reject
invalid physical dimensions without mutation.

The configured-instance test uses a synthetic sourced M3 fixture with an
exposed outside dimension changed from 7 to 8 mm. Typed owner overrides retain
nominal 3 mm, reference dimensions, pin and output identities; MCP inspection,
Undo/Redo, save/independent reopen and STEP export preserve that state. Direct
mechanical-source edits to symbol-owned geometry are atomically refused. No
generic MCP symbol-configuration tool is claimed. The initial test wrongly
attempted such a direct edit and was corrected after the ownership guard
refused it. A separate supplier command mistakenly selected `--lib` for the
binary-only app; the corrected `--bin multicad` invocation passes.

Independent source reviews found no actionable issues in the core reader,
catalog/packaging, native ownership or UI matching/cache changes. Logs use
`/tmp/multicad-cad233-{core,native-final,mcp-final,ui,supplier-final,packaging}.log`.

## Full verification and remaining acceptance

The first run on `516acab` passed formatting and strict locked
workspace/all-target Clippy (30m 10s). Full test compilation took 33m 44s,
then the application target stopped with 69 passes, one failure and one
ignored test: the original Library integration test still expected 28 included
files instead of the new 38. No production behavior changed to resolve it.
`ed7abf7` updates that expectation, and the complete application target passes
70 tests with one ignored in 23.64s.

On `ed7abf7`, formatting and strict Clippy pass (Clippy: 8.38 s), and full test
compilation completes in 9.15 s using the existing artifacts. The complete
suite passes: 2,361 tests, zero failures, four ignored across 264 groups,
including documentation tests. `./build` passes in 40.77s and produces
`artifacts/development/build-ruaZO7/MultiCAD.app/Contents/MacOS/multicad`.

The bundled `--version` and `--diagnostics` both exit zero and report
`ToskCAD 0.1.0-dev`, macOS/aarch64 and statically linked OCCT 7.9.3. The binary
reports its source commit as unstamped; `ed7abf7` is the recorded tested source,
not a claim of embedded commit metadata. The actual app bundle's Library
passes container, identity, hash and source/license closure checks for all
38 projects and 46 packaged files. Every bundled file is byte-identical to
its corresponding repository file.

Initial logs use
`/tmp/multicad-cad233-{fmt,clippy,workspace,build}.log`; the fresh run adds
`-final` before `.log`.

Actual macOS discovery/configuration/placement, Windows/Linux CI/runtime and
human acceptance are unverified. No installation, push, publication or new OS
windows were performed. Headed metric screws are the next prioritized outcome,
[CAD-235](https://plans.tokenet.de/CAD/235), once this prerequisite is reviewed.
