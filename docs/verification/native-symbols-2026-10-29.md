# Native symbol verification, 2026-10-29

Pinned static and dynamic symbols passed source, exact native geometry,
transactional, archive and macOS workflow checks. These are engineering results;
human acceptance and Windows/Linux runtime verification remain separate.

## Source and required checks

Production through `29a3afa` passed the required checks. The complete locked
workspace run passed 1,307 tests, with zero failures and two existing ignored
tests. All 25 release-contract tests passed.

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline
npm test
CARGO_INCREMENTAL=0 ./build
```

Final Rust/build logs are `/private/tmp/multicad-cad70-handoff-{clippy,tests,build}.log`.
The release-contract log is `/private/tmp/multicad-cad70-final-npm.log`.
Native commands used approved access to the shared OCCT cache. The existing
upstream `sprintf` deprecation is unchanged.

| Bundle | Executable SHA-256 | Check |
|---|---|---|
| `artifacts/development/build-GlEU4a/MultiCAD.app` | `4090627b7f83eda665cef65b35a9dc0529d32a7520baf82953faf549903a978e` | Actual native placement, independent copy/edit, library save/load, reviewed update, history and Save As |
| `artifacts/development/build-C0UTpc/MultiCAD.app` | `7d1a896c5e73ce80b9f253701593b948b6aac164173446c145fde4f34497e29e` | Final integrated build through `29a3afa`; not launched |

The interaction bundle includes the independent-copy implementation `7413f4d`.
The final build also hides the generated wrapper's independent assembly-edit
action. Native review instances were closed. Nothing was installed, pushed or
published.

## Automated native coverage

The [pinned-symbol decision](../architecture/0024-pinned-symbol-variants.md)
records the source and identity model. The production compiler evaluates each
embedded definition's native template before publishing a complete model,
including definitions with no placed instances. Source-only repair inspection
remains explicitly incomplete.

| Behavior | Evidence |
|---|---|
| Author-selected root, exposed parameters, static defaults and dynamic overrides | [Core tests](../../crates/multicad-core/tests/symbols.rs), native UI controller tests and source-neutral invalid-input cases |
| Two dimensions and placements use one pinned definition | [Native model tests](../../crates/multicad-model/tests/symbols.rs), exact world bounds and volumes |
| Internal constrained sketches and derived parameter chains survive | Genuine fully constrained panel/extrusion fixtures; private depth expression and exposed-formula override checks |
| Linked 2D/3D identity | Mapped plan sketch and outer Instance identity remain stable; actual solved area and native placed B-rep bounds agree |
| Explicit updates retain compatible values and maps | Changed constraints/defaults/exposure, removed-input conflicts, Undo/Redo and offline archive regeneration |
| Generic Move/Copy creates an independent symbol | Actual nested two-solid native copy; fresh wrapper/map/plan and private driver bindings; editing the original leaves copy geometry and pose unchanged |
| Native reference survives compatible upgrade | Captured real face provenance resolves after the changed definition regenerates |
| Identity and ownership failures reject atomically | Forged removed-ID reuse, aliased namespaces, direct generated-source edits, stale/foreign Session and changed resources |
| Unplaced invalid native definition cannot authorize Apply/export | [Definition-gate tests](../../crates/multicad-model/tests/symbol_definition_gate.rs), negative geometry and empty roots |
| Library I/O leaves project source and save state intact | [Host lifecycle tests](../../crates/multicad-ui/src/application/symbol_file_tests.rs), stale-target rejection and actual native picker workflow |
| Responsive forms retain checked Apply authority | Cached catalogues/defaults, native background candidate worker, physical-leaf receipt checks and fixed Inspector actions |

One integration failure exposed parameter normalization detaching inputs in an
intermediate document. Commit `b3ae8ae` computes all parameter bindings from the
original aliases and applies them atomically. Final ownership validation remains
active. Another regression distinguished local sketch identities from canonical
Document entity IDs in native candidate proof (`00df060`).

## Actual macOS workflow

The native system picker opened `part-r1.multicad` from
`artifacts/symbol-review-20261029-0108/`. Parameters displayed the public project
parameter while private generated parameters stayed out of the table. Model's
normal tree showed the two symbol instances, with private content behind Show
sources.

Library listed the pinned dynamic definition with three exposed inputs and a
linked 2D/3D representation. Place exposed only those inputs. The new instance
was named `Panel C native review`, with width 180 mm and world X = 550 mm.
Native Preview enabled Apply; applying committed revision 1. Header Undo removed
it at revision 2, and Redo restored the same instance at revision 3.

Editing that instance to 210 mm produced another checked native preview.
Cancel restored the unchanged 180 mm source. The shared Move/Copy tool then
enabled Create copy and a 250 mm Y offset. Preview and Apply committed revision
4. The copy offered its own symbol parameters; setting its width to 160 mm and
applying committed revision 5. Native Save As wrote
`symbols-place-copy-native-gui.multicad` without replacing the fixture.

Library's Save definition opened the operating-system Save dialog and wrote
`panel-library-native-gui.multicad`. The status explicitly retained the project
save state and revision. Compare update loaded `part-r2.multicad` through the
system Open dialog into the library catalogue without opening a new project tab
or changing canonical revision 5.

Selecting revision 2 displayed old/new pins, four changed source entities, four
affected instances, changed defaults and the removed height input. The old
height overrides on Panel A and Panel B were explicitly unchecked. Native
Preview displayed the regenerated geometry and enabled Apply. One Apply updated
all four instances and their pin at revision 6. Undo restored revision 1's design
at revision 7; Redo restored the upgrade at revision 8. Native Save As wrote
`symbols-reviewed-upgrade-native-gui.multicad`. The review instance was quit.

## Independent saved-source results

The production `model_file_review` reader rebuilt both GUI-saved projects and
the GUI-exported library. Reports are alongside them with `-report.json`
suffixes. All recorded references resolve. Both modeling projects retain the
same four outer Instance identities and four linked sketches/physical leaves.

| Instance | Before update, native volume mm³ | After update, native volume mm³ |
|---|---:|---:|
| Panel A | 108,000 | 106,400 |
| Panel B | 240,000 | 216,600 |
| Panel C native review | 115,200 | 161,500 |
| Placed copy | 102,400 | 142,500 |

Before update, Panel C spans (550, 0, 0) to (730, 80, 8) mm. Its independently
edited copy spans (550, 250, 0) to (710, 330, 8) mm. After update their new
geometry follows the revised constraint and defaults while retaining those
placements. The exported library reopens with the native reader and passes
template validation without materializing host-project occurrences.

The no-overwrite Rust `symbol_review` generator also writes four editable Part
and two-solid Assembly projects plus a manifest. It verifies actual constrained
plan areas, native solids, explicit conflict resolution and library-independent
reopen. Its generated files alone are not physical GUI acceptance.

## Boundaries and review notes

The supported template remappers currently cover constrained sketches,
primitives, extrusions, ordinary assemblies and their supported placement/joint
records, basic materials and typed inline hatches. Unknown/opaque providers and
unsupported assigned material-library metadata reject packaging explicitly.
They are not silently copied or flattened. Nested embedded symbol providers are
not accepted as templates; ordinary canonical Part/Assembly content can be
authored instead.

Each placement initially has its own generated source graph, including static
symbols. Templates, placements and native preflight have explicit budgets.
Generic subassembly alignment retains its existing unsupported boundary.
Library export preserves an existing file and asks for another revision filename
on a collision. Automatic library updates and external network lookup are absent.

The upgrade report retains the original conflict summary after a user unchecks
an obsolete override; Preview/Apply use the resolved current controls. That
summary could be clearer. Some long scrolled native Inspector controls required
coordinate interaction during computer-use verification, although their visible
fixed actions worked. No low-power performance, Windows/Linux runtime, remote
connected MCP transport or human workflow acceptance is claimed here.
