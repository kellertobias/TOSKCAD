# Architecture command navigation

CAD-223 implements the owner's selected drill-down treatment in Model. The root
Create group offers Wall, Door, Window, Cutout, Levels and Stairs. Cutout retains
the existing wall/slab cutter; there is no separate Shaft tile. Stairs opens the
existing stair form directly and requires an explicit level before validation.

Entering Water, Electrical, Heating, Furniture or Landscaping moves the actual
parent controls into a 24-point compact cue. A single full tool row below it
starts with Back. Close in the compact cue also returns to the parent. This
preserves the chosen parent-row treatment; it does not replace the parent with
only a breadcrumb or add a permanent second full tool row.

| Category | Existing tools and families |
| --- | --- |
| Water | Sanitary fixtures; washing machine and dishwasher as appliances; shared Library |
| Electrical | Switch, Socket and Junction box first, then other outlets under Connections; luminaires and lamp holder under Lighting |
| Heating | Radiator and shared Library |
| Furniture | Cabinet, Panel, all appliance families and shared Library |
| Landscaping | Planting, Fence and shared Library |

The shared [catalog](../../crates/multicad-core/src/equipment/catalog.rs) remains
the authority for dimensions, host types and source generation. The
[navigation metadata](../../crates/multicad-ui/src/architecture_navigation.rs)
contains command IDs, labels and category membership. Every equipment command
carries a typed catalog Kind into the existing form. Preview and Apply retain the
same canonical transaction and native validation paths. The Library entries in
these categories open the existing included/user browser.

## Navigation state and routing

Category entry, switching, Back and Close change user-local navigation only.
They neither place a default object nor discard an open operation. Pending form
controls remain in the Inspector until Apply or Cancel. Unavailable modeling
buttons retain the normal pending-edit and native-backend guards.

Category state belongs to each open project. The existing recent-view workspace
string stores a subpath such as `Architecture / Electrical`; it is not added to
the canonical document or Undo. Existing `Architecture` records restore the root.
An unknown Architecture child restores its parent without executing a command.
Other workspace modes and the existing recent-file JSON fields are unchanged.
See [view recall](../../crates/multicad-ui/src/application/welcome.rs).

Finder and command entry use explicit family presets. The generic `equipment`
command remains available. `stairs` opens Stairs; `slab` still opens Levels.
`washer` retains its hardware meaning. Panel and Cabinet keep Wood as their
canonical command-entry home, while an explicit Furniture entry remains in
Architecture. [Application routing](../../crates/multicad-ui/src/application/architecture.rs)
keeps the selected category consistent with the tool that opened.

## Verification boundaries

[Painted pointer tests](../../crates/multicad-ui/src/application/architecture_tests.rs)
check parent-control movement, Back/Close, category switching and source-neutral
behavior at 1100 × 700 and 1360 × 860, including native 2× input scale. They check per-project and saved context,
explicit preset forms and preservation of a pending operation until Cancel.
Catalog tests require every canonical family to remain reachable and aliases to
resolve to real commands. Native catalog, MCP and export tests check the shared
Junction box source and existing families separately.

These checks do not establish actual macOS window interaction or human acceptance.
Model 2D/3D authoring and selectable levels belong to CAD-246, compound box
cutouts to CAD-247, Wood grouping to CAD-248 and the configurable Box assistant
to CAD-249. This navigation change does not implement those sibling features.
