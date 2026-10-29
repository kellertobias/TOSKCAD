# Drawing guides

A drawing guide records floor plan, section or elevation intent in the
model. Generating pages turns it into ordinary drawing views that keep a link
back to the guide, so level and guide edits flow into the drawings instead of
being re-entered after export. The code is
`crates/multicad-core/src/drawings/guides.rs`, with native resolution in
`crates/multicad-model/src/drawings.rs`.

## Source

A guide is a `drawings.guide` extension entity. It holds:

- **Kind.**
  - **Floor plans.** A cut height above each level's floor and a view depth
    below the cut. One plan is generated for every listed level.
  - **Section.** A plan line from a start point to an end point, and an
    optional depth behind the cut. The view looks across the line toward its
    left side, and material on the right side is removed. The cut is limited
    to the line's length. When levels are listed, the section is limited
    vertically from the lowest slab bottom to the top of the highest level.
  - **Elevation.** An uncut exterior view looking in a plan direction, in
    degrees from +X (0 looks toward +X, 90 toward +Y).
- **Levels.** Required for floor plans, optional for sections.
- **Sources.** The Parts, Instances or assemblies shown.
- **Scale.** The drawing scale used for new views.

Cut height, view depth and section points can be parameter expressions. The
view range is stored explicitly; nothing is inferred from a screenshot.

## Generated views

A generated view is an ordinary `DrawingView` with a `guide` link naming the
guide and, for a plan, its level.

- **Live link.** During native evaluation a guided view takes its frame, cut
  range, window and sources from the current guide and levels. Raising a
  level moves its plan cut and widens the section window without
  regenerating.
- **Generation.** `prepare_guide` creates or edits the guide and, when asked,
  creates or updates its views in the same transaction.
  - Without a named book, it reuses the book that already holds the guide's
    views, or creates a new one.
  - Each new view gets its own A3 landscape page at the guide scale.
  - A rerun updates the matching views in place, keeping their identity,
    page position and scale, and never duplicates them.
  - A view whose level was removed from the guide is reported as stale and
    left in place.
- **Diagnostics.** A view whose guide or level was deleted, or whose level is
  no longer part of the guide, is reported as unresolved instead of drawn
  from an old frame.
- **Section styles.** Plans and sections are section views, so cut faces are
  hatched with each Part material's section style.
- **Copies.** Duplicating a page drops the guide link from the copies; only
  the original follows the guide.

Projects that use guides declare `drawings.guide.1`, and books with guided
views declare `drawings.guided_views.1`.

## Viewport

After a full evaluation, the model snapshot carries each guide's outline,
sized to the final model bounds plus 500 mm:

- each plan cut plane, as a horizontal rectangle;
- the section line's cut, as a vertical rectangle;
- the elevation's picture plane on the viewer's side.

The viewport draws them as dashed cyan loops. The camera tools menu has a
**Drawing guides** toggle.

## Access

- **Window.** Model → Create → **Levels & stairs** → **Drawing guide**.
  - Choose floor plans, a section or an elevation, then the levels, shown
    Parts and scale.
  - An existing guide can be chosen to edit it.
  - With **Generate or update the drawing pages on Apply**, Apply also
    writes the linked views.
- **MCP.** `set_drawing_guide` creates or edits a guide and, with
  `generate_pages`, reports the created, updated, stale and unresolved views.

## Limits

- **No automatic membership.** Sources are listed explicitly; a new Part is
  not added to existing guides.
- **Section shapes.** A section is one straight line; jogged sections and
  interior elevations with a depth limit are not supported.
- **View inspector.** The drawing view inspector shows a guided view's
  stored frame until the guide is rerun, and frame or cut edits made there
  have no effect while the view is linked. Evaluated vectors and exports
  always use the live guide.
- **Stale views.** Stale views are reported, not removed automatically.
