# ADR-0025: Derive gear-pair placement through assembly joint anchors

| | |
|---|---|
| Status | Accepted engineering design; implementation and workflow verification remain pending |
| Date | 2026-11-01 |
| Decider | Codex, using delegated implementation authority |

This record owns placement and source dependencies for external parallel-axis
gear pairs. [CAD-199](https://plans.tokenet.de/CAD/199) owns family coverage and
workflow acceptance. The existing [assembly decision](0020-directed-assemblies-and-physical-leaves.md)
continues to own occurrence identity, physical leaves and directed relationships.

## Context

A gear pair needs two identifiable parametric Parts, mating center positions and
tooth phase. Module, tooth count, helix and placement parameters can change after
creation. A saved rigid placement alone cannot express which gear dimensions
determine its center distance or rotation ratio.

The canonical document already supports reusable Assembly definitions, linked
Instances and directed joints with component-local and world attachment frames.
The UI and MCP share its typed preparation and native candidate evaluation.

## Decision

Store a versioned pair definition that references both canonical gear features,
a base frame and a typed drive angle. Retain each gear recipe in its own feature.
The pair records its output Assembly, Instances and joints as identities, without
making those outputs its evaluation prerequisites.

Extend joint anchors with explicit pair-derived local and mating frames. Use the
existing Fixed relationship for each member. The local anchor follows the gear's
current source frame; the mating anchor derives its pose from both resolved gear
recipes and the pair's base frame and drive angle. Keep these relationships under
pair control. Generic joint edits must not replace their derived anchors with
copied numeric values.

Resolve center distance and tooth phase during evaluation. Validate compatible
module and pressure-angle conventions, family, helix magnitude and opposite hands
where required. The initial parallel-axis implementation may require matching
face widths and matching center gaps; it must report those restrictions explicitly.
A change that makes the pair incompatible produces a diagnostic rather than a
silently retained old placement.

Create the pair definition, both gear Parts, its Assembly metadata, member
Instances and relationships through one typed transaction and one complete native
candidate gate. Use existing definition-only Part metadata so the source Parts
do not become duplicate physical outputs. Preserve identities on edits and reuse
the existing Undo/Redo, project persistence and occurrence export paths.

Run bounded native interference samples on the resulting mating solids and
report the tested phases. State the periodic geometry assumptions. Initially,
require each paired Part's current head to remain its gear generator; downstream
features that invalidate that assumption require explicit repair or a separately
verified extension. A finite sample set does not establish a continuous
collision-free motion, strength, load capacity or manufacturing fit.

## Consequences

Positive: parameter edits, occurrence placement, the viewport, native inspection
and manufacturing export resolve the same pair intent. Gear Parts retain their
independent identities and recipes without introducing another assembly solver.

Negative: derived anchors add a versioned capability, dependency validation and
editor rules to the joint boundary. Pair-controlled joints need a distinct edit
path. Initial family, width and downstream-feature restrictions reduce the set of
accepted pairs. Native interference checks add preview cost.

Neutral: pair motion remains an authored drive coordinate in the existing
directed placement evaluator. Closed kinematic loops, independent per-copy drive
overrides and general gear-train solving remain separate work.

## Alternatives considered

**Persist derived numeric placements.** This uses existing frame fields and keeps
the first implementation small. Later module or tooth-count edits can leave the
positions and phase stale unless every edit path also rewrites those outputs.
The derived-anchor design keeps that dependency in the canonical evaluator.

**Add a separate gear placement driver.** A dedicated driver can contain all
pair equations without extending joint anchors. It creates another placement
ownership and editing path beside existing assembly relationships. Derived
anchors retain the current directed-joint and physical-occurrence contracts.

**Adopt a simultaneous mechanical solver.** This could support closed gear trains
and coupled degrees of freedom. It also requires solver selection, branch and
overconstraint behavior, and a broader UI contract. A deterministic derived pair
fits the current directed assembly boundary.

## References

- [Canonical gear definitions](../../crates/multicad-core/src/model/gears/mod.rs)
- [Joint parameters and anchors](../../crates/multicad-core/src/model/assembly/parameters.rs)
- [Joint frame resolution](../../crates/multicad-core/src/model/assembly/resolve.rs)
- [Atomic mixed placement preparation](../../crates/multicad-core/src/model/placement/batch.rs)
- [KHK gear dimensions and normal/transverse conventions](https://khkgears.net/new/gear_knowledge/gear_technical_reference/calculation_gear_dimensions.html)
