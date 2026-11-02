# Compound architectural cutouts

CAD-247 adds **Use multiple boxes** to the existing Cutout tool. The reviewed
rectangular wall/slab operation remains available. A compound cutout retains its
boxes and explicitly chosen receiving Parts in one canonical source. Graphical
placement edits box footprints in world XY; numerical controls retain rotation,
parameter expressions and optional per-box vertical limits. Wireframes show the
finite boxes in the native viewer. Receivers and resolved heights appear before
Apply.

## Source and transactions

| Record | Contract |
| --- | --- |
| `architecture.compound_cutout`, version 1 | One source with 1–32 stable box IDs, 1–64 explicit receivers, shared vertical extent and optional floor-level range |
| `architecture.compound_cutout_receiver`, version 1 | Generated cut appended to the existing receiving Part, linked to its compound source and frozen upstream producer |
| Box | Parameterized plan center, width and depth, finite rotation in degrees, optional bottom/top override |
| Extent | Through the canonical heights of the selected receivers, or explicit bottom/top dimensions |
| Receiver | Explicit Part and producer IDs, plus a retained generated-feature ID; walls, slabs, pedestals and previous compound-cut histories are supported |

The source has no independent final Part. Default display/export therefore contains
the receiving solids. Retained receivers keep their Part, generated feature and
frozen producer identities on edits. Unrelated downstream operation kinds require
repair before they can become new cutout inputs. History traversal is bounded to
128 producers. Removing a receiver/source refuses dependent history or references.
Missing, suppressed, inconsistent or bypassed producers do not select replacements.

`prepare_compound_cutout` stages create, edit or removal through the shared Session
API. UI and MCP obtain complete native proof before committing. The native master
checks every receiver even when evaluation was requested for only one member.
Preview and Cancel leave source unchanged; Apply is one undoable transaction.
Save/reopen retains the source and generated histories. Linked drawing views refer
to the existing Parts and use their current native producers.

Native evaluation unions the actual boxes and requires one connected cutter.
Every box must hit a chosen receiver and every receiver must lose material. Slab
footprints stay strictly inside the outline and cannot touch existing holes.
The complete union must cut through slab thickness. Empty results and splits of
receiving physical solids fail. Wall-opening/cutout volume overlaps require editing
the existing source. Layered walls preserve separate physical components and
source-owned face materials, including newly generated cut faces.

## MCP calls

`set_compound_cutout` and `preview_compound_cutout` take `expected_revision`, `name`,
`boxes` and `receivers`. Each box supplies `center: [Quantity, Quantity]`, `width`,
`depth`, optional `id`, `rotation_deg` (default 0), and optional `bottom`/`top`.
Each receiver supplies `part`, `producer` and optional generated `feature` ID.
Global `bottom`/`top` are optional; provide both limits or neither at either scope.
`from_level` optionally constrains selected floor slabs, with optional `to_level`.
It does not silently add or replace receivers; `to_level` requires `from_level`.

Omit `cutout` to allocate a new source ID. Supply an existing ID to edit, or a
proposed uncommitted ID returned by Preview to create it. Preserve box IDs and
receiver `feature` IDs from Preview for stable Apply identity. An existing receiver
automatically retains its generated ID; a conflicting supplied ID is refused.

Both calls return `revision`, `committed`, `cutout`, the canonical `definition`,
changed receiver outputs with exact volumes/counts, and `native_geometry_checked`.
Preview returns `committed: false` and leaves revision unchanged. The canonical
definition contains stored dimensions and links; it is not a copy of the MCP
Quantity input schema. Invalid native geometry or stale revisions do not commit.

`inspect_compound_cutout` takes optional `cutout` and returns `revision` plus
`cutouts: [{cutout, definition}]`; omission lists all. `remove_compound_cutout`
takes `expected_revision` and `cutout`, preflights restored receivers, then removes
the complete source and its generated cuts atomically.

The two source capabilities are `architecture.compound_cutout.1` and
`architecture.compound_cutout_receiver.1`. Readers that lack them must refuse the
file. They do not flatten the source to legacy rectangular holes.

Contract sources: [typed source](../../crates/multicad-core/src/model/compound_cutout/records.rs),
[transaction](../../crates/multicad-core/src/model/compound_cutout/prepare.rs),
[native validation](../../crates/multicad-model/src/compound_cutouts.rs),
[MCP arguments/results](../../crates/multicad-mcp/src/tools/compound_cutouts.rs).
See [ADR-0031](0031-compound-architectural-cutouts.md) for the ownership decision.
