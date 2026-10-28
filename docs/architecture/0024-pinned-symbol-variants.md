# ADR-0024: Pin symbol definitions and materialize stable owned variants

| | |
|---|---|
| Status | Accepted engineering design; implementation verification remains separate |
| Date | 2026-10-29 |
| Decider | Codex, using delegated architecture authority |

## Context

A reusable cabinet or door has one authored source and a chosen public parameter
interface. Placed copies can have different dimensions, while internal formulas,
sketch constraints and feature history must remain intact. Projects must reopen
with the version they used even when a library changes or disappears.

The current native compiler, assembly paths, references, drawing scopes and
manufacturing exporters resolve canonical Part, Assembly and Instance entities.
Session history owns the Document. ProjectResources has a separate lifecycle,
so mutating an external definition payload during Apply would not automatically
participate in Undo.

## Decision

Embed an immutable, versioned `symbols.definition` Extension in the canonical
Document. It records the stable library symbol identity, revision, content hash
inputs, selected source root, supported dependency closure, exposed parameters
and optional local plan sketch. Library files carry these same definitions in
the native project container. Reopening a project performs no implicit library
lookup or upgrade.

Each placement has a `symbols.instance` record and a persisted map from template
identities to private canonical identities. Materialize its supported source
graph as ordinary canonical entities. A placement-owned wrapper Assembly and
ordinary outer Instance preserve the existing native occurrence and export
paths. Generated members and constraints are checked against the pinned source;
they are edited through the symbol's exposed values or an explicit definition
update. The outer Instance remains movable through existing placement tools.

Static symbols retain the definition's design values. Dynamic symbols accept
only author-exposed, explicitly unit-bearing values. An override replaces that
parameter's expression; unoverridden internal expressions and their stable
bindings continue to evaluate. Private parameter names avoid collisions between
placements. Generated parameters stay outside the general Parameters table.

Preserve the identity map during parameter changes and compatible upgrades.
Allocate identities only for newly added source entities. Remap supported schema
identities, including sketch element identities and bound parameter references.
Reject unsupported opaque providers rather than rewriting arbitrary UUID-like
strings or dropping their data.

An explicit update compares revision/content, changed parameters and source
entities, affected instances and incompatible overrides. Compatible values remain
attached to their source parameter identities. Conflicts require an explicit
resolution. The complete candidate must pass native evaluation and driving
reference checks before one attributable Apply updates the definition and its
affected instances. Cancel leaves canonical source unchanged; Undo restores the
previous pin, parameter values and generated graph together.

Use the same guarded typed preparation commands for native UI and external
control. Candidate display work uses the existing background native scene
provider. Bind Apply to the live Session, revision, source, resources and exact
prepared candidate. Library read/write jobs use the existing bounded file worker
and operating-system picker; exporting a definition does not mark the project
saved.

The optional local plan sketch uses the same identity map and outer placement.
Its link remains part of the symbol instance record. Drawing consumers can use
that representation without inventing a separate independently placed object.

## Consequences

Symbols reuse the existing exact geometry, feature selection, nested occurrences
and manufacturing paths. One source definition governs all its materialized
variants, and ordinary Session history covers the whole update.

Private source graphs increase document size. Every placement initially receives
its own materialization, including static copies; sharing identical evaluated
geometry can be added without changing authored identity. Source and placement
budgets bound expansion. Adding a new provider requires a tested remapping rule.
Removed source identities can invalidate downstream references; updates cannot
silently attach them to another edge or face.

## Alternatives considered

A recursive symbol geometry producer would compile each embedded template as a
second model. It would also require virtual component identities, nested leaf
path resolution and resource-aware changes across selection, references,
drawings and export. Flattening the result to a compound solid would lose those
component semantics.

Copying an untracked independent definition for every placement would simplify
editing but lose explicit pinning and update behavior. Mutating ProjectResources
outside Session history would split an update across two ownership systems.

## References

- [Symbol source contracts](../../crates/multicad-core/src/symbols/mod.rs)
- [Transactional Session](../../crates/multicad-core/src/commands.rs)
- [Assembly source ownership](../../crates/multicad-core/src/model/assembly/source.rs)
- [CAD-70](https://plans.tokenet.de/CAD/70)
