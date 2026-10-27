# ADR-0006: Own exact solids and make tolerances explicit

| | |
|---|---|
| Status | Accepted engineering decision; implementation and product acceptance remain separate |
| Date | 2026-10-27 |
| Decider | Codex, using the delegated architecture authority |

This decision owns kernel units, numerical limits and native handle ownership.
Source feature definitions, persistent topology references and export recipes belong
to their respective application modules.

## Context

A parametric rebuild needs reusable B-rep geometry for subsequent operations and
manufacturing exchange. Display tessellation has a different purpose and may change
with viewport settings. Sharing mutable triangulations with modeling operands would
couple those settings to cached native objects.

Floating-point classification can merge nearby boundaries or fail on degenerate
inputs. The application needs bounded, inspectable tolerances and recoverable
errors. A successful small regression corpus cannot guarantee every model scale,
Boolean arrangement or imported file.

## Decision

Use the owned [`Solid`](../../crates/multicad-kernel/src/geometry.rs) API for
modeling. Rust validates inputs and retains opaque native lifetimes through CXX.
The bridge converts OCCT failures to Rust errors and checks native results.
Boolean outputs without a volumetric component return `None`; disjoint unions may
contain multiple solids, reported through `solid_count`.

Coordinates, distances and surface queries use millimeters. Volumes use cubic
millimeters and areas use square millimeters. Profile frames and rigid placements
require orthonormal axes. Scaling, reflection and shear require separately defined
modeling operations rather than a rigid placement.

The following defaults and bounds are implemented by `GeometryPolicy::validate`.
Feature evaluators must retain their numerical policy with the source operation
when policy settings become editable.

| Setting | Default | Validation |
|---|---|---|
| Linear tolerance | 0.0000001 mm | Between 0.0000001 and 0.001 mm |
| Additional Boolean fuzz | 0 mm | From zero to the linear tolerance |
| Minimum feature length | 0.00001 mm | At least ten times the linear tolerance |
| Maximum absolute coordinate | 10000000 mm | Finite, above minimum feature length, no greater than this default |

The wrapper checks ordered profile closure against the linear tolerance and
normalizes wire orientation. OCCT still applies its own algorithm tolerances;
wrapper validation does not promise that every accepted profile can be built.
There is no automatic retry with a wider Boolean fuzz value or silent repair.
Callers receive an explicit error if native construction or validation fails.

Use non-destructive, single-threaded Boolean calls on immutable operands.
Tessellation copies the B-rep before attaching display triangulations and applies a
triangle budget. Changing display deflection must preserve the original solid's
volume, bounds and analytic face queries. STEP exchange explicitly uses millimeters.

Keep native solid handles on their owning thread. `Solid` implements neither
`Send` nor `Sync`, verified by compile-fail doctests. A background evaluator must
construct and destroy native geometry there, returning owned display/query data.
The command host must reject cancelled or superseded results before publication.

## Consequences

Positive: a caller can cancel or reject a candidate while retaining the prior
valid model. Display meshes can be regenerated independently of manufacturing
geometry. Evaluation-local face ordinals support interrogation without promising
persistent topology identity.

Negative: detached meshing copies native geometry and increases transient memory.
The explicit model envelope rejects some microscopic and geographically large
inputs. Native calls can still fail inside that envelope, and later evaluators
need cancellation and revision-aware publication in addition to these primitives.

Neutral: the Rust kernel interface receives numeric millimeter values. The shared
parameter engine converts typed quantities before invoking it. The application
must rebuild from canonical feature source rather than accumulate mesh edits.

## Alternatives considered

Modify display meshes for subsequent modeling. This would reuse viewport data and
avoid retaining a second representation. It loses analytic curves and ties
manufacturing results to tessellation settings, so it does not meet the source
model contract.

Automatically widen Boolean fuzz until an operation succeeds. This could recover
some near-coincident arrangements without user intervention. It can change gaps
and merge intended boundaries, so the chosen policy requires an explicit value.

Declare opaque native handles `Send` and move them between UI and worker threads.
This would simplify transferring cached shapes. Native thread safety and shared
OCCT subobject mutation are not established by the wrapper, so handles remain
confined to their owner.

## References

- [`geometry.rs`](../../crates/multicad-kernel/src/geometry.rs): numerical policy,
  profile checks, owned solid operations and display result validation.
- [`bridge.cpp`](../../crates/multicad-kernel/src/bridge.cpp): exception conversion,
  detached tessellation, Boolean settings, planar face queries and STEP units.
- [`geometry.rs` tests](../../crates/multicad-kernel/tests/geometry.rs): profiles,
  Booleans, placements, invalid inputs, exchange and representation separation.
- [`geometry_corpus.rs`](../../crates/multicad-kernel/examples/geometry_corpus.rs):
  reproducible native report and STEP fixture with policy and source identity.
