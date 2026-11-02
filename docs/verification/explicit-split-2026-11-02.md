# Explicit Split verification, 2026-11-02

CAD-222 groups Split inside Boolean and Align inside Move / Copy. Common/Create
keeps Add Primitive, Extrude, Revolve, Sweep and Loft. Primitive popup rows use
the shared icon-and-label control. Fillet/Chamfer and Shell/Thickness remain.
Implementation commits on `main`: `acbd30f` (contract), `5915583` (native and
canonical foundation), `58680bd` (explicit fields and grouped tools).

Actual macOS window interaction and human acceptance remain pending. No new
application windows were opened, installed or published for this item.

## Observable behavior

Both Targets and Splitting tool appear on entry, with counts and an explicit
active field. Compatible selected bodies populate Targets. Tool-first entry
works before native display publication; source choices do not certify geometry.
Enter advances from Targets to the tool, then validates and applies the complete
unchanged request. Clicking either field permits correction. Text entry,
popups, repeated Enter and a pointer-held Cancel do not bypass the native gate.

A cutter is explicitly a closed model or a chosen XY/XZ/YZ or canonical
construction plane. Plane offsets support units and parameter expressions.
Model cutters partition at their actual boundary and retain both inside and
outside results. Plane results retain positive/negative identities relative to
the normal. The form preserves target/tool names and role labels during preview,
and groups output names by target. Cancel, stale inputs, unsupported geometry and
any failed target leave the complete source unchanged. A successful multi-target
operation is one commit and one Undo entry.

## Evidence

| Layer | Checked behavior |
| --- | --- |
| Kernel | Exact half-space partitions, translated/rotated frames, volume conservation, reunion/no overlap, empty/tangent/disconnected rejection, cancellation, cap provenance and retained target ancestry |
| Canonical source | Two targets/four outputs prepared atomically; stable IDs, metadata, input roles, duplicate/invalid planes, stale revision and complete Undo/Redo |
| Native compiler | Closed-model boundary and keep-tool membership; authored and canonical planes; associative parameter changes without exchanging side IDs; one invalid target rejects the complete candidate; project/capability round trip |
| Painted form | Simultaneous fields, preselection, focus clicks, fresh Enter, explicit plane choice, stable names/IDs, stale producer errors, finite inspector layout and Cancel precedence |
| Production editor with native backend | Entry with no display, two selected bodies, plane selection and offset text entry, native failure/Cancel neutrality, four measured 13500 mm³ parts from two 30 mm cubes, one revision and full Undo/Redo |
| Ribbon | Group membership and real primitive popup routing through the shared command registry |

Focused foundation checks passed: kernel Split 12, core Split 8, existing native
model Split 7 and explicit native model Split 4. The final workspace run includes
the final boxed frame and role captions. Its UI library group passed 868 tests
with 0 failures and 2 intentional ignores in 24.27 seconds; all four explicit
native model tests passed again.

Initial restricted kernel compilation could not acquire the native cache lock.
The same pinned native checks passed with local cache access. The first strict
workspace pass found two new source lints; the authored frame was boxed in memory
without changing the serialized payload, and the guard was simplified.

## Final checks

The pinned Rust 1.99.1 toolchain and committed lockfile are used.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, 13m09s |
| `cargo test --workspace --locked` | Passed: 2262 passed, 0 failed, 4 ignored across 255 groups, including doc tests |
| UI library on final source | 868 passed, 0 failed, 2 ignored |
| `./build` | Passed; native development bundle assembled; native compilation 2m18s |
| Built binary `--version` / `--diagnostics` | Both exited successfully |
| Local documentation links and `git diff --check` | Passed |

The complete workspace run used local native-cache and Unix socket access.
Its compilation took 16m46s. The native apartment rebuild regression passed in
452.49 seconds. These are validation timings, not interactive drawing performance
measurements.

Development artifact:
`artifacts/development/build-k9yZIw/MultiCAD.app/Contents/MacOS/multicad`.
Both CLI checks report `ToskCAD 0.1.0-dev`; diagnostics identify statically linked
OCCT 7.9.3 and the compiled Rust/native runtime. The development binary does not
stamp a source commit. These smoke checks do not establish every workflow listed
in the application. No GUI window was opened and no application was installed.

## Boundaries and remaining acceptance

Each semantic output must be one nonempty connected native solid. Open shells
and multiple disconnected fragments on one side receive diagnostics before
mutation; they require a future persistent fragment contract. See
[ADR-0030](../architecture/0030-explicit-split-inputs.md).

Painted pointer tests and native geometry tests do not replace actual macOS
window interaction. Review still needs the real ribbon menus, field switching,
plane and model selection, preview roles, Enter, Cancel and Undo/Redo at normal
and compact window sizes. Windows/Linux runtime behavior is separate evidence.
No dedicated new MCP Split transport was added; canonical preparation and the
native compiler remain the shared typed application path.
