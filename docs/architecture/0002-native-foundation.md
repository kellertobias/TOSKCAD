# 0002: Native foundation evidence and module boundaries

Status: bounded CAD-49 experiment; production promotion requires the recorded proof results and review. Not a declaration that the application or CAD-50 packaging is complete.

## Dependency decisions

Keep domain state and command execution in safe Rust, independent of the UI, persistence encoding, constraint solver and B-rep implementation. Use opaque native-owned shapes behind a CXX boundary; convert exceptions to owned errors and validate results before publication. Do not expose OCCT pointers or solver IDs to documents or UI modules. No Python application runtime.

| Candidate | Version | Decision |
| --- | --- | --- |
| OCCT official source | 7.9.3, commit `a016080bf6738d6aeae020badee4e888ad1540a5` | Source-build selected toolkits behind our narrow adapter; immutable archive checksum in the proof manifest. |
| occt-sys bundled OCCT | 7.8.1 | Rejected for this integration: STEP lifecycle exercised a `Standard_Type` destruction crash on this Mac. Do not hide shutdown failures with process exit/leaks. |
| opencascade high-level wrapper | 0.3.0 | Inspected; narrower owned adapter selected to control result validation and exception/error handling. |
| sketch-solver | 0.1.0 | Pure Rust candidate behind a replaceable mapping module. Exercise DOF, contradictory dimensions and residual checks. Young dependency; a much larger constraint corpus remains CAD-61 work. |
| slvs / SolveSpace wrapper | 0.6.0 | Published wrapper failed compilation with generated bindings missing types on local Apple Clang 21/bindgen 0.64. No global registry patches or Python fallback. Retain SolveSpace as an alternative if the candidate fails the corpus. |
| eframe / egui | 0.36.2 | Compiled native UI with wgpu and AccessKit. Chosen compatible pair with docking; 0.37.0 was inspected but is incompatible with egui_dock 0.21.1's egui 0.36 requirement. |
| egui_dock | 0.21.1 | Bounded docking/resizing proof; keep layout IDs separate from domain identities. |
| serde / serde_json / tempfile | Exact versions in proof lockfile | Proof-only persistence, strict schema and same-directory atomic replacement; production encoding and migration policy remain undecided. |

Upstream sources: [OCCT 7.9.3](https://github.com/Open-Cascade-SAS/OCCT/releases/tag/V7_9_3), [upstream static-type lifecycle issue](https://www.occt3d.com/dev/tickets/0033656/), [Rust solver](https://github.com/neoscad/sketch-solver), [egui releases](https://github.com/emilk/egui/releases), [docking](https://github.com/anhosh/egui_dock), [SolveSpace wrapper](https://github.com/thekakkun/rust_slvs). The local crash report specifically identified `Standard_Type::~Standard_Type` and static map destruction; upstream issues are context, not proof of the exact local root cause.

## Production architecture target

1. **Document**: canonical entity IDs, unit-safe values, feature definitions and dependency graph. No widget state, GPU resources or native pointers.
2. **Command host**: registered typed commands/queries, revision checks, validation, previews, commit/Undo/Redo and attributable events. UI, CLI and MCP call this same host. Modules contribute commands and capability descriptors at a composition root.
3. **Evaluators**: geometry and sketch solver ports implemented by replaceable adapters. Compute asynchronously from immutable document snapshots; publish results only for the matching revision. Cache display meshes separately from exact solids.
4. **Storage**: codecs/migrations and atomic file service behind a port; load/validate/rebuild a candidate before replacing an open document. Preserve dirty work and unsupported files. Native dialogs belong to the shell, not the document.
5. **UI shell**: mode/layout policy, selection and tool lifecycle. Modules contribute ribbon/tool descriptors, Inspector editors and workspace content; adding a domain module should require registration and its implementation, without changing unrelated workspaces. Never serialize UI enums as feature types.
6. **Viewport**: scene snapshots, picking IDs, cameras and cached GPU data. A display offset or tessellated coordinate is never the authoritative manufacturing placement.
7. **Deliverables**: registered exporters consume exact model/drawing snapshots with explicit scope, units and output recipes. No exporter mutates the document.

Prefer statically linked Rust modules with typed registration first. A dynamic binary plugin ABI is not necessary to achieve modularity and would add version/lifetime hazards before the APIs stabilize. Future external extensions can use a versioned process/command boundary. Production modules must have independent tests without starting a window or importing the kernel where unnecessary.

## License and distribution evidence

Application/proof source remains GPL-3.0-or-later. OCCT is LGPL-2.1 with its additional exception; the original license/exception texts must accompany distributed native packages, along with the selected source and any patches and the required relinking/source materials. Static linking in this experiment does not waive those obligations. The complete distribution/source-package process remains CAD-50 work, and this lab is not added to public release assets.

eframe/egui/wgpu/CXX/sketch-solver offer MIT or Apache-2.0 licensing; egui_dock is MIT; cmake's crate is MIT or Apache-2.0. The resolved cross-platform Cargo inventory is [foundation/dependencies.csv](foundation/dependencies.csv). That metadata inventory is not a replacement for collecting notices, embedded assets/subcomponent terms or validating the eventual shipped binary dependency closure. OCCT source/license pin is separate because it is not a registry crate. See [official OCCT licensing](https://occt3d.com/dev/doc/overview/html/occt_public_license.html).

## Evidence and remaining gates

Reproduce commands and expected boundaries are in [the laboratory guide](../../spikes/native-foundation/README.md). Concrete results, commit IDs and machine-specific timings belong in CAD-49's Working Log and ignored `artifacts/foundation` reports.

macOS native smoke checks cover numeric/text keyboard input, layout, plane selection and part/Inspector selection. AccessKit exposes controls, but accessibility activation did not always focus a text field; normal coordinate focus and keyboard input worked. Screen-reader navigation and complete keyboard-only CAD operation remain unproven. CPU rendering at simulated scales is separate from real display/DPI transitions. Native window close protection and fully modal file dialogs are not implemented in this proof and must be implemented by the production shell/file lifecycle work.

Windows/Linux execution, physical low-power performance, large scenes, curve constraints, general profiles, topology references, plugin command schemas, full drawing/export behavior and MCP remain unproven. No downstream gate should be marked complete by source inspection or these small fixtures.
