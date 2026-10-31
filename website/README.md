# ToskCAD project website

`index.html`, `style.css` and `site.js` form a static project page for GitHub Pages. There is no package installation or build step. Relative asset paths support the `/TOSKCAD/` project prefix and opening the file directly.

Open `website/index.html` in a browser to preview the page. JavaScript enables the screenshot switcher. Without JavaScript, all screenshots remain visible. Reduced motion disables entrances and transitions.

The palette uses `#4599D2`, sampled from the approved logo background, with related pale-blue surfaces and navy text. Gallery images have bounded display sizes beside workspace explanations; their links open the full-size originals.

## Assets and evidence

- `assets/logo.png` copies the approved `assets/branding/toskcad-logo-blueprint-tosk-c.png`.
- `assets/icon.png` copies `assets/branding/app/AppIcon.png`.
- `assets/cabinet.png`, `assets/workbench.png`, `assets/front-panel.png` and `assets/apartment.png` are fresh native macOS captures from October 31, 2026. Each project was opened through the rebuilt application's Examples browser, which copies the bundled project and dependencies to an editable user folder. The captures show the Wood, Weld, Sheet metal and Architecture workbenches respectively.
- The source projects are [`examples/cabinet.tcd`](../examples/cabinet.tcd), [`examples/welded-workbench.tcd`](../examples/welded-workbench.tcd), [`examples/front-panel.tcd`](../examples/front-panel.tcd) and [`examples/apartment.tcd`](../examples/apartment.tcd). Their descriptions and limitations are stored in the project files.

The capture build is `artifacts/development/build-xJt76z/MultiCAD.app`, produced by `./build` with the pinned toolchain and lockfile. Its executable SHA-256 is `8eb05c1ce6253656e43afb90af96806b41134c1a0e213c7e96fbaa63730c00a4`. It includes the requested FDM tab label/order change and pre-existing local native-build changes, so it is evidence from the local working tree rather than a clean release build.
All imagery belongs to this project; no stock photography or generated application screenshots are used. Keep captions and provenance current when replacing captures. The original local captures live under ignored `artifacts/`; the publication copies are tracked here.

## Deployment

The [Pages workflow](../.github/workflows/pages.yml) uploads only `website/`, then deploys using the GitHub Pages environment. The repository Pages build type must be GitHub Actions. Website deployment is independent of application releases and their Forgejo reconciliation gates.
