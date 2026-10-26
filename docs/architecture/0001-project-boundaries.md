# 0001: Native application and bootstrap boundaries

Status: accepted product direction; kernel, solver and UI implementation choices remain gated by [CAD-49](https://plans.tokenet.de/CAD/49).

## Decision

MultiCAD targets macOS, Windows and Linux as a native application, mainly in compiled Rust. The preferred geometry foundation is OCCT behind a narrow native adapter. A native Rust UI/viewport and constraint solver must be proven before integration. No Python application runtime, FreeCAD installation or web mockup is a production dependency.

The initial workspace contains only safe Rust bootstrap metadata and a diagnostics executable. It deliberately selects no CAD, GUI, GPU, solver, serialization or MCP dependencies. The native dependency builds and desktop application packaging belong to [CAD-50](https://plans.tokenet.de/CAD/50) after the foundation proof.

## Application contracts

- One canonical parametric document, stable identities, explicit units and dependency-driven rebuilds.
- A shared typed command/query API for UI and external MCP clients, with previews, transactions, revision checks, Undo/Redo and attributable activity.
- Cached display meshes independent of exact modeling/manufacturing geometry. Numerical robustness requires tolerance policy, reference provenance and repair diagnostics, not a promise to eliminate floating-point error.
- Workspace/ribbon changes affect available tools, not project geometry.
- Colored modes: Parameters, Sketch, Model, Drawings, Render and Export; Measure and Library are global.
- Parameters is a full-width table with no sidebars or ribbon. Sketch initially retains the 3D viewer; only New Sketch or opening an existing sketch enables its editing tools. New Sketch selects a planar face or construction plane in 3D. No Model-level Face Sketch command.
- The Inspector is empty when idle, shows properties/quick actions for selections, and tool controls for active tools.
- Sheet metal includes attached fasteners and source-driven mounting geometry. FDM aids are Model features; orientation/mesh/3MF options are Export functions.
- Drawings owns multi-page documents, title blocks, material hatching and model-linked views. 3D drawing guides precede linked page generation. Named export sets resolve part-tag queries.
- MCP is external application control, not built-in AI chat.

## Licensing and distribution

The application source uses `GPL-3.0-or-later`; the complete text is in `LICENSE`. No third-party runtime packages are included in the bootstrap. This choice does not certify future OCCT/solver/UI dependency compatibility or distribution compliance: CAD-49 must inventory the selected versions, licenses, linking boundaries and notice/source requirements before integration.

The pinned Rust toolchain and CI are build infrastructure. Forgejo is authoritative for branches; GitHub builds mirrored commits and mints tag-only releases without version-bump commits. [Release wiring](../RELEASING.md) remains inactive until repositories exist and mirror tag reconciliation is verified. No repositories, remote URLs, signing credentials or publication are created by the bootstrap. Windows/Linux runtime and physical performance acceptance require their own evidence.

## References

- [Product plan and implementation index](https://plans.tokenet.de/CAD/48)
- [Repository bootstrap](https://plans.tokenet.de/CAD/107)
- [Native foundation gate](https://plans.tokenet.de/CAD/49)
- [Native dependency integration and packaging](https://plans.tokenet.de/CAD/50)
- [GNU GPL v3 text](https://www.gnu.org/licenses/gpl-3.0.html); repository copy obtained from the [SPDX license list](https://github.com/spdx/license-list-data/blob/main/text/GPL-3.0-or-later.txt).
