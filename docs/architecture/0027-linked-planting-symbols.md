<!-- Kind: ADR; Audience: native model, drawing and UI maintainers; Purpose: define ownership of planting geometry and plan symbols; Non-goals: botanical simulation, terrain or human visual acceptance. -->
# 27. Keep planting semantics on pinned symbols and render their authored plans

Status: Accepted (2026-11-01)

## Context

CAD-202 requires one placed plant with simple 3D context and a reliable scaled plan symbol. The symbol API already embeds a local plan sketch and remaps it with the same instance inputs as its model. Current model tests prove that link and placement, but production drawing generation does not consume `symbol_plan`. Projecting canopy solids would couple planting linework to faceting, hidden-line work and architectural cut height.

## Decision

Use existing pinned symbol definitions, private parameter variants and occurrence placement for plant dimensions and geometry. A typed plant-template record identifies the component, height/spread parameters and authored plan sketch. An instance-settings record carries size meaning, optional label and plan visibility; it contains no second placement or copied dimensions. Built-in generic tree and shrub definitions use the same installation Library as other reusable parts.

Top-down architectural drawing evaluation will consume the linked authored plan in its current occurrence pose. Plant plans use configured canopy/spread, rather than trunk size. Their foliage symbol remains independent of physical section geometry; section/elevation context retains the deliberately simple 3D form. Plan linework is transparent by default, with stable ordering and controlled paper line weights. Labels follow the same source and placement. All page formats consume the common checked page-render description.

Plant dimensions are explicitly planned/mature sizes or current-size settings. Generic presets make no species suitability, growth, mass or manufacturing claims. Public UI and MCP preparations must use the same canonical records and native preview guards.

## Consequences

One pin and parameter mapping keep plan and model identities together through placement, edits and reopen. Authored curves can preserve recognizable monochrome outlines at 1:50 and 1:100 without projecting detailed canopy surfaces. A changed installed preset cannot silently replace an embedded definition.

The cost is additional audited metadata, source capability/version handling, drawing overlay evaluation and graphical placement work. Generic symbol support alone does not prove that plant drawings, labels, overlap behavior or PDF/DXF geometry work; each needs native regression and macOS evidence. Instance settings must remain attached to their symbol source through supported copy/update operations.

## Alternatives

Projecting the simplified 3D crown would reuse the existing drawing path and avoid authored plan evaluation. It would also expose canopy construction edges and make plan visibility depend on projection/cut behavior. Independently placing a drawing icon would be easy to render, but would duplicate position and dimensions. A dedicated landscape asset store would duplicate the installation Library and its pin/update lifecycle. None meets the requested shared-source behavior as directly.

## Implementation evidence

The [native checkpoint](../verification/planting-2026-11-01.md) records canonical/native tests, independent PDF/DXF measurements, interactive macOS placement/edit/export, retained pins and the final workspace checks. This decision records the implemented architecture; PLAINER Review/test and human acceptance remain separate.
