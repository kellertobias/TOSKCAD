# ADR-0026: Retain hardware dimensions independently of the installed catalog

<!-- Kind: engineering decision. Audience: maintainers. Purpose: fix geometry and catalog-reference ownership. Non-goals: catalog coverage, standards or redistribution approval, native workflow acceptance. Comparable: ADR-0024; indexed from README.md. -->

| | |
|---|---|
| **Status** | Accepted engineering design; native and catalog verification remains separate |
| **Date** | 2026-11-01 |
| **Deciders** | Codex, under authorized CAD-200 execution |

## Context

[CAD-200](https://plans.tokenet.de/CAD/200) requires sourced mechanical presets,
editable nuts and washers, and offline pinned library definitions. A saved
project must keep its dimensions when the installed catalog changes. The
existing Library lists original project files; consuming projects embed symbol
definitions and update them explicitly under [ADR-0024](0024-pinned-symbol-variants.md).

A sourced designation and an editable geometry recipe answer different
questions. A catalog reference records where dimensions came from. The current
recipe determines the solid, including formulas and custom changes. A label
alone cannot establish dimensional conformance, stock availability, tolerance
class or redistribution permission.

## Decision

Retain complete typed geometry parameters in each canonical mechanical Part
feature. Native evaluation reads these parameters without looking up a mutable
installed catalog. Library symbol templates embed the same self-contained
source. Continue to use existing Part, occurrence, symbol-pinning and explicit
update transactions.

Keep optional catalog provenance beside the recipe: a catalog identifier,
revision, designation, source URL and a closed literal reference-geometry
snapshot. This snapshot is descriptive evidence for comparison; it never drives
or replaces the authored parameters. An ordinary parameter edit retains the
reference. Catalog presentation must distinguish unchanged sourced dimensions
from a custom or derived variant, and must not call an unknown reference a
verified built-in preset. Catalog matching and curation remain implementation
work owned by CAD-200.

Reference snapshots contain no parameter bindings or canonical entity IDs.
Symbol collection/remapping excludes their descriptive fields, including
UUID-looking catalog identifiers, revisions and URL segments. Remap only the
actual recipe/output identities and bound parameter references through the
provider's audited schema.

Reuse the existing typed thread model for nut thread intent. Nut height is the
thread length and the thread frame locates the whole nut; do not retain a
second independently editable height or axis. Cosmetic thread envelopes must
retain the existing manufacturing-export refusal until switched to Modeled.
A bearing mounting envelope is explicitly an envelope: external mounting
geometry does not establish modeled races, rolling elements or seals.

## Consequences

**Positive:** saved geometry remains stable across catalog updates and missing
library files. Parametric edits, Undo/Redo and pinned variants use canonical
source ownership. Source facts remain attributable without silently overriding
custom geometry.

**Negative:** provenance duplicates a small dimensional reference snapshot, and
catalog presentation needs explicit comparison and version verification.
Maintainers must update the audited identity boundary when a provider gains new
ID-bearing fields. CAD-200 owns these obligations and the source/rights review;
this decision supplies neither verified catalog data nor native workflow proof.

**Neutral:** a changed recipe may retain its original source reference while
being displayed as derived/custom. A Modeled nominal thread still carries the
limits of its selected profile; representation does not establish a tolerance
class or complete standards compliance.

## Alternatives considered

**Resolve dimensions from an installed catalog ID on every rebuild.** This keeps
saved source compact and centralizes corrections. It also makes old geometry
change when the catalog changes or disappear when a version is removed. It does
not provide the required independent offline pin.

**Store only copied dimensions.** This is self-contained and simple to evaluate.
It loses the original reference needed to distinguish curated presets from
custom edits and to review a later explicit catalog update.

## References

- [CAD-200: sourced standard-parts library](https://plans.tokenet.de/CAD/200)
- [Existing file metadata and Library ownership](file-metadata-library.md)
- [Canonical mechanical Part source](../../crates/multicad-core/src/model/mechanical_parts/mod.rs)
- [Audited symbol identity remapping](../../crates/multicad-core/src/symbols/remap.rs)
- [Thread geometry and representation limits](threads.md)
