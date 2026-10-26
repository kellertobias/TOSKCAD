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
- Parameters uses the full window, without sidebars or a ribbon. Sketch starts in the 3D viewer; New Sketch selects a plane there, or an existing sketch is opened, before editing tools appear. No Face Sketch command in Model.
- MCP provides external application control and attributable previews/history. Do not introduce built-in AI chat. FDM design aids belong in Model; print packaging belongs in Export.

## Verification

- Use the pinned Rust toolchain and committed lockfile. Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` for relevant Rust changes.
- Test observable behavior and failure cases. Do not claim a CAD workflow, valid manufacturing file, cross-platform runtime or performance result from a placeholder or source inspection.
- Node.js is developer/CI-only for prototype maintenance and release tooling, never a native runtime dependency. `node mockup/verify.mjs` checks illustrative UX behavior, not CAD correctness; `npm test` checks release contracts.
- Forgejo owns branches; GitHub runs builds and creates tag-only releases on tested mirrored commits. Do not add release commits, tracked version/changelog updates, npm publication or Forgejo credentials to GitHub. Read `docs/RELEASING.md` before changing release wiring. Keep publication disabled until the real repositories and deployed tag reconciliation are verified. No push is currently authorized.
- macOS interactive checks and Windows/Linux CI results are separate evidence. Do not experiment on the user's Windows computer.
