# ADR-0015: Persist captured extrusion operations as typed associations

| | |
|---|---|
| Status | Accepted engineering decision; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

An extrusion that modifies several target Parts has extrusion tool producers
and a Boolean producer for each target. Reopening the operation requires its
original member identities and affected-body selection. Independent operations
can share tool geometry, so matching tool inputs cannot establish ownership.

A target can also have later history that depends on an earlier Boolean result.
Changing its affected-body membership must preserve those dependencies or return
an explicit failure. The canonical document already supports typed Extension
records alongside geometry-producing Features.

## Decision

Persist the association in an `EntityData::Extension` record with type
`model.extrude_group`, schema 1 and capability `model.extrude_group.1`.
`ExtrudeGroupDraft` allocates its persistent operation identity once.
`ExtrudeBooleanRequest.group` optionally includes this draft when preparing the
complete creation transaction.

`ExtrudeGroupDefinition` records tool Part/producer bindings and target
Part/Boolean feature bindings. Its links point toward those members; member
Features never link back to the group. `extrude_group_owner` uses explicit
membership to find the operation. The association introduces no geometry
provider or change to native compiler root selection.

Derive common extrusion parameters, termination face inputs, Boolean operation
and keep-tools behavior from the child definitions. Group validation checks
agreement and unique ownership of member Features. Boolean provider schemas
remain unchanged. `read_extrude_group` preserves readable metadata when a member
is missing; `inspect_extrude_group` and grouped editing require the members to
exist.

Treat `captured_scope` as historical query intent. Its component, inclusion and
exclusion identities do not become live model dependencies or an automatic
rediscovery rule. Editing the operation changes its explicit captured target set.
Existing targets retain their Boolean identities and frozen predecessor
producers. Added targets require explicit bindings to current unsuppressed
producer heads and newly allocated Boolean identities.

`prepare_extrude_group_edit` builds all child and membership mutations before
calling the original Session's preparation API once. No intermediate preparation
observes partially changed common settings. The resulting candidate uses the
shared actor, revision and foreign-session guards. Native evaluation still
validates the complete candidate before Apply.

Remove a target's owned Boolean only when it is the final entry in that Part's
history and has no other incoming source references. The group and target Part
links being updated are the expected exceptions. Otherwise return the involved
identities and an actionable error. Preserve the target Part, its preceding
history and metadata, and its organizational parent.

Typed individual extrusion and Boolean edit preparations reject grouped members
and identify their operation. A reference repair that changes one member
inconsistently can also fail group validation. This decision does not provide a
grouped reference-repair interface.

Keep ungrouped source valid through `ExtrudeBooleanRequest.group = None`.
Existing files retain individual-feature editing. No operation association is
inferred from shared tools or reconstructed automatically during opening.

## Consequences

The operation has a stable identity across previews and save/reopen. Atomic
membership edits preserve retained producer identities and participate in the
existing transaction history. Canonical member geometry remains in its provider
definitions.

The cost is a consistency obligation across the children and their association.
Callers must update shared settings together and route typed member edits through
the operation. Missing members need repair before grouped editing can continue.
Dependency checks can reject target removal even when a user wants a smaller
captured set.

Automatic rediscovery during editing requires a separate group-aware query
contract. It must evaluate the stored tool versions and each existing target's
pre-group producer, while new targets use current heads. The current query's
requirement for current, unconsumed tool producers remains unchanged. Editing a
group does not create a temporary document that drops downstream source.

## Alternatives considered

**Infer associations from identical tools.** This avoids additional source
metadata and can suggest candidates in older files. Independent operations can
share tools, so inference cannot authorize an edit or assign persistent ownership.

**Store membership on every Boolean definition.** This makes the association
visible beside each result. It would add grouping metadata to geometry provider
schemas and still require a home for the complete operation association.

**Rediscover targets from final body geometry.** This reuses the current exact
query API. A cut target already contains the cut, and its tool may be consumed;
those inputs cannot reproduce the operation's original selection basis.

## References

- [Typed association, capture and edit contracts](../../crates/multicad-core/src/model/extrude_group.rs)
- [Membership validation and editable inspection](../../crates/multicad-core/src/model/extrude_group/validation.rs)
- [Atomic edits and removal guards](../../crates/multicad-core/src/model/extrude_group/prepare.rs)
- [Optional grouped creation](../../crates/multicad-core/src/model/extrude_boolean.rs)
- [Core behavior and native-file compatibility tests](../../crates/multicad-core/tests/model_extrude_group.rs)
- [Exact native rebuild and persistence tests](../../crates/multicad-model/tests/extrude_group.rs)
- [Document Extension contract](../../crates/multicad-core/src/document/model.rs)
- [Source-bound model queries](0013-source-bound-model-queries.md)
