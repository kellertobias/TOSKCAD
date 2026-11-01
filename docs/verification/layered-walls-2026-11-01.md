# Layered-wall verification checkpoint

CAD-204 is in progress. This checkpoint covers canonical source, exact native
geometry and reference guards on macOS. It does not establish actual macOS editor interaction,
shared Library integration under CAD-206, or human acceptance. Native UI-control tooling is
unavailable; CLI tests are not a substitute for those graphical checks.

## Source and compatibility

The existing wall-type catalog retains uniform schema 1 and adds layered schema
2. A layered wall has explicit placement/reference and flip data under wall
schema 2. Older readers reject the new capabilities rather than flattening the
construction. Coatings have zero volume, cavities have real width without solid
material, and finite solid layers retain their own material links and identities.
The total is derived. Conflicting totals/material assignments, missing reference
layers and disguised schema-1 placement fields are rejected.

Source tests cover actual TCD persistence, cancellation, history, parameter
identity after rename, unit conversion, layer reordering and duplication,
physical mounting faces, opening/chase anchor repair, open-path reversal and
incoming-reference guards. Existing uniform wall and equipment contracts remain
covered separately.

## Native geometry

The fixture has five finite layers, 80/120/40/100/12.5 mm, a 30 mm internal air
cavity and a zero-thickness coating. Total construction width is 382.5 mm;
finite material widths sum to 352.5 mm. Native component identities come from
exact source/topology ancestry, not solid enumeration order.

| Native case | Independently checked result |
| --- | --- |
| 4000 × 2700 mm straight run; 900 × 2100 mm door and 500 × 1000 mm window | Five real material solids; volume 2,964,525,000 mm³; each finite thickness and layer volume measured; no coating/cavity solid |
| Centered L path, 4000 + 3000 mm, height 2700 mm | Disjoint material bands; volume 6,658,402,500 mm³; flipped construction 6,666,097,500 mm³, including the cavity's changed corner position |
| Closed 4000 × 3000 mm rectangle with Outside reference | Same material bounds and volume for both windings; volume 11,883,712,500 mm³ |
| 400 mm wide, 100 mm deep full-height chase | Cuts the outer 80 mm layer and 20 mm of the next; remaining volume 3,699,000,000 mm³; two fragments retain the first layer identity; exact back plane at depth 100 mm |
| STEP write/read of the opened straight wall | Five exact solids, matching volume and bounds |
| Below-minimum finite layer / unsupported exterior-cavity chase | Explicit failure; no inflated layer or flattened wall fallback |

Native common-volume checks reject overlapping material bands before compound
publication. A dedicated guard test distinguishes touching and overlapping
solids and observes cancellation. Component extraction additionally verifies
count/volume conservation, independent owned lifetimes, STEP geometry, stable
identity through reorder and unchanged parent topology. Compound construction
and extraction check cooperative cancellation between native calls.

## Evidence and remaining work

Logs and a scope manifest are retained in
`artifacts/layered-wall-review-20261101/`. Focused results at this checkpoint:
17 layer/source tests plus four existing equipment tests; six native layered-wall
tests; four existing native wall tests and three vertical-cutout tests; one exact
overlap-guard test; three component tests. These counts overlap across repeated
logs and must not be added as a unique total.

Formatting and strict all-target workspace Clippy pass for this geometry increment
(Clippy: 7m36s). Required final feature-wide workspace tests remain pending. Native geometry
checks above do not demonstrate the graphical type editor, the complete
T-junction contract, or cross-project pinned
wall-type updates. Windows/Linux CI and actual macOS interaction remain separate
evidence. No push, installation or publication is part of this checkpoint.

## Material publication increment

Exact face provenance now supplies finite-layer material and source width.
Unknown, incomplete or ambiguous ownership fails explicitly. Cached evaluation
runs the same ownership check. Shared material styles resolve directly without
creating temporary Parts or inheriting unrelated Part stock/appearance overrides.
The viewport has a validated immutable face palette and preserves native picking.

Native export tests pass for the five-layer fixture at 1:100, 1:50 and 1:10.
Every finite-layer boundary is independently measured from both PDF drawing
commands and parsed DXF lines. Hatches use the layer material's configured weight
and stay outside the air cavity. Layers under 0.5 mm on paper omit hatching,
keep true boundaries with fine cut lines and produce a larger-detail warning.
The initial 0.5 mm cut weight obscured thin finishes; inspection of a Poppler
render led to the scale-aware correction. The retained 300 dpi crop shows the
corrected 1:100 output. This is exported-sheet review, not native GUI acceptance.

Focused checks: seven native layered-wall tests, four existing wall tests and
three vertical-cutout tests and two drawing-export tests pass; 13 core material
and 17 core layer tests pass; the face-palette
validation test passes. Both offscreen hardware GPU tests and the shader validator
pass (three tests), covering material colors, instance fallback, selection,
section clipping, edges and cached meshes. Native fixtures and PDF/DXF/SVG files
are in `artifacts/layered-wall-review-20261101/material-plans/`. Formatting and strict all-target workspace Clippy pass for this increment;
final feature-wide workspace tests remain
pending. The editor/shared-command increment below follows this checkpoint;
separate-wall T joins, Library integration under CAD-206 and actual macOS
interaction are still outstanding.

## Construction editor and shared commands

Atomic shared construction editing is committed in `7d169bc`. The same typed
source transaction serves the native wall form and MCP. Source tests cover the
affected-user list, shared changes with retained instance height overrides,
Cancel, one-step Undo/Redo and boundary removal with an atomic current-wall
repair. Another wall's unresolved boundary refuses the entire change. The
complete core suite passes: 455 tests, zero failures, 61 result groups.

The native form now includes the proportionate layer diagram, labeled coatings
and air, band/row selection, add/remove/duplicate/reorder, material/dimension
controls, derived total, construction and instance references, and mirrored flip.
Shared editing lists existing affected walls; duplication makes an independent
type in the existing catalog. Immutable source stamps and evaluated snapshots
avoid idle hashing and parameter reconstruction.

Six focused UI tests pass. They inspect actual egui paint output and send pointer
events to material bands and the coating row, verify the effective instance
reference marker and mirrored bands, and check source refresh across distinct
Sessions with identical document ID/revision. Native form preparations produce
real layered solids, preserve shared versus duplicated scope and instance IDs,
and save/reopen as editable TCD source. Both uniform and layered rooms use the
existing viewport path-click handler and native preview, with local redraw
cancellation restoring the original closed path. These are headless widget and
native service checks, not a macOS window authoring demonstration.

Four MCP integration tests pass, including the new layered workflow. It checks
native finite-solid counts/volume, retained IDs and bound dimensions after a
parameter rename plus name reuse, duplicate-ID refusal, retained placement,
shared type plus boundary repair in one edit, cancellation and read-only access.
Construction inspection returns editable tool input and full bound source;
unused-type creation explicitly makes no solid claim. Existing native layered,
wall and cutout regressions pass: 14 tests across three groups. Formatting and
strict whole-workspace/all-target Clippy pass (7m27s). Logs are retained in
`artifacts/layered-wall-review-20261101/`; full final feature workspace tests
remain pending. Separate-wall T joins, termination policy, CAD-206 integration,
actual graphical checks and human acceptance remain outstanding.

## T junctions and termination

Explicit joined starts and ends now use the same canonical layer/source model.
The supported rule is perpendicular flush-butt contact: the host remains
continuous and the finite branch layers end at its physical face, without
wrapping. Exact native tests cover both ends, a closed-room host, an exterior
air cavity requiring a 30 mm extension, and rotation/translation of the whole T.
They measure each material member's thickness, length and volume, preserve layer
UUID ownership and verify contact with zero common volume. Independent STEP
write/read retains the component count, volume and contact. An otherwise valid
T whose branch crosses another part of the host is refused for material overlap.

A five-layer T fixture also has a host door and branch window. Native volumes
match the cut areas. PDF drawing commands and independently parsed DXF lines
preserve the physical material contact at 1:100, 1:50 and 1:10; the door swing is
emitted once across the material components. Poppler-rendered 1:50 and 1:10
sheets were inspected. Native TCD/PDF/DXF/SVG files are retained in
`artifacts/layered-wall-review-20261101/t-junction/`. This exported review does
not replace native-window interaction.

Core source tests cover explicit alignment without source mutation, capability
refusal by older readers, canonical host links, topology-change repair, affected
joined branches, unsupported angle/datum/corner/height/duplicate-end cases,
host voids, branch end-opening limits and trimmed equipment anchors. Equipment
void checks also distinguish different segments of the same wall. The complete
core suite passes: 461 tests, zero failures, 62 result groups.

Seven UI tests pass, including pointer clicks on the actual joined-end controls
and native preview of their aligned result. Five MCP tests pass, including
alignment, retained joins during flip, stale topology and wrapping refusal, and
explicit detachment. Native model regressions pass: 12 layered-wall tests plus
four existing wall and three cutout tests. The production application's scene
publisher also passes a live per-layer material-color/source-update check. These
headless/native checks create no desktop windows.

Formatting and strict whole-workspace/all-target Clippy pass for this increment
(Clippy: 6m16s). The final full workspace test run is active on the frozen source;
no pass is claimed yet. Actual macOS editor interaction, CAD-206 cross-project
path/context/Library integration and human acceptance remain outstanding.
