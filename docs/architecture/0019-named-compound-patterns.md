# ADR-0019: Keep pattern members inside one parametric Part

| | |
|---|---|
| Status | Accepted engineering decision; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

A parametric pattern changes its member count when a parameter changes. The
canonical document must preserve its authored output identity through those
rebuilds. Allocating canonical Parts during native evaluation would make derived
geometry mutate the document outside its transaction history.

The [Split contract](0014-semantic-solid-partitions.md) gives two authored
membership sides independent Part identities and requires each side to remain
connected. A pattern has an authored member index that a native fragment lacks,
so it can name repeated geometry without relying on fragment order.

## Decision

Persist linear and circular patterns as `model.advanced` features in the source
Part's history. Freeze the preceding producer, retain one output Part identity,
and store a bound integer count with a translation step or rotation axis/step.
The canonical [parameters](../../crates/multicad-core/src/model/advanced/parameters.rs)
bound each pattern to 128 members, including the original at index zero.

Evaluate unmerged members as an exact native compound. Each member receives
provenance under its authored index, preserving the member discriminator when
count or dimensions change. Native topology ordinals and display triangles do
not define persistent references. Missing or ambiguous members require a repair
diagnostic.

An explicit merge option invokes native Boolean union. Its changed boundaries
can invalidate references, which the ordinary reference resolver reports.
Unmerged overlapping members remain separate solids; their existence does not
establish a collision-free or manufacturing-ready assembly.

Keep the [rigid Instance contract](0017-part-and-instance-placement.md) separate.
An Instance of a patterned Part places its complete compound, while linked
Instances follow changes to that Part definition. Reflection remains a distinct
geometry operation; it does not permit negative factors in
[Scale](0018-exact-scale-owned-components.md).

This extends the supported producer types. It preserves Split's connected-output
guarantee and its explicit rejection of unidentified disconnected fragments.

## Consequences

Positive: count changes preserve the authored Part and producer identities.
The existing transaction, dependency, native preview and export boundaries can
consume the resulting exact geometry without a second collection of model data.

Negative: members share the Part's material, tags and placement. A member does
not have independent assembly motion or an independently editable Part record.
Users who need those properties must create separate Parts or placed Instances.

Neutral: consumers must inspect the native solid count rather than assume that
every Part contains one connected solid. The
[native compound adapter](../../crates/multicad-kernel/src/advanced_operations.rs)
preserves member geometry without silently fusing it.

## Alternatives considered

**Create a canonical Part and Instance for every member.** This makes individual
placement and metadata convenient. Parameter-driven count changes would also need
to add and remove document entities transactionally, so it requires a separate
authored assembly-generator contract.

**Keep repeated members only in the renderer.** This minimizes geometry work and
is useful for display-only repetition. Manufacturing and reference consumers
would lack the exact solids, so it cannot implement a modeling pattern.

**Always fuse repeated geometry.** This yields one connected result when members
touch appropriately. Disconnected members would require different behavior, and
fusion would remove boundaries the user may need, so merging remains explicit.

## References

- [Canonical requests](../../crates/multicad-core/src/model/advanced/request.rs)
  allocate output identities before preparation.
- [Atomic commands](../../crates/multicad-core/src/model/advanced/prepare.rs)
  retain Part history and source ownership through create/edit.
- [Native fixtures](../../crates/multicad-kernel/tests/advanced_solids.rs)
  check compound solid counts, overlapping members and STEP readback.
