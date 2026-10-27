# ADR-0010: Separate component ownership from feature dependencies

| | |
|---|---|
| Status | Accepted engineering decision; native hierarchy acceptance remains separate |
| Date | 2026-10-27 |
| Decider | Codex, using the delegated architecture authority |

## Context

A component owns model entries and establishes the default body-cut boundary.
An organizational group preserves that boundary while arranging entries. A sketch
can reference another source in the same component or elsewhere without ownership
creating a feature-history cycle.

Existing documents represent reusable assemblies with `Assembly.instances` and
placed references with `Instance.component`. The same legacy Instance can occur in
several assembly lists. Loading such a document must preserve identities and
placements without silently selecting one parent or duplicating source.

## Decision

Keep `Assembly { instances }` unchanged as the component/assembly definition
identity. Add a distinct Group entity marker. Store ordered model containment,
source visibility and the active component in typed `Document.hierarchy` state.
The document UUID denotes the root container; an absent active component denotes
root modeling scope. Default hierarchy state is omitted from serialization, so
legacy source roundtrips retain their original shape.

New membership is not an entity reference or evaluation prerequisite. Validate
containment separately from the feature dependency graph. Additionally validate
an expansion graph containing new membership, legacy assembly-instance membership
and `Instance.component` edges. This catches recursive nested instantiation even
when neither containment nor the feature graph alone contains a cycle.

Containment ancestors participate in invalidation reachability, allowing a member
change to invalidate consumers of its component/group. These edges do not enter
feature topological ordering. Geometry and drawing evaluators still own their
aggregate evaluation strategy; organizational ownership is not a new geometry
producer.

A Part remains an independent canonical body definition. An Instance references
that definition with a placement; placing it elsewhere does not transfer the
source Part's ownership. Core queries return source ownership and inherited source
visibility. Renderers add occurrence placement and visibility. Cut controllers
also require current valid geometry rather than treating a source query as proof
of a valid solid.

A group supplies no component boundary. A body's nearest containing Assembly owns
it; an unowned body belongs to root. Nested and sibling components retain separate
boundaries. Ancestor visibility excludes hidden bodies from the corresponding
source-body cut candidates. Expanding and collapsing the UI tree stays local UI
state, avoiding semantic source transactions for navigation.

Keep shared legacy instance membership loadable with explicit diagnostics.
Ownership queries on ambiguous entries return an error rather than choosing the
first UUID. An explicit Move removes the selected entry from every legacy parent
and assigns one ordered destination in the same transaction. Normalizing an entire
legacy component requires unshared, valid instance references and preserves their
order. It never allocates replacement body or instance identities.

Provide typed hierarchy commands through the existing prepared transaction API.
`Session::prepare_scoped` adds newly inserted model entries to the active component
in that same candidate. Global parameters/materials and existing linked sources
stay put. Cancellation changes no source; commit, Undo and Redo preserve one shared
history for UI and external callers. Source creation factories opt into this seam.

Native projects derive `core.hierarchy.1` from non-default hierarchy state or Group
entities. Unsupported readers reject that capability before interpreting source.
The core cache interpretation fingerprint includes hierarchy semantics. Canonical
source hashes also include the persisted hierarchy state.

## Consequences

Existing Assembly constructors and legacy JSON documents remain compatible.
Ownership queries are independent of geometric feature history, while invalidation
still reaches component consumers. Typed state gives validation and persistence a
single source of truth without embedding GUI widgets or native geometry handles.

The document gains metadata, and raw mutation clients must update membership in
the same transaction when removing a member or container. Legacy shared parents
need explicit normalization before unambiguous ownership or ordering. Whole-source
cache keys conservatively invalidate after hierarchy changes. Fine-grained geometry
cache reuse and repeated-instance occurrence UI remain evaluator/UI responsibilities.

## Alternatives considered

**Add ordered members to the Assembly variant:** this colocates assembly fields,
but changes every existing Rust constructor and does not by itself provide root
ordering, visibility or active scope. Typed document state preserves that public
representation while supplying the shared hierarchy rules.

**Use membership as ordinary dependency links:** this reuses graph traversal,
but makes ownership an evaluation prerequisite and can introduce false cycles
when contained sketches/features reference their context. Separate validation and
invalidation graphs preserve both meanings.

**Store hierarchy as an opaque extension definition:** this avoids changing the
canonical document shape, but makes core ownership and cut-scope validation depend
on decoding optional provider data. A typed optional field makes those model-wide
rules explicit; unavailable feature definitions remain independently preserved.

## References

- [Hierarchy state and validation](../../crates/multicad-core/src/hierarchy/mod.rs)
- [Ownership, visibility and cut-scope queries](../../crates/multicad-core/src/hierarchy/index.rs)
- [Prepared hierarchy commands](../../crates/multicad-core/src/hierarchy/operations.rs)
- [Canonical mutation and validation boundary](../../crates/multicad-core/src/document/mod.rs)
- [Behavior and compatibility tests](../../crates/multicad-core/tests/hierarchy.rs)
- [Native project capability/cache contract](project-files.md)
