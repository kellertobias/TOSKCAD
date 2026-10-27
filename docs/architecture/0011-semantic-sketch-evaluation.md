# ADR-0011: Keep authored sketch semantics separate from evaluated geometry

| | |
|---|---|
| Status | Accepted engineering decision; native workflow acceptance remains separate |
| Date | 2026-10-27 |
| Decider | Codex, using the delegated architecture authority |

This decision owns constrained sketch source, derived evaluation and exact region
queries. [Canonical document ownership](0004-canonical-document.md),
[native geometry policy](0006-geometry-policy.md) and
[plane occurrence context](0009-sketch-plane-occurrences.md) retain their scopes.

## Context

A parameter change must update solved geometry while preserving the expression
and its identity bindings. Writing a numerical result into an authored dimension
would remove that dependency. Replacing every point guess after each solve would
also make unrelated parameter edits change the sketch's canonical source.

Spline poles can participate in point constraints even when the numerical solver
has no spline entity. Exact profiles also require geometric predicates beyond the
core's analytic line and circular-curve implementation. A display polyline cannot
establish a spline's intersections or manufacturing boundary.

The core must validate editable source without depending on the native kernel.
The UI needs responsive evaluation and must distinguish current geometry from a
retained presentation after a failed rebuild. Native handles require an owning
thread, while the document persists portable definitions.

## Decision

Keep authored source and evaluated geometry as separate values.
[`Source`](../../crates/multicad-core/src/constraint_sketch/source.rs) owns point
guesses, authored curve identities and constraints. Dimensions retain their
unit-bearing literals or bound expressions. Expression dependencies use parameter
UUIDs through explicit dimension links; renaming a parameter does not rebind it
by display name.

The constrained sketch provider remains `core.constrained_sketch`. Its schema and
native-container requirements are:

| Schema | Source content | Evaluation and capability rule |
|---|---|---|
| 1 | Literal dimensions with lines, arcs and circles | Retains solved-source editing compatibility; spline payloads are rejected |
| 2 | Bound dimensional expressions, including retained semantic source after expression removal | Requires `EvaluatedSketch` and `core.constrained_sketch.2` |
| 3 | Nonperiodic clamped splines with pole UUIDs and a typed basis; bound dimensions remain available | Requires `EvaluatedSketch` and `core.constrained_sketch.3` |

Preserve existing literal payload serialization. A replacement retains an
existing semantic schema after its last expression or spline is removed. New
literal definitions may retain the compatibility schema. The
[`canonical edit boundary`](../../crates/multicad-solver/src/canonical_edit.rs)
prepares semantic source explicitly, using the maximum required or existing
schema. The [native container](../../crates/multicad-core/src/project_file/model.rs)
declares the required capability and includes sketch semantics in its cache
interpretation fingerprint.

Return [`EvaluatedSketch`](../../crates/multicad-solver/src/evaluated.rs) with
canonical source, solved geometry and resolved dimensional values kept separately.
Semantic edit preparation commits the requested source and dependency links.
Computed coordinates and resolved dimension values remain derived results.
[`prepare_draw`](../../crates/multicad-solver/src/draw.rs) validates existing-point
snaps against current solved geometry, then merges only newly authored identities
into canonical source. Dropping a prepared preview changes no canonical source.
Failed solves return diagnostics without a commit-capable preview. Require UI and
MCP adapters to use these actor-attributed transactions.

Evaluate immutable source snapshots on a bounded compiled Rust worker.
`SketchEvaluationContext` shares parameter evaluation and the canonical document
hash across its requested sketches. The
[`UI evaluation feed`](../../crates/multicad-ui/src/sketch_evaluation.rs) adds
scope, request generation, reference context and query-service generation to the
document identity, revision and source hash. It rejects superseded results and
keeps any retained presentation unavailable for picking or downstream work.
Profile point classification has its own request generation.

Inject [`RegionQueries`](../../crates/multicad-core/src/sketch_regions.rs) through
a worker-side factory supplied by the host. The core owns authored boundary
identity, connectivity, nesting and partial diagnostics. Its analytic service
explicitly rejects unsupported spline predicates. Fallible classification
propagates that failure to callers. For supported nonintersecting boundaries,
each bounded area remains independently selectable, with immediate nested
boundaries as holes.

Use [`NativeRegionQueries`](../../crates/multicad-kernel/src/sketch_queries.rs)
for spline intersections, bounded area and point classification. Analytic-only
predicates retain the Rust analytic implementation. The native adapter constructs
true lines, trimmed circles, circles and B-splines for mixed profiles. Exact
extrusion consumes those curves and authored origins. Native handles stay local
to their calling thread; the UI receives owned geometry and diagnostic data.
Cancellation checks surround predicates and loop iterations. An active native
predicate completes before cancellation takes effect.

## Consequences

**Positive:** Parameter edits preserve expressions and authored UUIDs while
updating solved geometry. Exact region selection and manufacturing consume the
same curve definitions. Core source validation remains independent of OCCT, and
the host can supply a different exact query service through the same interface.

**Negative:** Consumers must carry evaluation stamps and handle unavailable
geometry instead of reading point guesses as a result. Query-service injection
and asynchronous classification add host integration work. Retained display data
cannot authorize an operation, and bounded native calls can delay cancellation.

**Neutral:** Display sampling belongs to
[`Geometry::segments`](../../crates/multicad-core/src/sketch.rs) and
[`SplineBasis`](../../crates/multicad-core/src/constraint_sketch/spline.rs).
Changing those samples does not change an exact profile. Native project files
persist canonical source separately from disposable derived caches.

## Current implementation limits

The [solver adapter](../../crates/multicad-solver/src/adapter.rs) registers spline
poles as ordinary solver points. Point constraints and parameter-driven distances
can move them. It creates no spline solver handle and rejects both driving and
reference constraints targeting the spline curve. Spline tangency and length
constraints remain unsupported, as do knot or weight solving.

The typed basis supports finite, bounded, nonperiodic clamped splines with optional
positive weights. Its validator owns the degree, pole, knot and conditioning
limits. The [native curved-profile adapter](../../crates/multicad-kernel/src/curves.rs)
owns profile budgets and validates construction against the numerical policy.
The extractor diagnoses crossing and touching loops. Source corrections remain
explicit edits.

[`Trim and extend`](../../crates/multicad-core/src/sketch_trim.rs) operate on line
targets with supported line, arc or circle boundaries. Spline targets and spline
boundaries return explicit unsupported errors. Display samples never substitute
for those missing predicates. A production modeling compiler, complete spline
editing and manufacturing export workflows require their own implementation and
acceptance evidence. MCP transport and authentication remain separate application
work.

## Alternatives considered

**Persist every solved coordinate and replace formulas with numerical values:**
this gives consumers a directly readable geometric snapshot and reduces derived
state handling. It removes authored dependencies and couples parameter evaluation
to unrelated source changes. Separate evaluated values preserve semantic edits.

**Make the core call OCCT directly:** this gives region extraction one concrete
implementation and removes host injection. It also couples source validation to
native deployment and handle ownership. An injected query boundary keeps those
responsibilities in the compiled native adapter and worker.

**Extract spline regions from display polylines:** this would reuse line-only
connectivity and picking code. Approximation can change intersections and bounded
areas with sampling settings. Manufacturing and exact region classification
therefore use the native curve basis.

## References

- [Source schemas, container capabilities and compatibility tests](../../crates/multicad-core/tests/sketch_spline.rs)
- [Bound expressions and stable parameter identity](../../crates/multicad-core/src/parameters/bound.rs)
- [Semantic source transactions](../../crates/multicad-solver/src/transaction.rs)
- [Spline pole solving, unsupported constraints and undo tests](../../crates/multicad-solver/tests/spline.rs)
- [Transform, pattern and trim behavior for spline source](../../crates/multicad-solver/tests/spline_operations.rs)
- [Exact mixed curves, provenance and STEP roundtrip tests](../../crates/multicad-kernel/tests/curves.rs)
- [Evaluated spline regions, native volumes and reopen tests](../../crates/multicad-ui/tests/spline_sketch.rs)
- [Native query-factory installation](../../crates/multicad-app/src/main.rs)
- [Native project container decision](0008-project-container.md)
