# ADR-0031: Retain compound cutout intent and receiver histories

| | |
| --- | --- |
| Status | Implemented; automated verification complete, native-window acceptance pending |
| Date | 2026-11-02 |
| Decider | Codex within CAD-247 implementation scope |

## Context

CAD-148 stores a rectangular cutout identity in wall and slab records. That
workflow remains valid, but flattening several boxes into those rectangles would
lose their editable identities, their union and their vertical extents. CAD-247
requires one persistent compound definition and one atomic change to explicitly
chosen receivers.

## Decision

Keep the existing rectangular source, tools and tests. Add a versioned compound
cutout feature under the same Cutout command. It owns 1–32 box identities and
1–64 explicit receiver inputs. Each box has a parameterized plan rectangle and
rotation. Its finite vertical interval inherits the shared extent or has an
explicit override. The shared extent either follows the chosen architectural
receivers or uses authored bottom/top dimensions. These fields define actual
box volumes; no display mesh enters manufacturing geometry.

The compound source is a native cutter feature without an independent final
Part. Generated receiver features append cuts to the existing Part histories.
They refer to the compound source and frozen input producers. Retain receiver
Part IDs and retained generated-feature IDs across edits. Box IDs name source
provenance rather than native enumeration order. The source, all generated cuts
and history updates are prepared in one canonical Session transaction shared by
UI and MCP. Preview alone changes no source.

Receiver inputs must resolve to supported wall/slab histories, including earlier
compound cuts. A chosen level range remains explicit and never selects unrelated
solids during a rebuild. Removing or changing a receiver must refuse to orphan
downstream history. Missing, suppressed or inconsistent generated sources require
repair rather than fallback selection. New capabilities make unsupported readers
refuse the new definition; existing rectangular files serialize unchanged.

The native model worker builds the exact box union and requires one connected
volumetric cutter. Overlapping boxes are permitted; disconnected unions, boxes
missing every receiver and selected receivers with no material intersection fail
the whole operation. Empty receiver results and disconnected receiver pieces are
refused. Layered walls retain exact material components and their source-owned
face provenance; native component order is not a material identity.

Retain CAD-148's slab boundary, existing-hole and through-penetration rules.
Contributing footprints that cut a slab must lie strictly inside its outline and
must not meet existing holes. The union must penetrate the complete slab
thickness; partial pockets are not silently treated as shafts. Check the actual
union, including overlapping or stacked boxes. Wall cuts retain the prohibition
on splitting surviving physical material. Existing openings and other cutouts
must receive explicit overlap diagnostics.

The form displays receiver names and the resolved vertical extent before Apply.
It shows contributing boxes separately from the cut result, with graphical add,
move, resize and remove controls. Editing a generated cut reopens its compound
source. The legacy rectangle path remains available; no separate Shaft command
or second building document is introduced.

## Verification boundary

Implementation must demonstrate an editable L-shaped penetration, wall/slab/
combined targets, level ranges, overlap and failure cases, stable IDs, one Apply,
Undo/Redo, UI/MCP parity and container reopen. Native volume checks, independent
export readbacks and linked plan/section views must prove the receiving geometry.
Required Rust checks and actual macOS window interaction remain separate evidence;
this decision record establishes neither runtime correctness nor human acceptance.

Implementation evidence and remaining acceptance are recorded in the
[CAD-247 verification report](../verification/compound-cutouts-2026-11-02.md).
