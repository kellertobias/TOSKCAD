# ToskCAD project website

`index.html`, `style.css` and `site.js` form a static project page for GitHub Pages. There is no package installation or build step. Relative asset paths support the `/TOSKCAD/` project prefix.

The four examples appear in alternating image-and-description sections. On smaller screens, the main image sits above its description. Clicking a project image opens an in-page preview with zoom, previous/next controls and arrow-key navigation. Escape closes the preview and returns focus to the image link. Without JavaScript, the links open the original files. Reduced motion disables entrance animations.

The palette uses `#4599D2`, sampled from the approved logo background, with pale-blue surfaces and navy text. The five mode swatches use the native colours in [`registry.rs`](../crates/multicad-ui/src/registry.rs).

## Assets and evidence

- `assets/logo.png` copies the approved `assets/branding/toskcad-logo-blueprint-tosk-c.png`.
- `assets/icon.png` copies `assets/branding/app/AppIcon.png`.
- `assets/workbench.png`, `assets/cabinet.png`, `assets/front-panel.png` and `assets/apartment.png` are native macOS captures from October 31, 2026. They show the Weld, Wood, Sheet metal and Architecture workbenches. The cursor was parked over the application title, outside the model and drawing content.
- Captures use editable copies under ignored `artifacts/website-focus/`, with cameras saved through the native MCP API. The cabinet uses its saved “Doors and drawers open” pose. The panel uses the native assembly preview with zero separation to display the grey aluminium and black PETG materials. The ordinary aggregate assembly view currently displays a uniform colour.
- `assets/workbench-weld-plan.svg`, `assets/cabinet-connector-plan.svg` and `assets/apartment-plan.svg` are vector drawing exports evaluated from the native model, not traced screenshots. The weld sheet uses a copy of the drawing with spaced callouts, compact weld identifiers and a schedule without the long notes column. The cabinet sheet shows the side, hinge and runner drilling positions. The apartment sheet is the model-derived floor plan with wall cuts, door swings and evaluated dimensions; it is not a separately authored constrained sketch.
- The source projects are [`examples/welded-workbench.tcd`](../examples/welded-workbench.tcd), [`examples/cabinet.tcd`](../examples/cabinet.tcd), [`examples/front-panel.tcd`](../examples/front-panel.tcd) and [`examples/apartment.tcd`](../examples/apartment.tcd). The apartment source clears the bedroom approach by turning the sofa along the left wall and moving the coffee table. The panel's end parts are tagged for print and assigned PETG, with separate print export sets.

The capture application is `artifacts/development/build-XnuMFV/MultiCAD.app`, produced by `./build` with the pinned toolchain and lockfile. Its executable SHA-256 is `94851a7d7c4491cab168b2b924f6d3cc77dfe426513ad6e6e4985f71ef8e9734`. It includes local drawing-preview and title-block changes and is evidence from the working tree, not a clean release build. Model edits in the capture copies use the same typed transactional document API as the application.

Regenerate a drawing with `cargo run -p multicad-app --example website_plan --locked -- examples/welded-workbench.tcd WB-102 website/assets/workbench-weld-plan.svg`. The same command accepts the cabinet page `C-103` and apartment page `AP-101`. The exporter refuses unresolved views or detached dimensions.

All imagery belongs to this project. No stock images, synthetic application screenshots or substituted geometry are used. Keep captions and provenance current when replacing captures.

## Deployment

The [Pages workflow](../.github/workflows/pages.yml) uploads only `website/`, then deploys using the GitHub Pages environment. The repository Pages build type must be GitHub Actions. Website deployment is independent of application releases and their Forgejo reconciliation gates. CSS and script URLs carry a revision query to refresh cached gallery behaviour.
