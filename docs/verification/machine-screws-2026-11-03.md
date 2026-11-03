# Metric machine screw verification, 2026-11-03

CAD-235 adds retained metric socket-cap and external-hex screw recipes, native
geometry, editable Library definitions and shared UI/MCP control. Required
formatting, strict Clippy, the full workspace suite, native packaging and the
built-application smoke pass. Actual macOS OS-window
interaction and human acceptance remain unverified; automated results below
do not establish those outcomes.

## Source and scope

Work stays on local `main` in `/Users/keller/repos/_apps/multicad`.

| Commit | Scope |
| --- | --- |
| `8ce8237` | Exact helix endpoints during bounded retries; unchanged convergence guard |
| `141da5a` | Canonical screw recipe, native geometry, UI and regression tests |
| `a6d5e15` | Equivalent UI conditional cleanup required by strict lint |
| `8d73609` | Mechanical screw MCP controls and generic pinned-symbol import/configuration |
| `58773c1` | Curated source, nineteen native projects/previews, supplier and packaging closure; frozen runtime source |

The [architecture reference](../architecture/machine-screws.md) defines the
underhead frame, separate head/drive types, derived plain shank, real six-sided
socket and shared helical thread builder. Cosmetic threads cannot become
manufacturing geometry through STEP, mesh or print export.

The [curation](../../library/machine-screws-curation.json) contains nineteen
selected metric presets with explicit coverage for all twelve requested
nominal diameters. Eight socket sizes run from M2 through M10, with selected
longer partial-thread variants. External-hex presets cover M4, M5, M6, M8 and
M10. Unavailable rows mean no preset in this selected source matrix, rather
than physical nonexistence. Custom parametric dimensions remain available.

Socket dimensions combine HPC 2025's selected lengths/thread lengths and head
envelopes with declared actual socket targets within NBK's published ranges.
Hex dimensions use selected Norelem DIN 933 product rows and a separately dated
Nufast cross-check. M10 DIN AF17 remains distinct from ISO AF16. Material and
grade are unspecified. No stock, fit-class, strength or current-standard
certification is asserted. Flat socket bottoms, sharp head edges and omitted
tooling/fillets/chamfers/thread transitions remain declared limitations.

Original BOLTS `hex_socket.blt` and `hex.blt`, LGPL-2.1-or-later licensing,
hashes, adaptations and primary references are recorded in the bundled
[notice](../../library/sources/bolts/MACHINE-SCREW-NOTICES.md).

## Focused automated evidence

| Check | Recorded result |
| --- | --- |
| Canonical screw, catalog-fact and legacy mechanical targets | 16 passed |
| Kernel thread target, including complete/fractional metric endpoints | Seven passed; 18.94 s |
| Native screw target, including minimum-feature regression | Six passed; 38.62 s |
| Actual shipped screw catalog and complete coverage target | Two passed; zero failed; 647.95 s |
| Unchanged legacy native mechanical target | Four passed; 4.24 s |
| UI unit target, after lint corrections | 925 passed, zero failed, two ignored; 26.62 s |
| UI integration targets | 46 passed; zero failed |
| Mechanical-part MCP target | Six passed; 15.05 s |
| Generic symbol MCP target excluding shipped-pin integration | Six passed; 13.70 s |
| Complete generic symbol MCP target with shipped pins | Seven passed; zero failed; 188.05 s |
| Complete application target with authorized local sockets | 70 passed; zero failed; one ignored; 23.83 s |
| `npm test` packaging/release contracts | 32 passed; zero failed; 4.451 s |

Native tests independently probe socket flats/corners/floor, head forms,
plain shanks, helical crest/gaps, rotated frames and both handedness values.
They exercise native export/readback, modeled versus cosmetic refusal,
preview/cancel/apply, Undo/Redo and independent save/reopen. New volume checks
use the declared linear geometry policy and unchanged numerical integration
budget alongside independent bounds and material probes. Existing legacy
absolute thresholds remain unchanged.

The actual shipped-asset target checks every one of the nineteen independent
expected rows against its curation, manifest hash and saved source. It compiles
both the authoring document and immutable template, then measures all 38 native
solids and their STEP readbacks. Checks cover head/flats/socket/floor, shank,
early and late helical phase/material, bounds and policy-derived volume
agreement. The separate coverage test verifies all twelve per-family nominal
statuses without inventing unavailable presets.

The initial native screw run exposed a continuous M3 thread integration
relative error of approximately `1.552e-7`, above the existing `1e-7` guard.
The bounded fallback then exposed an almost-zero final edge at a complete-turn
boundary caused by accumulated floating-point span additions. Indexed spans
now retain the exact endpoint and merge only machine-scale remainders.
Only typed ridge-loss or volume-integration failures trigger the alternative
representation; both attempts enforce the unchanged native volume guard.
Diagnostic tighter-tolerance, quarter-span and healing experiments were
removed. Independent material-difference checks in both directions were empty
for seven metrology cases, with native/STEP bounds agreeing within
`2.7e-13 mm`. The legacy M6 nut retains its existing absolute `1e-5 mm³`
round-trip check and passes.

Generic MCP import, placement and replacement-map instance overrides share
canonical symbol ownership and candidate-native preflight. Owner-session and
revision checks occur inside the same exclusive closure as review or commit.
Tests cover owner edits, session replacement, cancellation and draft state
between preparation and completion. All seven tests pass, including the
shipped M3 socket partial-thread and M6 hex full-thread instances. Configuring
head height retains their pins, output identities and provenance, then saves,
removes the copied source and independently reopens/exports native STEP.
The initial race fixture used an invalid parameter name; correcting that
fixture allowed the test to exercise and pass all four interleavings.

Independent read-only reviews found no remaining actionable issue in the
native minimum-feature guard, generic-symbol reconciliation or source/docs
contracts. An initial new regression assertion did not compile because its
success type does not implement `Debug`; the assertion was corrected without
changing production behavior. The corrected six-test target passes.

## Catalog integration and remaining checks

The first generation produced nineteen actual native previews and preserved
the prior 38 project bytes and manifest records. Visual inspection found that
the original camera hid the socket opening. The renderer now rotates only a
temporary preview occurrence, preserving the authored underhead frame and
definition pins. That inspection also exposed the authored source body being
rendered alongside the rotated occurrence. Symbol previews now filter by the
temporary instance root; generic full-model previews retain all leaves. The
first corrected M2 thumbnail shows one screw with a visible hex socket and
modeled thread. Complete regeneration passed at
`/private/tmp/cad235-library-review-single`. All nineteen previews were
extracted and visually checked in
`artifacts/machine-screw-review-20261103-final/native-previews.png`.

Closure validation passes for 57 projects and 69 packaged files. Every prior
project byte and manifest record remains unchanged. All nineteen corrected
previews preserve their complete immutable definitions/templates from the
initial generation. The new nineteen TCDs and manifest are copied into
`library/`; source and notice files are bundled with them.

Formatting and strict locked workspace/all-target Clippy pass (Clippy: 7m 48s).
The first lint attempt also encountered a sandbox denial opening the existing
native cache lock. The authorized rerun uses the existing cache; no build
policy or dependency pins changed. Rust lint findings were equivalent
conditional simplifications and an example-module import visibility fix.

The initial application target passed 67 tests, with one ignored; three
local-socket tests failed because the sandbox refused listener creation.
No production change was made for that environment restriction. The required
full locked workspace run uses authorized local-socket/cache access. On
frozen `58773c1`, formatting and strict Clippy pass again (Clippy: 0.43 s).
Test compilation completes in 19m 23s, and the application target passes all
70 tests with one ignored in 23.83s. The complete workspace suite passes:
2,389 tests, zero failures and four ignored across 268 groups, including
documentation tests and the actual nineteen-file native/STEP target.

`./build` passes in 55.32s and produces
`artifacts/development/build-nyBghF/MultiCAD.app/Contents/MacOS/multicad`.
The actual bundled `--version` and `--diagnostics` exit zero and report
`ToskCAD 0.1.0-dev`, macOS/aarch64 and statically linked OCCT 7.9.3. Source
commit metadata is unstamped; `58773c1` is the recorded tested source rather
than embedded commit metadata. The actual bundle validates 57 Library projects
and 69 packaged files, all byte-identical to the repository.

The actual bundled executable passes the real stdio MCP smoke in 19 tool
calls, approximately 196s. It imports the shipped M3 L30/B18 socket-cap pin,
places it, changes underhead length to 35 mm while retaining 18 mm of thread,
and saves/exports STEP. After the copied source file is removed, a separate
process reopens the saved consumer with the same pin, identities, provenance
and dimensions. Editing to 36 mm and Undo restore 35 mm, followed by a second
successful STEP export. Both processes close with exit zero.

Both exports report native volume `292.1690845738779 mm³` and STEP readback
volume `292.1690966587257 mm³`; the reopened source retains the same native
and readback values. This does not assert byte-identical STEP metadata. The
saved consumer, two STEP files, checksums and RPC transcripts are retained
under `artifacts/machine-screw-mcp-20261103/`; `smoke-report.json` records the
executable hash, calls and results. The exchange folder is `exchange-J64ZFj`.

Focused logs use `/tmp/multicad-cad235-*`. Frozen formatting/Clippy use the
`-frozen` suffix; full workspace/build logs are `-workspace.log` and
`-build.log`; the actual executable smoke log is `-stdio-smoke.log`.

Actual macOS OS-window interaction, Windows/Linux CI/runtime and human
acceptance remain unverified. No installation, publication, push or new OS
windows have been performed for this item. Automated completion is ready for
Review/test; it is separate from human acceptance.
