# ToskCAD project website

`index.html`, `style.css` and `site.js` form a static project page for GitHub Pages. `releases.html` and `releases.mjs` list native downloads from GitHub Releases. There is no package installation or build step. Relative asset paths support the `/TOSKCAD/` project prefix.

The four examples appear in alternating image-and-description sections. On smaller screens, the main image sits above its description. Clicking a project image opens an in-page preview with zoom, previous/next controls and arrow-key navigation. Escape closes the preview and returns focus to the image link. Without JavaScript, the links open the original files. Reduced motion disables entrance animations.

The palette uses `#4599D2`, sampled from the approved logo background, with pale-blue surfaces and navy text. The five mode swatches use the native colours in [`registry.rs`](../crates/multicad-ui/src/registry.rs).

## Assets and evidence

- `assets/logo.png` copies the approved `assets/branding/toskcad-logo-blueprint-tosk-c.png`.
- `assets/icon.png` copies `assets/branding/app/AppIcon.png`.
- `assets/workbench.jpg`, `assets/cabinet.jpg`, `assets/front-panel.jpg` and `assets/apartment.jpg` are native macOS window captures from November 9, 2026. They show the Metal, Wood, Metal and Architecture workbenches, respectively. The JPEGs are 1360 × 860.
- The captures use editable copies under `artifacts/website-focus/`. The cabinet uses the saved “Doors and drawers open” pose and an angled front view. The workbench shows the weld callouts in Model, and the panel shows the aluminium sheet and black PETG end parts.
- `assets/workbench-weld-plan.svg`, `assets/cabinet-connector-plan.svg` and `assets/apartment-plan.svg` are vector drawing exports evaluated from the native model, not traced screenshots. The weld sheet uses a copy of the drawing with spaced callouts, compact weld identifiers and a schedule without the long notes column. The cabinet sheet shows the side, hinge and runner drilling positions. The apartment sheet is the model-derived floor plan with wall cuts, door swings and evaluated dimensions; it is not a separately authored constrained sketch.
- The source projects are [`examples/welded-workbench.tcd`](../examples/welded-workbench.tcd), [`examples/cabinet.tcd`](../examples/cabinet.tcd), [`examples/front-panel.tcd`](../examples/front-panel.tcd) and [`examples/apartment.tcd`](../examples/apartment.tcd). The apartment source clears the bedroom approach by turning the sofa along the left wall and moving the coffee table. The panel's end parts are tagged for print and assigned PETG, with separate print export sets.

The capture application is the optimized macOS inspection build in `artifacts/qa/ToskCAD-Inspection.app`. The signed executable has SHA-256 `c52bd8f16d5c3bd369ae6a03e3fb7cdb7da499c2724d838e7203970d076fa310`.

Regenerate a drawing with `cargo run -p multicad-app --example website_plan --locked -- examples/welded-workbench.tcd WB-102 website/assets/workbench-weld-plan.svg`. The same command accepts the cabinet page `C-103` and apartment page `AP-101`. The exporter refuses unresolved views or detached dimensions.

All imagery belongs to this project. No stock images, synthetic application screenshots or substituted geometry are used. Keep captions and provenance current when replacing captures.

## Deployment

The [Pages workflow](../.github/workflows/pages.yml) uploads only `website/`, then deploys using the GitHub Pages environment. The repository Pages build type must be GitHub Actions. Website deployment is independent of application releases and their Forgejo reconciliation gates. CSS and script URLs carry a revision query to refresh cached gallery behaviour.

## Release downloads

The download page fetches the public releases API for `kellertobias/TOSKCAD`, showing up to six published semantic releases with their platform binaries, sources and SHA-256 links. It only accepts asset URLs belonging to that exact repository and release. A platform download appears when its runtime ZIP, source ZIP and checksum are all present. Missing releases, API failures and request timeouts leave the direct GitHub Releases link available; failures also offer Retry. No token is needed or included.

Run `node --test tools/website-releases.test.mjs` from the repository root to check release filtering and asset selection. The same checks run with `npm test`.
