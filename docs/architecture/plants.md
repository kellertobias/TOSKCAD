<!-- Kind: reference; Audience: native CAD and drawing maintainers; Purpose: explain planting source and output contracts; Non-goals: botanical advice, growth simulation or human visual acceptance. -->
# Trees, shrubs and planting plans

Planting lives in Model → Architecture. The Planting tool creates generic trees or shrubs by clicking the garden plane. Height, outer canopy/spread, position, elevation, rotation, name, size meaning and an optional plan label belong to the same preview and Apply transaction. Cancel discards the candidate. Existing plants can be selected in the planting inspector; ordinary Move / Copy also retains their linked source and settings.

The generic tree defaults to 6 m height and 4 m canopy spread, with a narrow trunk and cylindrical crown. The shrub defaults to 0.8 m height and 1 m spread. These simple native envelopes provide spatial context. They do not claim species dimensions, growth prediction, foliage detail or a real vegetation material volume. Users explicitly choose whether dimensions describe planned/mature size or current size.

## One retained definition

`architecture.plant_template` schema 1 identifies a symbol component, height and spread parameters and its authored plan sketch. All four are typed canonical references; dimensions must be length parameters. A pinned symbol captures and remaps that metadata with its model, parameters and sketch. `architecture.plant_instance` schema 1 refers to the canonical symbol ownership record and stores size meaning, optional label and plan visibility. It contains no duplicate dimensions or placement. Duplicate owners, mismatched links, non-plant targets and invalid labels are rejected.

Projects containing direct or embedded planting sources require `architecture.plant.1`. An older reader must reject that required capability. No network lookup supplies geometry or updates an existing pin. Generic tree and shrub presets use the existing included Library, retain the same pins as the creation tool, and remain editable through ordinary symbol inputs.

## Drawing policy

A horizontal top projection uses the solved authored plan at the plant's current occurrence pose. The tree has two concentric rings; the shrub has one. The outer circle is the actual canopy/spread, so 4 m prints as 40 mm at 1:100 and 80 mm at 1:50; 1 m prints as 10 mm and 20 mm. Rotation, translation and dimension edits use the same canonical parameter and occurrence sources as the 3D model.

Plan symbols replace canopy/trunk hidden-line projection. Transparent outlines are appended after building and path geometry in stable instance order, with 0.18 mm paper strokes in PDF/SVG and the native canvas. Drawing DXF retains the `VIEW_VISIBLE` role layer for downstream plot styling. A crown above the architectural plan cut remains represented. Optional labels use 3.5 mm paper text through the common page renderer and native canvas. SVG, PDF and drawing DXF consume that same checked page geometry. Hiding a planting footprint does not hide the physical envelope in sections or elevations. Tilted plants, bottom views and physical sections retain normal solid projection.

The canopy-spread convention follows the scale-planning principle described in the University of Florida's [Drawing a Planting Plan](https://ask.ifas.ufl.edu/publication/EP456). The generic geometry and circles are authored in this repository; no illustration or botanical catalog is copied.

## Developer evidence

`planting_review` generates a native garden project with overlapping trees, shrubs, building/path context and pages at 1:100 and 1:50. `planting_exports` independently parses PDF paths and drawing-DXF line entities to measure both outer footprints and inner tree rings. Core, model, MCP and UI tests cover transactions, invalid input, exact vectors, visibility, pose, copy settings, pins and history. These tests do not replace interactive macOS verification or human acceptance. See [the ownership decision](0027-linked-planting-symbols.md) and [the native checkpoint](../verification/planting-2026-11-01.md).
