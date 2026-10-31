# ToskCAD project website

`index.html`, `style.css` and `site.js` form a static project page for GitHub Pages. There is no package installation or build step. Relative asset paths support the `/TOSKCAD/` project prefix and opening the file directly.

Open `website/index.html` in a browser to preview the page. JavaScript enables the screenshot switcher. Without JavaScript, all screenshots remain visible. Reduced motion disables entrances and transitions.

## Assets and evidence

- `assets/logo.png` copies the approved `assets/branding/toskcad-logo-blueprint-tosk-c.png`.
- `assets/icon.png` copies `assets/branding/app/AppIcon.png`.
- `assets/model.png` and `assets/parameters.png` copy `12-model.png` and `11-applied.png` from the native shared-workflow review. The [review notes](../docs/verification/native-shared-workflow-2026-10-29.md) describe the October 29, 2026 session and its limitations. These historical captures precede later layout changes.
- `assets/drawings.png` copies the October 31, 2026 native cabinet drawing review's `cabinet-overview.png`. It shows generated drawing pages, rather than a desktop application window.

All imagery belongs to this project; no stock photography or generated application screenshots are used. Keep captions and provenance current when replacing captures. The original local captures live under ignored `artifacts/`; the publication copies are tracked here.

## Deployment

The [Pages workflow](../.github/workflows/pages.yml) uploads only `website/`, then deploys using the GitHub Pages environment. The repository Pages build type must be GitHub Actions. Website deployment is independent of application releases and their Forgejo reconciliation gates.
