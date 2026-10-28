# ADR-0016: Convert extrusion operation kinds without rebinding producers

| | |
|---|---|
| Status | Accepted engineering decision; additive to ADR-0015; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

A captured Boolean extrusion operation can return to New body mode while
preserving its extrusion producers. New body mode has no Boolean results, which
the schema 1 association cannot represent. Independent extrusions in older files
also need an explicit conversion path that preserves their existing identities.

The association and removal rules in [ADR-0015](0015-captured-extrusion-operations.md)
continue to apply. Operation conversion must preserve source dependencies and
leave unaffected extrusions unchanged, including siblings that share a sketch.

## Decision

Add schema 2 of `model.extrude_group`, with capability `model.extrude_group.2`.
Persist `ExtrudeGroupKind::Boolean` or `ExtrudeGroupKind::NewBody` explicitly.
Boolean mode requires result members and derives its operation and keep-tools
setting from their definitions. New body mode requires no Boolean results.
Its edit request must contain no targets and set keep-tools to false.

Keep the schema 1 wire representation unchanged. Decode it as a Boolean group,
including its existing nonempty-result requirement. Ordinary schema 1 edits
retain schema 1. New body operation groups require schema 2. A schema 2 group
remains at schema 2 after conversion back to Boolean mode.

The `Definition` codec retains decoded schema provenance outside the serialized
payload to prevent an implicit downgrade during read/write. An empty schema 1
record remains invalid; it never becomes an inferred New body operation.
Extrusion and Boolean geometry provider schemas remain unchanged.

Create associated New body outputs through `GroupedExtrudeRequest` and
`prepare_grouped_extrude`. The draft operation ID and output IDs are allocated
once and reused across previews. The underlying `prepare_extrude` API remains
available for legacy callers and internal Boolean tool creation.

Use `prepare_extrude_group_edit` for conversion of an existing group. Switching
to New body removes all owned Boolean results using the removal guards in
ADR-0015 and restores the target Parts' preceding histories. The group identity
and extrusion Part/producer identities remain unchanged. A blocked removal
rejects the complete candidate without deleting another result first.

Changing an applied New body group to Boolean mode requires explicit
new target bindings and newly allocated Boolean identities. No removed Boolean
identity is inferred or silently resurrected. Captured target selection remains
explicit; automatic rediscovery retains the separate query contract described
in ADR-0015.

Adopt legacy source through `inspect_ungrouped_extrude`,
`AdoptExtrudeGroupRequest` and `prepare_adopt_extrude_group`. The request pins
exactly one selected extrusion producer and its Part. It can identify a historical
producer explicitly; adoption preserves that version rather than substituting
the current head. Existing group membership prevents adoption into another group.

Adoption and the requested conversion form one original-Session transaction.
Preserve the selected producer's profile, Part and Feature identities, names and
history. Matching sketches or parameters never authorize adopting siblings.
Opening an editor or allocating an adoption draft changes no canonical source.
When a legacy edit finishes in New body mode, parameter and face edits continue
through the legacy preparation path without persisting a group.

`ExtrudeGroupState.operation()` and `keep_tools()` derive valid settings for
both kinds, including zero Boolean members. New body preparations report the
owned extrusion producers as outputs; Boolean preparations report result
producers. Native candidate verification still checks final body membership and
producer-version consumption before Apply. The native geometry compiler needs
no new provider for the association kind.

## Consequences

Operation changes preserve the producer identities needed by downstream source.
New body creations retain a durable association for later edits, while legacy
parameter-only edits preserve their ungrouped representation. The same typed
transaction path supports preview cancellation and Undo/Redo.

The cost is another association schema and compatibility codec. Readers must
support the schema 2 capability before editing its source. Conversion can fail
when an owned result has later history or external references; callers must
resolve those dependencies rather than cascade-delete them.

An explicitly selected historical producer can differ from its Part's final
head. Controllers must verify producer retention separately from current body
membership. Tool consumption remains version-scoped, including when another
operation uses the same Part.

## Alternatives considered

**Permit empty results in schema 1.** This avoids a capability addition, but
changes an existing validity rule and leaves older readers unable to distinguish
New body source from malformed Boolean membership.

**Recreate extrusion Parts when switching operation.** This reuses creation
without preserving producer versions. It changes persistent identities and can
invalidate source that refers to the existing extrusion.

**Adopt every extrusion sharing the selected sketch.** This offers a convenient
batch edit. Independent operations can share a sketch, so the selection cannot
authorize changes to those other producers.

## References

- [Kind, state and typed request APIs](../../crates/multicad-core/src/model/extrude_group.rs)
- [Versioned association codec](../../crates/multicad-core/src/model/extrude_group/codec.rs)
- [Grouped New body creation](../../crates/multicad-core/src/model/extrude_group/creation.rs)
- [Explicit single-producer adoption](../../crates/multicad-core/src/model/extrude_group/adoption.rs)
- [Atomic conversion and removal guards](../../crates/multicad-core/src/model/extrude_group/prepare.rs)
- [Core group tests](../../crates/multicad-core/tests/model_extrude_group.rs), including
  [conversion and ordering regressions](../../crates/multicad-core/tests/model_extrude_group/conversion.rs)
- [Native conversion and adoption tests](../../crates/multicad-model/tests/extrude_group_conversion.rs)
- [Captured association decision](0015-captured-extrusion-operations.md)

Evidence from committed sources `1d17cc1` and `6666926`: the following commands
report 18 passing core group tests and two passing native conversion/adoption tests.

```sh
cargo test -p multicad-core --test model_extrude_group --locked --offline
cargo test -p multicad-model --test extrude_group_conversion --locked --offline
```
