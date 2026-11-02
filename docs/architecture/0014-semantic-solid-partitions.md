# ADR-0014: Identify bounded Split outputs by tool membership

| | |
|---|---|
| Status | Accepted engineering decision; native UI acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

CAD-222 extends explicit inputs and plane/multi-target splitting in
[ADR-0030](0030-explicit-split-inputs.md); this schema 1 membership contract remains.

## Context

Split needs persistent independent Parts that survive dimensional edits.
Native fragment order and size can change during those edits. The current model
provider associates one producing Feature with one connected solid, and later
features can reference that producer's geometry.

## Decision

Support a bounded solid-tool Split with two explicit semantic outputs: the
target material inside the tool and the target material outside the tool.
Allocate each output's Part and Feature identity once when creating the request.
Store the frozen target/tool producer identities and semantic side in each
output definition. Never derive persistent identity from native enumeration,
volume rank or nearest centroid.

Compute exact intersection and subtraction and require both results to be
connected, nonempty solids. Check their summed volume against the original
target using the model tolerance. Reject the pair if either partition is invalid,
including when only one output is being evaluated. Preparation creates both
outputs atomically; the native evaluation gate precedes Apply.

Clone the target's material, tags and organizational parent onto the result
Parts. Retain original source/history and consume the exact target producer
version. Keep tool controls whether that exact tool producer remains an
independent final body. Later producer versions can reactivate their Parts.

Each output depends only on its target/tool producers. It does not depend on its
peer output. Deleting one result therefore does not create a dependency cycle or
make the remaining result lose its semantic identity.

## Consequences

Inside and outside outputs remain identifiable when their relative volumes
reverse. Existing single-producer scene, reference and downstream feature paths
can use each result directly.

The limit is deliberate: either membership side containing multiple disconnected
fragments fails with a diagnostic. General multi-fragment Split needs an
additional persistent selection contract. The initial compiler recomputes the
validated pair for each output, so a normal two-output Split performs four native
Boolean calls. The model layer owns that cost until cache reuse is measured and
specified.

## Alternatives considered

**Number native fragments.** This supports arbitrary partitions with little
extra source metadata. Native order can change under parameter edits, silently
reassigning Part identities and downstream references, so it is unsuitable for
persistent parametric results.

**Use one compound Part for every fragment.** This preserves the entire native
result and avoids naming fragments. It cannot provide independently editable
Parts or the current one-connected-solid producer contract.

## References

- [Canonical Split definitions](../../crates/multicad-core/src/model/split.rs)
- [Native pair validation](../../crates/multicad-kernel/src/split.rs)
- [Model compilation](../../crates/multicad-model/src/split.rs) and
  [integration tests](../../crates/multicad-model/tests/split.rs)
- [Owned model evaluation](0012-native-model-evaluation.md)
