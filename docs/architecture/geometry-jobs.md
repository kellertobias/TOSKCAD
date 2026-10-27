# Background geometry jobs

[`GeometryWorker`](../../crates/multicad-kernel/src/worker.rs) evaluates a numeric
geometry program on a dedicated thread. Native solid handles stay on that thread;
the caller receives copied display meshes, surface metadata, provenance and opaque
solid keys.
The numerical rules are owned by [ADR-0006](0006-geometry-policy.md).

## Submission and publication

A program contains a document revision, numerical policy, ordered operations and
output indices. Operations reference earlier entries, which makes cycles invalid.
Outputs retain their requested order, including `None` for empty volumetric results.
A program with no operations and no outputs represents an empty document.

`NamedBox` and `NamedExtrude` carry canonical feature and authored curve identities.
Boolean and transform operations propagate their native ancestry. Each published
solid contains owned topology elements. `topology_snapshot(owner)` binds them to
the canonical output entity supplied by the feature compiler and the receipt's
source revision. Unnamed operations produce no inferred provenance. The source
intent and repair rules are described in [Geometry references](geometry-references.md).

`submit` validates the program and attempts a nonblocking enqueue. The queue holds
four pending commands. Each program permits at most 4096 operations, 1024 outputs
and 65536 profile edges across its operations. The mesh triangle budget applies to
all outputs together. A full queue returns `Busy` without superseding the previous
accepted request.

An accepted build increments the worker generation. The worker rejects older
requests before computation, between operations and before publication. Receiving
a result also checks the generation, so an obsolete receipt cannot be adopted
merely because its calculation finished earlier. A successful publication retains
its exact solids in the worker cache and replaces that cache as one unit.

Use `GeometryJob::try_take` from the application event loop. `wait` blocks and is
intended for tests or headless callers. `cancel` rejects the receipt and causes the
worker to stop at a subsequent operation boundary. Dropping a job also requests
cancellation. A native OCCT call already in
progress runs to completion; cancellation does not guarantee immediate preemption.
Worker shutdown cancels publication and joins after native work reaches a boundary.

## Exact geometry access

Each `SolidKey` identifies one worker, generation, source revision and output slot.
`write_step_to_staging` rejects foreign keys, superseded generations and mismatched
revisions. The writer reads the retained B-rep, so modifying a returned display
mesh cannot affect the STEP geometry.

A staging write is an intermediate operation. Cancellation or a newer revision
can reject its receipt after the native writer has produced a file. The caller
owns that temporary path, cleans up rejected output, checks its canonical source
revision again and publishes the file through its export transaction. The kernel
worker does not implement final file replacement or an export recipe.

## Verification and integration boundary

[`worker.rs` integration tests](../../crates/multicad-kernel/tests/worker.rs)
execute native background builds, Booleans, staging exchange and cancellation.
They verify stale/foreign key rejection, empty results, recovery after failures,
whole-model mesh limits and shutdown. The unit test in the worker module uses an
undrained live channel to check queue pressure without timing assumptions.

The application still needs a canonical feature evaluator to construct these
programs, manage document revisions and adopt accepted scene snapshots. Native
viewport integration, interactive performance and final manufacturing exports
require their own checks. Background native tests establish the worker behavior,
not a completed application modeling workflow.
