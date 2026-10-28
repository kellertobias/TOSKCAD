# ADR-0017: Separate Part geometry history from instance placement

| | |
|---|---|
| Status | Accepted engineering decision; workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

A reusable Part has one geometry definition and can appear in several places.
Editing that definition must update its instances, while moving an instance must
preserve the Part's source geometry. Referenced placement also needs to follow
current upstream geometry without creating a dependency on its own output.

[Canonical transactions](0004-canonical-document.md) and the
[owned native evaluator](0012-native-model-evaluation.md) supply the shared
command and geometry boundaries. Display transforms alone cannot preserve
manufacturing geometry, reference rebuilding or attributable Undo.

## Decision

Persist direct Part moves as `model.transform` features, with capability
`model.transform.1`. Each feature freezes the preceding producer in `BodyInput`
and appends to the same Part history. The output Part identity is metadata;
the dependency link targets the preceding producer. This preserves an acyclic
history while retaining the Part's material, tags and containment.

Persist independent occurrence poses through the optional
`Instance.placement_source` identity. An Instance depends on its component and
its placement driver. The driver's owner identity is metadata, with no dependency
back to its own Instance. Numeric drivers use `model.instance_placement.1`;
referenced drivers use `model.instance_alignment.1`.

A driver starts from a validated literal rigid pose or a previous owned driver.
An Instance with no driver retains its existing placement representation.
A missing or invalid referenced driver produces an error without using the
legacy placement as a fallback. Copying an Instance clones its driver chain
with new identities and a new owner, while preserving parameter bindings.

Represent evaluated rigid poses as a proper rotation matrix and translation.
Numeric controls rotate around an explicit pivot, then translate in world
coordinates. Axis direction and signed angle remain canonical source values.
Matrix composition avoids repeated quaternion conversions; exact cardinal-angle
values retain exact zero and unit components. Native kernel tolerances still
govern B-rep validity and numerical measurements.

Persist `model.align` Part features (capability `model.align.1`) and instance alignment drivers with
explicit source and target anchors. Geometry captures live only on canonical
input links. The payload identifies those captures, or an explicit construction
plane or frame. Frame alignment requires a named semantic attachment; native
plane parameterization alone does not define authored UV orientation.

Axis alignment uses a native line, circle axis or planar normal with an explicit
Up vector. A parallel Up vector is an error. Point alignment preserves rotation.
Offsets and roll use typed dimensions and stable parameter bindings. Unsupported
curve descriptors and missing or ambiguous references require another choice
or explicit repair.

A moving Part's source anchor targets its frozen input Feature. A moving
Instance's source anchor uses component-local geometry and is transformed by
its current base pose. Targets may identify a frozen Feature, a dynamic Part
or a world-space Instance. The `model.part_reference.1` capability advertises
current-head Part aliases. Dependency hashes include the effective head's
source definition, including edits that leave the Part's feature IDs unchanged.

Evaluate required Part aliases and instance poses within the native dependency
order before consumers read their topology. Alias snapshots are reference data;
they add no physical output occurrences. Instance solids are actual rigidly
transformed native B-reps. Display meshes remain shared in component coordinates,
with separate occurrence transforms. World topology derives from current native
component descriptors and is checked against the transformed solid.

Use `prepare_placement_batch` for mixed explicit Part and Instance operations.
It returns one original-Session transaction with separate Part outputs and
Instance identities. Selecting a Part definition and its own Instance for the
same move is rejected because the definition edit already affects that instance.
Preview and cancellation preserve canonical source; Apply commits the exact
candidate validated by native evaluation.

The native UI uses the existing modeling candidate lane. It checks the source
hash, originating Session, form generation and native receipt before Apply.
Occurrence evidence verifies each current component producer and its resolved
pose separately from Part-output evidence. A source-only recovery view cannot
authorize a complete-model Apply.

## Consequences

Definitions and occurrences share geometry without sharing editable pose
ownership. Native consumers can query a Feature's component geometry or an
Instance's world solid by stable identity. Parameter edits and referenced
placement rebuild through the same dependency graph used by modeling commands.

The cost is an additional driver graph and reference closure. Current-head Part
aliases can introduce indirect cycles, so the evaluator must reject those cycles
rather than fall back to historical geometry. A copied referenced driver chain
also needs independent capture identities and explicit ownership validation.

Current UI alignment operates on one moving Part or Instance at a time; numeric
moves and copies support multiple explicit selections. Subassembly placement
and motion relationships require the assembly contract. Unsupported spline-axis
alignment remains an error. Low-power performance and cross-platform interaction
require separate measurements; native macOS checks do not establish those results.

## Alternatives considered

**Store only viewport transforms.** This simplifies interaction and preserves
shared meshes. It leaves exact manufacturing solids and reference consumers
without canonical placement source, so it cannot implement this contract.

**Apply every move to the Part definition.** This reuses ordinary feature history
and updates every copy consistently. It cannot place two instances independently
and gives a selection of one occurrence unintended effects on other copies.

**Clone the whole Part for every placed copy.** This gives each copy independent
history and uses existing Part evaluation. It duplicates definitions and breaks
the intended shared-component behavior. Explicit independent Part duplication
can remain a separate operation.

**Use final Part identities for every history input.** This follows later edits
with fewer source identifiers. A feature appended to that same Part would depend
on its own current output. Frozen source producers and explicit dynamic target
aliases keep those meanings distinct.

## References

- [Rigid math](../../crates/multicad-core/src/model/transform_math.rs) and
  [Part transform source](../../crates/multicad-core/src/model/transform.rs)
- [Instance drivers](../../crates/multicad-core/src/model/placement.rs) and
  [atomic batch](../../crates/multicad-core/src/model/placement/batch.rs)
- [Alignment source](../../crates/multicad-core/src/model/alignment.rs) and
  [current reference resolution](../../crates/multicad-core/src/model/alignment/resolve.rs)
- [Dependency hashes](../../crates/multicad-core/src/references/dependency.rs)
- [Native occurrences](../../crates/multicad-model/src/occurrences.rs),
  [alignment tests](../../crates/multicad-model/tests/alignment.rs) and
  [mixed-placement tests](../../crates/multicad-model/tests/placement_batch.rs)
- [Native UI controller](../../crates/multicad-ui/src/modeling/controller.rs),
  [occurrence evidence](../../crates/multicad-ui/src/modeling/controller/proof.rs) and
  [native candidate tests](../../crates/multicad-ui/src/modeling/controller/placement_tests.rs)
