# ADR-0021: Transfer source features through explicit occurrence paths

| | |
|---|---|
| Status | Accepted engineering design; implementation verification and workflow acceptance remain separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

Hardware holes and cutout boundaries must drive receiving geometry while the
source hardware stays independently editable. A reused assembly can contain both
the source and receiving Part. Depending on that assembly's complete geometry
would make a receiving cut depend on its own result.

The [physical leaf contract](0020-directed-assemblies-and-physical-leaves.md)
distinguishes complete occurrence paths from reusable Part definitions. A drill
feature also needs a stable preceding receiving solid and current source
geometry. World-space display coordinates cannot identify either dependency.

## Decision

Store one `model.transfer_group` extension with the source, ordered receiving
rows, and sole ownership of their geometry captures. Each row has its own stable
identity and one `model.transfer` output feature in the receiving Part's history.
Freeze that row's preceding `BodyInput`. Group edits preserve the receiving
definitions, input producers and output identities, and commit atomically.

Bind source captures to the source Part alias so they follow its current head.
Bind the receiving entry face to its frozen preceding producer. Persist complete
source and target occurrence paths separately from these component-local
captures. Cuts edit Part definitions, so every placed copy receives the same
local result; show that consequence in the Inspector.

Support three explicit source selections: complete circular edges or cylindrical
faces, all circular inner loops of one planar face, and one planar face's outer
boundary. A valid face with no inner loops produces no cuts in the second mode.
A missing explicit circle remains a failure. Reject unsupported inner-loop
geometry rather than silently omitting it. Store each receiver's diameter
override, radial clearance, blind or through depth, and cut direction.

Cancel the common canonical occurrence prefix before computing relative poses.
Apply the remaining source-to-receiver transform exactly once. Keep authored
pose links for structural diagnostics and reverse invalidation. For geometry
evaluation order, expand the current diverging pose drivers and their geometric
anchor suppliers instead of requesting aggregate assembly geometry. A genuine
dependency on the receiving result is still a cycle. Include the current pose
inputs in dependency fingerprints; never freeze their suppliers at creation.

Keep file readers independent of live geometric resolution. Missing paths or
source geometry must remain saveable and repairable, while native evaluation
reports their failure. Preserve semantic capture identities during explicit
repair. Never substitute a nearby face, hole, display triangle or topology
ordinal.

Limit initial projection to parallel or antiparallel planes and axes. The native
kernel receives exact target-local profiles. Zero clearance preserves supported
line, arc and circle boundaries; nonzero offsets are bounded to audited circles
and strictly convex line outlines. Through cuts use finite receiving bounds.
Reject wrong directions, empty removals, complete removal, disconnected results
and unsupported offsets explicitly.

Use the same typed preparation and transaction API from the UI and external
control. A current, scoped native source-inspection receipt can authorize a
replacement capture even when full model compilation fails. It cannot authorize
Apply or manufacturing output. Apply requires a complete native evaluation of
the exact candidate document, followed by one guarded transaction.

## Consequences

**Positive:** Receiver edits remain associative through supported pitch, pattern
and placement changes. Shared assembly transforms cancel before geometry work,
and source-only inspection keeps broken transfers repairable.

**Negative:** The dependency graph needs separate structural and evaluation
semantics. New pose drivers must declare their geometric suppliers correctly.
Definition-level cuts also cannot represent different machining on two copies
of one Part without first giving them independent definitions.

**Neutral:** Projection support is deliberately bounded. Curved entry surfaces,
tilted drilling and general topology-changing offsets require later contracts.
The typed API does not establish connected MCP transport or manufacturing
acceptance.

## Alternatives considered

- **Copy world-space centers into independent holes.** Simple to implement and
  useful for disconnected edits, but loses association and provenance when
  hardware placement or patterns change.
- **Depend on the complete placed assembly.** Reuses existing aggregate pose
  evaluation, but introduces a false cycle when the receiver belongs to it and
  performs unnecessary geometry work for shared transforms.
- **Machine an individual occurrence without changing its definition.** Allows
  copy-specific holes, but needs an explicit derived-component ownership model.
  It must not silently alter the meaning of existing linked Instances.

## References

- [Canonical transfer requests](../../crates/multicad-core/src/model/transfer/parameters.rs)
- [Exact native cuts](../../crates/multicad-kernel/src/transferred_cuts.rs)
- [Persistent geometry references](geometry-references.md)
