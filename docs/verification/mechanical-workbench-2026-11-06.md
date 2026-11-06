# Mechanical workbench verification

Mechanical is a Model workbench with canonical Thread, Gear and mechanical-part
forms, the shared global Library, configurable navigation and existing assembly
integration. The five workbench criteria have implementation and native macOS
evidence below. The current full workspace suite passed 2732 tests with zero
failures and four intentional ignores. Catalog completeness, additional
connection workflows and human acceptance remain separate.

The current [runtime](../../artifacts/mechanical-complete-20261106/runtime.json)
is the packaged macOS build from `d91ba92b9d72c707bc26b70f73aa30b973a7828e`,
executable SHA-256
`b2844e0194b2e2803520d580e32f771da4ef8cb714e3bac64f9acc5989de5650`.
The final suite source `44cef27d7c943753004f3129e2d8e998e7da21c3` differs only
by a test initializer correction; product code is identical. This report adds
no source implementation or new whole-workspace run.

## Five acceptance criteria

| Criterion | Evidence and boundary |
| --- | --- |
| 1. Mechanical belongs in Model; Motion is separate and Sketch contextual | The [registry](../../crates/multicad-ui/src/registry.rs) retains Project, Model, Motion, Drawings and Export. Mechanical contains Thread, Gear and mechanical-part actions. Existing Weld tools remain; Sketch editing is contextual in Model. The approved mode boundary supersedes the original foundation layout. |
| 2. Generator/Library discovery and configurable visibility/order | The current native repeat opens all three forms, finds the shared Library from Mechanical and Wood, hides/reorders Mechanical, confirms Common fallback and finds Mechanical commands while hidden. Restart retains the settings; exact original preference bytes are restored afterward. |
| 3. Shared canonical source and UI/MCP lifecycle | The [thread aggregate](thread-generator-2026-11-04.md) and [gear aggregate](gears-complete-2026-11-06.md) supply native generators, typed preparation, Preview/Cancel/Apply, history, persistence and manufacturing evidence. Those complete workflows are separate from this navigation-only repeat. |
| 4. Existing assembly integration and reusable parts across workbenches | Assembly creation uses the model tree, placement uses global Library, and pose/relationships/interference use Motion. The [Library placement record](library-placement-2026-11-05.md) covers actual Mechanical/Wood placement, pinned source, atomic history, private ownership, offline reopen and native leaves. Gear pairs reuse ordinary Assembly/Instance/Fixed relationships. |
| 5. Native discovery/settings persistence and explicit extension scope | The current stamped macOS repeat verifies discovery and settings across a fully closed restart without applying geometry. The historical foundation remains attributed separately. Purchased catalog completion, additional connection modes and proposed mechanism extensions are not declared complete by the workbench. |

The source regression tests cover registry routing, mode boundaries, hidden
workbench search, preference migration and persistence. The current integration
retains one shared Library and canonical document; it introduces neither a
second parts catalog nor a separate assembly model.

## Current actual macOS discovery and settings

The [interaction manifest](../../artifacts/mechanical-complete-20261106/screens/manifest.jsonl)
attributes the native captures to the package above. Model/Mechanical opens the
Thread, Gear and hardware forms. Each was cancelled at revision 0. Global
Library opens from Mechanical and shows the M3 filter results. Opening the same
browser from Wood confirms the shared route; no part was placed or applied.
The final project remains at revision 0.

Settings moved Mechanical one position earlier, before Relationships, and hid
it. The active workbench fell back to Common. The first search attempt did not
focus the finder input and instead opened Extrude through its shortcut; Cancel
left revision 0. It is not counted as successful search. The confirmed
`17-hidden-mechanical-search-confirmed` capture records the later finder result
while Mechanical was hidden. Weld and Motion retain their existing tools in
captures 18 and 19.

The application was fully closed before restart, recorded in the
[restart inventory](../../artifacts/mechanical-complete-20261106/settings-restart-closed-apps.json).
The restarted Settings retained Mechanical hidden and before Relationships.
The four preference snapshots preserve the actual states:
[before](../../artifacts/mechanical-complete-20261106/preferences-before.json),
[hidden](../../artifacts/mechanical-complete-20261106/preferences-hidden.json),
[after restart](../../artifacts/mechanical-complete-20261106/preferences-after-restart.json)
and [restored](../../artifacts/mechanical-complete-20261106/preferences-restored.json).
The before/restored files are byte-identical; the hidden/after-restart values
retain the intended visibility and order. An initial early read preceded the
asynchronous restore write; the completed read confirms exact restoration.
This verification establishes settings persistence, not application latency.

The [final closure receipt](../../artifacts/mechanical-complete-20261106/final-closure.json)
confirms no running CAD application, byte-exact preference restoration and
project revision 0 with no committed QA geometry.

## Historical and current checks

The original [foundation receipt](../../artifacts/mechanical-review-20261101/review.json)
records source `017e767`, actual native settings/navigation checks and a locked
workspace run with 1975 passes, zero failures and three ignores. It explicitly
marks Thread/Gear integration incomplete at that checkpoint. Its old ribbon
and outstanding-generator statements remain historical; later generator and
Library records supply current functionality.

The [current complete runner](../../artifacts/gear-coverage-20261106/checks/expanded-workspace/workspace-summary.json)
records 2732 passed, zero failed, four intentional ignores across 315 result
groups, exit 0, on the exact final suite source. The command was
`cargo test --workspace --locked --no-fail-fast -- --test-threads=4`, with
optimization 3, debug assertions and overflow checks enabled.
[Formatting and strict workspace all-target Clippy](../../artifacts/gear-coverage-20261106/checks/expanded/gate-corrections.json)
pass separately. Current native OS interaction is distinct from offscreen
painted tests and packaged read-only protocol checks.

[CAD-200](https://plans.tokenet.de/CAD/200) retains sourced purchased-part catalog
and filtering/rights scope; [CAD-201](https://plans.tokenet.de/CAD/201) retains
additional graphical connection workflows. Showing the mechanical-part editor
or an M3 Library result does not complete either item. Other proposed mechanism
extensions likewise remain separately scoped. These checks establish no
operator-latency percentile, physical fit, manufacturing certification,
Windows/Linux runtime result or human acceptance.
