# ADR-0013: Evaluate exact model queries with their source snapshot

| | |
|---|---|
| Status | Accepted engineering decision; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

An extrusion cut needs the bodies it actually intersects before a transaction
can specify its targets. Visibility and component ownership determine the default
pool; explicit inclusions and exclusions can change that pool without changing
the document. Display bounds can reject distant candidates but cannot prove a
volumetric intersection. Native B-reps remain owned by the model worker.

## Decision

Send bounded, typed query options alongside an evaluation request. Keep the
portable request and receipt definitions in
[`core::model`](../../crates/multicad-core/src/model/evaluation.rs). Execute each
query against the exact model compiled for that request, on the same owner
thread, before publishing immutable display data.

For cut targets, use cached bounds as a broad phase and native solid intersection
as the final test. Include only positive-volume intersections. Derive defaults
from visible bodies directly owned by the selected component, including its
organizational groups. Explicit body or component inclusions and body exclusions
remain stable source identities. A query failure produces a diagnostic receipt
without discarding an otherwise valid model scene.

Bind each publication to the document identity, revision and canonical source
hash. Also retain the runtime's private request generation: two requests can use
the same document, revision and public display stamp while carrying different
query options. An obsolete query must not overwrite the latest result. Query
correlation IDs identify receipts within a request; they do not prove freshness.

Attach immutable evaluation metadata to the production scene. Visibility
filtering preserves the same metadata and stamp. A retained stale scene keeps
its original metadata; an isolated upstream repair view omits it. UI transaction
controllers still own source, session, form-generation and Apply validation.
Structural receipt validation alone never authorizes a commit.

## Consequences

The UI and external command callers can reuse exact queries without owning native
handles. Display publication and query receipts describe one evaluated source.
Legacy display providers continue to accept requests without probes and return
an explicit unsupported error for probe requests.

The cost is an additional exact intersection per broad-phase candidate and a
full model evaluation per request. Work is bounded and superseded at checked
boundaries, but an active native call cannot be interrupted internally. The model
worker owns this limitation; low-power performance remains unverified.

## Alternatives considered

**Query the display mesh.** This would reuse existing picking data and avoid
native work. Mesh approximation cannot establish manufacturing intersections or
distinguish all contacts from positive-volume overlaps, so it cannot select
canonical cut targets.

**Expose a synchronous query on retained native models.** This could avoid a
rebuild. It would require a separately specified cache and source/options
freshness protocol, while a UI call could block on geometry. The owned worker
request keeps those boundaries explicit with the current evaluator.

## References

- [Owned model evaluation](0012-native-model-evaluation.md)
- [Exact target query](../../crates/multicad-model/src/targets.rs) and
  [tests](../../crates/multicad-model/tests/targets.rs)
- [Runtime](../../crates/multicad-model/src/runtime.rs) and
  [tests](../../crates/multicad-model/tests/runtime.rs)
- [Production scene adapter](../../crates/multicad-app/src/modeling.rs)
