# Parametric machine screws

CAD-235 extends the existing `model.mechanical_part` provider with one retained
`MachineScrew` recipe. The native UI, MCP tools, Library templates and
manufacturing evaluators use that same canonical source. Runtime geometry does
not look up dimensions in an installed catalog.

## Geometry and mounting convention

`underhead_length` measures from the underhead bearing plane to the tip.
`thread.length` is the threaded extent; their relationship derives full versus
partial threading. `shank_diameter` describes only the plain section. There is
no second threaded-length field or persisted full-thread boolean.

`thread.frame` locates the whole screw. Its origin is the underhead plane and
its positive normal points toward the tip. In local coordinates:

| Component | Axial extent |
| --- | --- |
| Head | `[-head_height, 0]` |
| Plain shank, when present | `[0, underhead_length - thread.length]` |
| Shared modeled external thread | `[underhead_length - thread.length, underhead_length]` |

The native builder composes these local solids and applies the world frame
once. It does not fuse a full-length cylinder beneath the thread grooves.
Named component edge/cap roles remain distinct through Booleans, and the
result must retain one valid solid and complete face ancestry.

Head form and drive are separate typed enums. Supported pairs are a circular
socket-cap head with a real six-sided recess, or an external-hex head driven
by its flats. A socket's across-flats value is the actual cavity dimension,
not a nominal key label. Corner-wall checks use `AF / sqrt(3)`; axial checks
retain a positive floor. Native geometry policy additionally governs resolvable
dimensions. Impossible lengths, head/drive pairs, walls, depths or shared
thread parameters refuse the candidate before commit.

The socket is flat-ended, with depth measured from the head top. Socket-bottom
tooling geometry, mouth treatment, underhead fillets, edge chamfers, surface
texture and imperfect-thread transitions are omitted. The model uses sharp
flat thread ends. These are declared modeling boundaries, not complete
standardized manufacturing detail or strength certification.

## Canonical source, references and readers

Every authored dimension can bind a project parameter. Head, drive, shank,
underhead length and nested thread dimensions all enter the existing dependency
closure. The optional provenance snapshot stays literal and self-contained.
Ordinary edits keep that source reference and report whether physical and
discrete head/drive/profile intent still match. Pose and display representation
do not change the dimensional reference basis.

Screw source requires `model.mechanical_part.machine_screw.1`, including
suppressed features and template-only imported definitions. The existing
provider schema stays at version 1 with this conditional reader gate. Older
washer, nut and bearing source keeps its existing fields, geometry and required
capabilities. Literal provenance remains outside identity remapping; actual
parameter bindings and output identities use the audited provider path.

Cosmetic threads remain display envelopes. The nested screw representation is
included in the existing producer-history manufacturing guard, so STEP, mesh
and print paths cannot export it as modeled thread geometry. Switching to
Modeled requires native evaluation of the shared helical builder.

## Sourced presets

[Curation](../../library/machine-screws-curation.json) records nineteen discrete
metric rows, explicit diameter/underhead/thread lengths and status for all
twelve requested nominal sizes. It also records original source hashes,
primary cross-checks, limits and declared adaptations. Material and grade are
unspecified. Missing rows mean no preset in this selected matrix; users can
author custom geometry without receiving catalog attribution automatically.

Socket rows retain HPC 2025's stated outer-diameter envelopes and explicit
threaded lengths. Surface treatment is unspecified: numeric agreement with a
knurled maximum does not establish a knurled or plain product variant. Actual
socket AF midpoints are engineering targets within the separate NBK
ISO 4762:2004 / JIS B 1176:2015 ranges. Minimum depth and remaining-floor
dimensions remain explicit. External-hex rows use Norelem's selected DIN 933
references, with the separately dated Nufast cross-check. DIN M10 AF17 is not
interchanged with ISO 4017 AF16.

The [source notice](../../library/sources/bolts/MACHINE-SCREW-NOTICES.md) records
LGPL originals, modifications and omissions. The developer-only Library
generator renders native previews and preserves all existing files/pins outside
the selected family. Its screw mode is documented in the [Library guide](../../library/README.md).

## Native numerical checks

The helix builder retains the established continuous representation when it
passes physical and volume-convergence guards. A typed ridge-loss or volume
integration failure retries bounded per-turn faces on the same helix. Integer
span indices avoid accumulated endpoint roundoff; only machine-scale remainder
is merged into the preceding edge, and the exact requested endpoint is kept.
Both attempts retain validity, connectedness, physical crest/gap, ancestry and
the unchanged Gauss–Kronrod convergence guard.

New complex-screw metrology tests compare analytic and STEP volumes using the
declared geometry/integration uncertainty: `epsilon * surface_area + r * |V|`
per measured solid, summed for a round trip. `epsilon` is the native linear
tolerance and `r` is the existing `1e-7` convergence limit. Independent bounds,
socket/flats/floor and thread-material probes accompany those measurements.
Existing legacy absolute thresholds, including the M6 nut's `1e-5 mm³`
round-trip bound, remain unchanged. A numerical bound is not a fit class or
physical fastener acceptance test.

MCP creation, source import, placement and owner-override configuration are
described in the [MCP reference](mcp-server.md). Imported definitions keep pins;
configuration uses exposed source parameter IDs. Direct edits to symbol-owned
features remain refused, and native preflight is reconciled against the current
owner session before review or commit.
