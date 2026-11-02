# ToskCAD project website

`index.html`, `style.css` and `site.js` form a static project page for GitHub Pages. `releases.html` and `releases.mjs` list native downloads from GitHub Releases. There is no package installation or build step. Relative paths support the `/TOSKCAD/` project prefix.

Five bundled projects appear in alternating image-and-description sections. On smaller screens, each main image sits above its description. Clicking a project or hero image opens an in-page preview with zoom, previous/next controls and arrow-key navigation. Escape closes it and returns focus to the image link. Without JavaScript, links open the original files. Reduced motion disables the entrance and logo animations.

The palette uses `#4599D2`, sampled from the approved logo background, with pale-blue surfaces and navy text. The five named mode fields use the native colours in [`registry.rs`](../crates/multicad-ui/src/registry.rs). The copy describes the actual workbenches and references in each example; the front panel has illustrative hardware, without a KiCad import.

## Assets and evidence

The native screenshots were captured on macOS on November 1, 2026. Each shows an editable copy of a bundled project, with the cursor parked in an empty header area, clear of the application name, controls and model.

| Assets | Source and capture |
| --- | --- |
| `logo.png` | Approved `assets/branding/toskcad-logo-blueprint-tosk-c.png`. |
| `icon.png` | Approved `assets/branding/app/AppIcon.png`. |
| `logo-monochrome.png` | Approved `assets/branding/toskcad-logo-titleblock.png`, used on a white footer. |
| `workbench.jpg`, `workbench-vise.jpg` | Updated 32-part bench, captured in Weld and Common. |
| `drill-guide.jpg`, `drill-guide-print.jpg` | Native 13-part guide in Common; Print preview selects its four tagged printed parts and excludes metal hardware. |
| `cabinet.jpg` | Wood workbench, angled view with the saved “Doors and drawers open” pose. |
| `front-panel.jpg` | Sheet metal workbench, native assembly preview at zero separation to show grey aluminium and black PETG. The ordinary aggregate assembly view currently uses a uniform colour. |
| `apartment.jpg` | Refreshed apartment in Architecture with its linked bench and cabinet, using a 1.2 m display section to show the furniture. The cut is uncapped and leaves source geometry unchanged. |

All seven raster captures use the native executable from the successful [macOS package job](https://github.com/kellertobias/TOSKCAD/actions/runs/41280782003/job/124772142089) at source commit `0562e39bf798694ab678d0baa950d01c66370994`. The capture copy has a separate bundle identifier and a new ad-hoc signature so input stays separate from other running ToskCAD windows. Its executable SHA-256 is `05cceb75cb46ec4a2cefaee4908c98e7878e00b30cf6d0130b827a20aad9c1f7`. Application code and bundled project files are unchanged. This locally signed capture copy is not a distributed release artifact.

Capture copies live under ignored `artifacts/website-revision/captures/` and `artifacts/website-focus/`, with cameras saved through the native MCP API. The sources are [`examples/welded-workbench.tcd`](../examples/welded-workbench.tcd), [`cabinet.tcd`](../examples/cabinet.tcd), [`front-panel.tcd`](../examples/front-panel.tcd), [`drill-guide.tcd`](../examples/drill-guide.tcd) and [`apartment.tcd`](../examples/apartment.tcd). The apartment's references were explicitly refreshed while retaining its project, room and placement identities. Its sofa and coffee table leave the bedroom approach clear.

The print package preserves model coordinates, including the guide fence below Z=0. The slicer must orient and arrange the parts. Export verification does not establish printability or a fit for every printer.

The SVG sheets are evaluated native drawing exports:

- `workbench-weld-plan.svg`: canonical portrait sheet with 22 numbered weld callouts and a compact weld schedule.
- `workbench-assembly-plan.svg`, `workbench-top-plan.svg`, `workbench-frame-cut-list.svg`: WB-101, WB-103 and WB-104, including the drilled top, machining schedule, member cuts and BOM. The top is a drawing sheet, not a CNC contour file.
- `cabinet-connector-plan.svg`: panel views with shelf-pin rows, hinge cups and runner screw positions.
- `cabinet-cut-list.svg`, `cabinet-assembly-plan.svg`: C-107 panel cutting list and C-101 assembly views.
- `drill-guide-plan.svg`: top and front views with associative overall dimensions.
- `apartment-plan.svg`: floor plan with wall cuts, door swings and evaluated dimensions. It is generated from the model, rather than authored as a separate constrained sketch.

Regenerate a sheet with `cargo run -p multicad-app --example website_plan --locked -- examples/welded-workbench.tcd WB-102 website/assets/workbench-weld-plan.svg`. The command accepts any page number in the project, including cabinet pages `C-101`, `C-103` and `C-107`, drill-guide page `DG-101` and apartment page `AP-101`. The exporter refuses unresolved views or detached dimensions and leaves the input unchanged.

The workbench and cabinet schedules are shown in the drawing sheets above. The weld plan includes 22 numbered welds in the authored sequence. Keep these examples visual; CSV files are not published on the website.

WB-103 also carries explicit grid callouts from the evaluated `list_parameters` output in `workbench-parameters.json`: 20 mm through holes, 80 mm pitch and 390/140 mm edge offsets. After regenerating the native sheet, run `node tools/website-bench-plan.mjs website/assets/workbench-top-plan.svg website/assets/workbench-parameters.json website/assets/workbench-top-plan.svg`. It adds readable parameter values in the sheet's unused area without changing the exported geometry.

The front-panel drawing uses a different route because this release has no flat-pattern producer in Drawings. `front-panel-flat-pattern.json` is the native `unfold_sheet` result for the actual sheet Part in `examples/front-panel.tcd`; `front-panel-developed.dxf` comes from `export_sheet_dxf` for that same Part. `front-panel-flat-plan.svg` presents those developed contours on a dimensioned sheet. It is a website drawing of native geometry, not a screenshot or evidence of an implemented flat-pattern drawing tool. It preserves the display opening, fourteen circular holes and two bend lines. Bend IDs follow model feature order, not a press-brake sequence.

Regenerate that presentation with `node tools/website-flat-plan.mjs website/assets/front-panel-flat-pattern.json website/assets/front-panel-flat-plan.svg`. The developer-only renderer accepts closed straight contours and full circles, and rejects unsupported or inconsistent input. `node --test tools/website-flat-plan.test.mjs` checks the actual blank against independent bend-allowance and area calculations, and checks failure cases. No Node.js runtime is added to the application.

All imagery belongs to this project. No stock images, synthetic application screenshots or substituted geometry are used. Keep captions and provenance current when replacing captures.

## Preview and verification

Serve `website/` with a local static HTTP server. The copy received a humanizer pass, and the page was reviewed at desktop, tablet and phone widths. Evidence lives in ignored `.taste-skill/redesign/`. The skill's official probe and judges run through passive local preview instrumentation; browser interactions and screenshots use CUA. That instrumentation is not published.

## Deployment and downloads

The [Pages workflow](../.github/workflows/pages.yml) uploads only `website/` and deploys through the GitHub Pages environment. The repository Pages build type is GitHub Actions. Deployment is independent of the GitHub-native application release gates. CSS and script URLs carry a revision query to refresh cached assets and behaviour.

The downloads page fetches the public releases API for `kellertobias/TOSKCAD`, showing up to six published semantic releases with platform binaries, sources and SHA-256 links. It accepts asset URLs only from that repository and release. A platform download appears when its runtime ZIP, source ZIP and checksum are present. Missing releases, API failures and timeouts leave the direct GitHub Releases link available; failures also offer Retry. No token is needed or included.

Run `node --test tools/website-releases.test.mjs` to check release filtering and asset selection. The same checks run with `npm test`.
