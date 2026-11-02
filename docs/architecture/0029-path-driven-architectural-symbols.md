# ADR-0029: Pin architectural construction blueprints in the symbol Library

| | |
| --- | --- |
| Status | Accepted engineering design; implementation and acceptance remain separate |
| Date | 2026-11-02 |
| Decider | Codex, using the product owner's delegated implementation authority |

## Context

A reusable wall or fence must regenerate from a receiving project's path and
context while retaining its authored construction. Layers keep physical
thicknesses; fence posts and bays recompute along the run. A role label cannot
give an ordinary Part architectural joins, openings or post-layout behavior.

At `581bbe8`, [symbol definitions](../../crates/multicad-core/src/symbols/types.rs)
contain an embedded template, a Part/Assembly root and exposed numeric
parameters. [Materialization](../../crates/multicad-core/src/symbols/materialize.rs)
copies a private graph into a wrapper Assembly and outer Instance.
[Provider remapping](../../crates/multicad-core/src/symbols/remap.rs) rejects
unrecognized architectural providers. These verified boundaries do not establish
path/context or material-choice support.

[Wall constructions](walls.md) and [fence constructions](fences.md) already own
the canonical domain data and generation rules. The
[single Library](../../crates/multicad-ui/src/library.rs) lists saved native
projects and embedded symbol definitions. Existing projects depend on stable
definition hashes, private identities and explicit pinned updates.

## Decision

Extend the existing symbol definition with an audited architectural blueprint
root in its embedded template. The blueprint references the canonical wall or
fence construction and declares a typed public interface. It contains no
receiving path, level, generated member graph or mandatory sample run. A sample
run may remain in the authoring project as a preview fixture; it is outside the
reusable construction closure.

Determine Wall/Fence compatibility from the referenced canonical construction
and supported provider, then verify any declared role against that evidence.
Refuse incompatible roots and inputs before placement. Use an explicit required
architectural-symbol capability. Keep ordinary symbol serialization and pins
unchanged when no architectural blueprint is present.

Keep author-selected numeric parameters alongside typed construction dimensions,
material slots and supported construction choices. Bind slots to stable domain
identities, such as a wall layer or fence infill recipe, rather than arbitrary
JSON paths. Validate units, target kinds, choice membership and budgets. Receiving
material references are typed native Material identities. Private formulas and
unexposed construction fields remain protected.

Place an architectural blueprint through a role-specific instance record in the
same symbol subsystem. Retain its embedded definition pin and private identity
map. Supply path, level/base and supported placement context from the receiving
project. Materialize real canonical wall/fence records and physical Parts through
their existing domain planners. A placed wall remains a wall for joins, openings,
equipment and drawing consumers; a fence retains its named posts and infill.
Generic geometric scaling does not implement this provider.

Compose construction materialization, exposed inputs, receiving context and
generated member reconciliation into one source candidate before final Session
validation and native preflight. Reuse bounded domain source builders. A change
that adds/removes fence members must reconcile their identities and references;
updating a dimension while retaining a stale member set is insufficient. Cover
both explicit instance edits and supported parameter-driven topology changes.
Source preparation alone never certifies native solids or manufacturing output.

Extend the established compare/update workflow to include architectural instances.
Keep compatible public inputs and source-to-private identities. Report removed
or retargeted slots, layers, members and incompatible references for explicit
repair. Native evaluation of the complete affected candidate precedes one Apply.
Cancel preserves the current pin; Undo restores the pin, context, inputs and
generated source together. Reopen and evaluation use embedded definitions without
requiring the authoring file or Library to remain available.

Use the existing authoring metadata and Add to Library workflow. Draw Wall/Fence
selects compatible definitions from that Library; consuming Inspectors show the
exposed configuration and receiving placement controls. The domain type editor
continues to author the same canonical construction. UI and MCP use the same
typed preparation and validation contracts.

## Consequences

The existing Library, immutable pins, native domain evaluators, drawing consumers
and transactional history remain the shared foundations. Architectural placement
can change topology without stretching construction dimensions or flattening
physical members into an opaque compound.

The implementation costs are explicit provider/remapping coverage, another typed
instance form, stronger ownership checks and composed source reconciliation.
Maintainers must test compatible upgrades, removed references, parameter-driven
member changes and material choices together. Every supported provider adds
maintenance cost. Unverified or unsupported combinations stay unavailable rather
than inheriting support from a role label. Existing transaction and source budgets
continue to bound expansion.

This decision does not establish completed graphical authoring, actual macOS
interaction, cross-platform runtime or complete architectural provider coverage.
Those require implementation and recorded verification. Terrain-following fences,
gates and engineered foundations remain outside the flat-run domain contract.

## Alternatives considered

A captured sample Assembly would reuse the current materializer with fewer
changes. It retains physical content well, but its baked path and generic outer
placement cannot supply the required wall/fence path semantics or recompute fence
bays. Choose the construction blueprint and domain provider instead.

A separate architectural type catalog would simplify domain selection. It would
also split authoring, Library metadata, pinning and update ownership across two
systems. The product requires the existing single Library, so keep one definition
workflow and extend its typed provider coverage.

A virtual recursive geometry provider could avoid storing generated canonical
members. It would require new ownership, reference, occurrence and exporter
contracts for joins, hosted equipment and physical members. Materialize ordinary
canonical domain records to preserve those existing consumers.

## References

- [ADR-0024: Pinned symbol variants](0024-pinned-symbol-variants.md)
- [Transactional Session](../../crates/multicad-core/src/commands.rs)
- [Fence source preparation](../../crates/multicad-core/src/model/fence/prepare.rs)
- [Architectural symbol integration scope](https://plans.tokenet.de/CAD/206)
