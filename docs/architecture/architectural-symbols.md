# Project-authored wall and fence symbols

A dynamic architectural symbol captures an authored construction and its selected
public inputs. Drawing an instance supplies its own path, level and base context.
Wall layers retain their physical thickness; fence posts and infill regenerate
from the receiving run. The existing Library stores and lists these definitions.

The ownership decision is recorded in [ADR-0029](0029-path-driven-architectural-symbols.md).
Implementation and test evidence are separate from actual macOS interaction and
human acceptance.

## Author and register a construction

Create a layered wall or fence construction with its graphical construction
editor. Create symbol accepts that canonical construction, or derives it from a
selected unpinned wall body or fence member. The source picker also lists eligible
constructions. A sample wall path, fence run and generated members are excluded
from the captured definition.

The interface editor selects the public dimensions, material choices and options.
Numeric parameters used by the construction have a separate explicit exposure
list. Unchecked settings remain private. Native Preview and Apply publish the
interface and embedded definition together in one undoable transaction.

Save the authored project through the existing file workflow. File metadata owns
the name, description, preview and Add to Library action. There is no separate
architectural catalog. Open that Library file in another project and explicitly
embed the selected definition. Its Draw Wall or Draw Fence action opens the
compatible drawing tool.

The live source construction is the editable draft. Its published definition is
an immutable snapshot of the captured dependency closure. After changing a source
construction, publish a new revision of the same symbol. Existing consuming pins
retain their previous content until an explicit update.

An exposed slot must keep a valid target in the live source. Remove its exposure
before deleting or changing that target's construction family. The source
transaction refuses an interface that points at a missing layer or recipe.

## Shared source contract

The existing [SymbolDefinition](../../crates/multicad-core/src/symbols/types.rs)
contains the construction-only template. Its root is the typed
`symbols.architecture_blueprint` extension with capability
`symbols.architecture.1`. It links one canonical construction and declares its
role and interface. No second construction representation is stored in the pin.

| Input | Public contract |
| --- | --- |
| Numeric parameter | Exposed source parameter ID, finite numeric value and compatible unit |
| Dimension | Stable slot ID targeting a supported height, layer thickness, post dimension, spacing or infill dimension |
| Material | Stable slot ID, authored preset material ID or explicitly allowed receiving-project material ID |
| Option | Stable slot ID and one authored typed choice: wall reference, fence layout or purchased-panel short-bay policy |
| Receiving context | Compatible role, path, closed state, level and base; wall flip or fence post positions |

Dimension formulas supplied to instances bind receiving-project parameters by
identity. Source formulas retain their private remapped parameter identities.
Authored material presets use source IDs; project materials use receiving IDs.
Unknown, private, removed, incompatible or disallowed inputs fail preparation.

The providers currently accept layered wall schema 2 and fence construction
schema 1. Uniform wall types, arbitrary extension providers and role conversions
are refused. Ordinary Part/Assembly symbol placement cannot place a construction
blueprint without receiving architectural context.

[Atomic authoring](../../crates/multicad-core/src/symbols/architecture/author.rs),
[placement](../../crates/multicad-core/src/symbols/architecture/placement.rs) and
[editing](../../crates/multicad-core/src/symbols/architecture/edit.rs) all prepare
shared Session transactions. Preparation validates source; native adapters must
compile the complete candidate before committing or enabling Apply.

## Placed walls and fences

A `symbols.architecture_instance` record owns its embedded definition pin, source
identity map, public overrides and output anchors. The actual wall feature or
fence run owns the receiving path and context. The instance record does not store
a competing path or context snapshot.

A placed wall is a canonical wall feature and body. Its opening types, openings,
cutouts and supported junctions use the existing [wall contracts](walls.md).
Layer materials remain available to native section hatches and equipment
attachment. Private construction values are hidden in receiving inspectors;
public settings and supported receiving context remain editable.

A placed fence is a canonical run and group containing named physical members.
[The fence rules](fences.md) recompute its posts, bays and infill. Retained
semantic members retain their feature/body IDs. Removed or changed physical
families require valid reference handling. Parameter-driven changes reconcile
members in the same Session transaction before native evaluation.

The graphical fields and fence source-layout drafts are cached by source/form
state. Idle frames do not repeatedly parse captured templates. Source layout and
post-plan queries remain distinct from exact native preview receipts.

## Explicit revision changes

Review definition update compares a selected replacement pin against every affected
instance. Its report includes public interface changes, removed named wall
layers, changed fence recipes, removed physical members and unresolved repairs.
Compatible private and output identities and compatible public overrides survive.

Removed inputs require explicit discard or repair. A surviving slot whose target
changed requires an explicit reset; its old value cannot silently acquire a new
meaning. Material and option overrides must remain among the new allowed choices.
Role conversion requires a new symbol identity.

Input repairs and receiving context repairs can be composed with the pin update.
Core wall context also supports atomic opening/cutout/junction replacement and
new canonical opening types. Unspecified wall details remain unchanged. Native
Preview evaluates the complete replacement candidate. Cancel leaves the document
unchanged; Apply commits one transaction, and Undo restores the previous pin,
private source and complete receiving output state.

An embedded pin remains sufficient after the authoring file or Library disappears.
Opening a newer Library file never updates consuming projects automatically.

## MCP control

[Typed adapters](../../crates/multicad-mcp/src/tools/architecture/mod.rs) call the
same core preparation functions as the graphical tools. Their schemas are
published by MCP discovery. Lengths use the existing unit-bearing `Quantity`
shape: exactly one finite `value` or `expression`, plus `unit`.

| Tool | Mutation | Contract |
| --- | --- | --- |
| `inspect_architectural_symbol` | No | Omit `entity` to list definitions; otherwise inspect a definition, record or output. Returns public defaults and, for an instance, current context and overrides |
| `author_architectural_symbol` | Yes | Select a live canonical construction and explicit public slots/parameters. Retain symbol, blueprint and slot IDs for later authored revisions |
| `import_architectural_symbol` | Yes | Embed one architectural definition from a `.tcd` Library file in the configured exchange folder |
| `draw_architectural_symbol` | Yes | Supply an embedded definition, complete public overrides and compatible receiving context |
| `edit_architectural_symbol` | Yes | Replace public overrides and optionally context for a record or output |
| `compare_architectural_symbol_update` | No | Compare a Library revision and optional per-record repairs; returns source conflicts and affected outputs without committing |
| `update_architectural_symbol` | Yes | Apply the explicit replacement and repairs after full native preflight |

Every mutation requires `expected_revision`. A stale revision, cancellation,
failed preparation or failed native preflight leaves the document unchanged.
Read-only server mode rejects mutations. Normal owner review policy still
applies to a prepared mutation.

An empty override map means inherit the pin. Editing `inputs` replaces all four
public maps; omitted map entries inherit, rather than retaining earlier overrides.
Omit `context` to retain the current path and all wall details. When supplying a
fence context, retain the returned IDs for retained corners and provide the full
post-position list. Wall context edits preserve openings, cutouts and junctions;
existing wall tools edit those details separately.

Update `inputs` and `contexts` are keyed by architectural record ID and repair
only the named affected records. Other records retain their existing overrides
and context. `update_architectural_symbol` takes `expected_revision` plus an
`update` object shaped like the comparison request.

Library `source` contains a bare `file_name` and optional `source_definition`.
Omit the latter only when the file contains exactly one architectural definition.
Traversal and paths resolving outside the configured exchange folder are refused;
normal project-file limits and capabilities apply.

Inspection and comparison report `native_geometry_checked: false`. Authoring or
importing also report false for the new construction, because it has no receiving
path. Successful mutations report `native_model_preflight: true` for the complete
candidate. Drawing, editing and updates with affected instances report true for
`native_geometry_checked`. These fields do not establish manufacturing export
validity or actual macOS interaction.

## Verification boundary

[Core tests](../../crates/multicad-core/src/symbols/architecture/tests.rs),
[native tests](../../crates/multicad-model/tests/architectural_symbols.rs),
[UI tests](../../crates/multicad-ui/src/symbols/tests.rs) and
[MCP tests](../../crates/multicad-mcp/tests/architectural_symbols.rs) exercise
transaction/history behavior, native role geometry, offline pins, interface
controls and external control. Native coverage includes wall openings, joined
faces, material section hatches and attached equipment, plus fence regeneration.

Painted egui interactions and native compiler tests remain separate from actual
macOS interaction. The [2026-11-04 verification](../verification/architectural-symbols-2026-11-04.md)
records recovered wall/fence authoring, Library placement and explicit updates,
receiving details and drawing output, with scoped evidence and repository checks.
Its unavailable-source check covers the registered local author file, not
network isolation. Existing wall and fence provider limits continue to apply,
including supported wall junctions and bounded flat fence paths. Human acceptance
remains separate.
