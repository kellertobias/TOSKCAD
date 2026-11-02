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
CAD-246 adds a separate optional Architecture context to version 3 local recent
metadata: `architecture` is null before that context is initialized, or holds
`plan` (boolean) and `level` (UUID or null for World). The store reads version 1
visits and version 2 mode/workspace records and writes version 3. Other workspace
routes keep their existing meanings. See
[view recall](../../crates/multicad-ui/src/application/welcome.rs) and the
[recent-file store](../../crates/multicad-app/src/recent_projects.rs).

The representation and active level survive a local reopen; camera positions
remain session state and reopen uses automatic framing. A recalled level that
has been removed keeps its identity and produces a repair message. It never
silently becomes the first floor. An explicit World context stays World.
Malformed fields and nil level identities are refused without overwriting the
existing navigation file. Navigation persistence never writes to the `.tcd`
container or changes the canonical source or Undo history.
Changing modes before history loads preserves an unvisited Architecture context
without restoring the older mode. An explicitly chosen World context replaces
the previous floor, including when pending visits are coalesced.

The Model workspace offers linked 2D plan and 3D model representations. The
geometry worker derives wall cuts, hatching, door swings, window panes and stair
graphics through the existing native drawing evaluator. The temporary projection
does not create a drawing book or another building document. Wall cuts use the
evaluator's default plane, 1000 mm above each wall's base. Other objects retain
their native top-view geometry. The initial plan fit uses the visible X/Y extents
and retains the selected level elevation.

The detached plan graphics carry the evaluated source hash and canonical Part
identities. Level and visibility filters apply to both representations. In 2D,
wall and stair plan graphics replace their occluding mesh surfaces for painting;
the original native geometry remains the object-picking and reference source.
Physical face, edge and vertex picking belongs to the 3D representation. A saved
3D section plane is preserved while the plan uses its native wall cuts. A failed
plan projection produces a visible diagnostic in 2D and leaves valid 3D geometry
available. Switching representations does not request another model rebuild.

Pending modeling previews use level membership from their exact prepared
document, cached separately from canonical sidebar facts. The same filter stays
in effect after Apply while the canonical model rebuilds. Creating an opening
from Common preserves an explicitly selected wall and follows its level when
entering Architecture, keeping the host visible in a recalled plan view. Inside
Architecture, the selected active level
remains authoritative for new opening forms.

Level buttons remain available while a modeling form is open. They change the
visible floor without reassigning that form or any saved objects; the form's
level field remains its placement authority. New/Edit level and member selection
stay guarded until the pending operation is applied or canceled.

Saved doors and windows expose Pick location in viewport as well as their numeric
controls. A graphical move retains the opening identity and type within its
existing wall. Picks on another wall are refused; removal mode disables picking.
The move still requires native Preview before Apply and remains cancellable.

Fences use their existing optional canonical level attachment. A new run inherits
the active level; reopening a run preserves its saved attachment. The level
sidebar lists each run once and filters its native post and infill Parts through
that run's ownership. Plants have World placement rather than a floor attachment.
Starting Planting selects World explicitly; existing plants retain their saved
pose. World plants and level-owned fence runs can be reopened from their sidebar
entries through the existing forms.

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
