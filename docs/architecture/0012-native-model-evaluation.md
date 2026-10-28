# ADR-0012: Evaluate model features on an owned native worker

| | |
|---|---|
| Status | Accepted engineering decision; CAD-64 workflow acceptance remains separate |
| Date | 2026-10-28 |
| Decider | Codex, using delegated architecture authority |

## Context

A sketch can produce multiple independent Parts. A later sketch can attach to a
generated face, so evaluating all sketches before all solids would lose the
dependency order. The UI also needs a candidate model before a transaction is
committed. That candidate shares the canonical document's base revision but has
different source content.

Native B-reps require one owning thread. Display meshes and reference snapshots
can cross threads; a mesh cannot replace a B-rep for manufacturing or exact
operations.

## Decision

Keep portable feature definitions and source-only preparations in
[`core::model`](../../crates/multicad-core/src/model/mod.rs). Store stable sketch
region intent, output Part identity and parameter bindings. The feature depends
on its sketch and upstream producers; its output Part is metadata. This avoids
introducing a dependency cycle through a final Part's feature history.

Use [`multicad-model`](../../crates/multicad-model/src/lib.rs) to evaluate exact
native geometry. Its compiler follows the canonical dependency graph, evaluates
parameters once per job, and shares solved sketches and extracted regions among
their consumers. It retains upstream topology snapshots so face-attached sketches
can resolve their source before generating downstream geometry. Missing or
changed region boundaries require explicit repair; selection never falls back
to the closest area or a tessellated polygon.

The initial executable provider supports unchanged-section New body extrusion
with independent normal start offset, sweep direction and distance extents.
Unsupported providers and controls return errors. This decision does not claim
completion of CAD-64's Boolean, taper or referenced termination requirements.

[`Runtime`](../../crates/multicad-model/src/runtime.rs) keeps one active job, one
replaceable pending job and one reply. Superseded work stops at checked
boundaries. Native handles are constructed and destroyed on that worker. A
publication callback also builds display acceleration structures there, keeping
that work outside the UI paint and polling paths. Replies retain their request
generation and the evaluated document identity, revision and source hash.

Keep canonical scene evaluation and modeling candidates on separate backend
instances. The UI holds the exact prepared transaction. A candidate reply can
enable Apply only after its source, session, revision, form generation and
expected output producers have been checked. Apply commits that transaction;
Cancel drops it. A retained older scene is never relabeled as current geometry.

## Consequences and verification

The UI and MCP can share the same typed preparations and native evaluator. New
geometry providers belong in the model layer rather than application widgets.
The application composition root supplies the display adapter and separate
backend factories.

The compiler currently rebuilds the required model graph for each job. It caches
within that job, but does not yet reuse unchanged B-reps across revisions. The
feature and display budgets bound a job; they are not a low-power performance
acceptance result. Assembly occurrence placement remains outside this initial
provider.

[`Compiler tests`](../../crates/multicad-model/tests/compiler.rs) check real
volumes, bounds, shared-sketch identities, invalidated profiles and a STEP
roundtrip from the owned B-rep. [`Runtime tests`](../../crates/multicad-model/tests/runtime.rs)
check superseded publication and nonblocking shutdown. Native UI acceptance and
Windows/Linux runtime evidence remain separate.
