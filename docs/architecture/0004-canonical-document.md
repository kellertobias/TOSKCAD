# Canonical document and module ownership

CAD-51 establishes `multicad_core::document`. It stores editable source definitions;
it does not evaluate solids, solve sketches or implement the native project format.

## Identity and references

`Document` owns a private map of `EntityId` to `Entity`. Each document and entity
has a persistent UUID, independent of names, insertion order and feature positions.
Renaming or reordering existing entities retains their IDs. A copy operation must
allocate new IDs and remap internal links; cloning Rust values alone is not a copy
command. Instances reference a shared part or assembly and add a placement, rather
than duplicating its definition. Two parts can reference separate features that
consume the same sketch, and carry independent materials and tags.

`EntityData` distinguishes parameters, planes, sketches, features, parts, assemblies,
instances, materials, drawings, export sets and extensions. Part feature order is
explicit; evaluation dependencies are explicit links, not inferred from array order.
A feature consuming an earlier result should reference the producing feature,
not its enclosing final part, which would create a dependency cycle. Persistent
face/edge provenance and repair policies remain CAD-55 work.

`Link` supplies a target UUID and a semantic role, so a broken connection identifies
the owner, missing target and purpose. The graph deduplicates edges for traversal
without discarding the original roles used by diagnostics. Every external source
dependency must be declared as a link, including dependencies used inside a
module's payload. Display names must never act as persistent reference keys.

## Validation and invalidation

`insert` and `replace` reject structural errors and cycles and restore the previous
state on failure. These are single-record operations, not command transactions,
Undo/Redo or concurrent edit protocols (CAD-53). Document fields cannot be mutated
through a borrowed entity; callers edit a value and submit it through the API.

`validate` checks identities, schema, source field invariants and acyclicity.
Missing targets and wrong target kinds remain loadable for repair and are returned
by `reference_diagnostics`. Successful structural validation does not mean a model
is evaluable or a solid is valid. An evaluator must reject affected outputs with
unresolved references, unknown providers or invalid provider-specific definitions.

`topological_order` returns sources before dependents, using UUID order to break
ties. Cycle errors contain an actual closed dependency path. `affected` returns
only the requested sources and their transitive dependents that still exist, in
that evaluation order. Removing a source retains its dependent links and allows
`affected([deleted_id])` to identify the surviving dependents. Callers must also
discard the removed entity's own cached results. These methods reconstruct the
graph using ordered maps; incremental indexing and large-model performance are
not yet validated.

## Extending the model

`Definition` carries a stable provider `type_id`, positive schema version and JSON
source payload. Providers for cabinetry joints, bends, welds, drawings or export
recipes own their schemas and typed decoding. Their definitions declare entity
dependencies alongside the payload, allowing the core graph to remain independent
of those domains. The core preserves unavailable provider payloads for recovery;
it does not pretend to validate or execute their semantics. Provider registration,
migration, evaluation and command integration are subsequent work.

Definitions must contain editable source only. Native pointers, solver handles,
display meshes and transient widget state are forbidden by the ownership contract.
Arbitrary JSON is not a security sandbox; provider validation is still necessary.
Parameter expressions and units currently store source text and explicit bindings;
typed units, expression parsing and evaluation belong to CAD-52. A binding role
can name an expression variable while its target remains a stable entity UUID.

## Consumer boundaries

| Consumer | Owns | Relationship to the document |
| --- | --- | --- |
| Core | Source entities, IDs, structural validation and graph | Canonical authority; has no geometry, solver or UI dependency |
| Command host (CAD-53) | Revisions, transactions, previews and undo history | Sole application mutation path for UI and MCP; wraps the low-level document API |
| Geometry/evaluation | Native shape ownership and evaluation caches | Reads feature definitions and resolved inputs; caches by entity/revision; native handles never enter source records |
| Sketch solver | Constraint translation and transient solve state | Reads sketch/parameter source; returns results or diagnostics for command-controlled changes |
| Persistence (CAD-56) | Container, atomic writes, migrations and recovery | Serializes the canonical source plus separately identified payloads; validates before replacing a live document |
| Viewport | Retained display meshes, picking and selection state | Renders derived results; never becomes manufacturing geometry or a second editable model |
| UI and MCP | Interaction and attributable requests | Query the same document and submit the same typed commands; no workspace-owned document copies |

The native host uses this source model through shared parameter and transaction
APIs. The [native editing contract](0005-native-editing.md) describes its current
UI and source snapshot wiring, plus the remaining evaluation and persistence limits.
The foundation laboratory's private proof document remains separate.

## Serialization evidence and limits

The Serde boundary rejects unsupported document versions, unknown structural
fields, duplicate entity keys, invalid identity mappings and cycles. It preserves
repairable links and versioned module data. JSON float round-trip parsing is
enabled and tested for exact finite coordinate bits, including negative zero.
This prevents serialization drift; it does not remove floating-point geometry
tolerances or establish robust Boolean operations.

Tests exercise a mixed-material assembly with independent parts from a shared
sketch, drawing/export definitions, real temporary-file DTO round trips, retained
identities after unrelated edits/reordering, deterministic dependencies, rollback,
deletion/repair, unavailable modules and corrupted data. The JSON DTO is not a
native project file. Source snapshot atomic writes are described in
[0005](0005-native-editing.md); container recovery, migrations and manufacturing
payload persistence still require CAD-56.
