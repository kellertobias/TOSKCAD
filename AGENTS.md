# MultiCAD development

## Scope and authority

- PLAINER is authoritative for work ownership, prerequisites, scope and review. Product plan: https://plans.tokenet.de/CAD/48.
- Read and claim the relevant item before implementation. Record real tests, commits, limitations and remaining work; agent completion is not human acceptance.
- Preserve unrelated changes. Make a separate Conventional Commit for each coherent implementation step. Push, remote creation, publishing and installation require user authorization; repository setup is local-only.

## Product boundaries

- The production application is native and primarily compiled Rust. No Python application runtime. Third-party native C++ dependencies and small audited adapters are allowed.
- `mockup/` is the offline HTML/SVG UX reference, not the production application. Preserve it unless the user explicitly requests mockup changes.
- CAD-49 must prove the kernel/solver/UI choices before CAD-50 integrates native dependencies and desktop packaging. Do not declare those gates complete from a scaffold or mockup test.
- Keep a single canonical parametric document and typed transactional API shared by the UI and MCP. Display meshes are not manufacturing geometry.
- Project keeps the parameter table, left-side icon-and-label actions for Apply and Rebuild, Discard Edit, Preview, Undo and Redo, and one file-metadata sidebar (name, description, preview, Add to Library). Project has no ribbon (CAD-220, product-owner revision 2026-11-01, superseding the 2026-10-27 Parameters ribbon). No modeling inspectors or parts sidebars in Project. Sketch editing is contextual within Model. The global Add Sketch action uses a selected valid planar face or requests a plane in the 3D viewer; existing sketches can reopen before editing tools appear. No separate Create Engraving entry. No Face Sketch command in Model.
- Top-level modes are Project, Model, Motion, Drawings and Export (CAD-221, product-owner revision 2026-11-01). Add Sketch and Import are global right-side actions beside Library, Measure and MCP history. Mechanical geometry stays in Model; assembly pose, relationship and interference controls belong in Motion. Export contains Common, Print and Render; materials are defined in Model.
- MCP provides external application control and attributable previews/history. Do not introduce built-in AI chat. FDM design aids belong in Model; print packaging belongs in Export.

## Verification

- Use the pinned Rust toolchain and committed lockfile. Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` for relevant Rust changes.
- Test observable behavior and failure cases. Do not claim a CAD workflow, valid manufacturing file, cross-platform runtime or performance result from a placeholder or source inspection.
- Node.js is developer/CI-only for prototype maintenance and release tooling, never a native runtime dependency. `node mockup/verify.mjs` checks illustrative UX behavior, not CAD correctness; `npm test` checks release contracts.
- GitHub is primary, as selected by the user on 2026-10-31. Follow the ToskLight tag-only release pattern: GitHub Actions tests and builds Linux, Windows and macOS packages before publishing a tag and downloadable binaries on the exact tested commit. Do not add release commits, tracked version/changelog updates or npm publication. Read `docs/RELEASING.md` before changing release wiring. Publication requires `MULTICAD_RELEASES_ENABLED=true`; keep credentials out of source. Push and release publication are authorized for the requested release setup.
- macOS interactive checks and Windows/Linux CI results are separate evidence. Do not experiment on the user's Windows computer.
