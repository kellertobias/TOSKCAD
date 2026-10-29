# Native STL and GLB exports, 2026-10-29

These checks record agent verification, not human acceptance.

## Scope

Export-set mesh recipes now write:

- **STL:** binary, in the chosen unit, with the unit named in the header;
- **GLB:** glTF 2.0 in meters, as a named scene with assembly groups, meshes
  shared between instances, rigid placements and part appearances.

Publication reads both formats back before a bundle is published. The
behavior is described in [export sets](../architecture/export-sets.md#glb-scenes).

## Source and build

The checks used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| Strict all-target Clippy | Exit 0 |
| `cargo test --workspace --locked --offline` | 1,400 passed, 0 failed, 2 existing ignored |
| `npm test` | 25 of 25 passed |

## Evidence

### Native exports: `crates/multicad-model/tests/mesh_exports.rs`

The readers share no code with the writers: a binary STL parser, and a glTF
reader that decodes the buffers and composes node matrices and quaternions
itself.

- **STL.** An explicit scope of the panel and its rotated copy writes two
  millimeter files:
  - the panel at 0–10, 0–20, 0–30 and the copy at 30–50, 5–15, 2–32;
  - 6000 mm³ each;
  - the file length matches the facet count, and facet normals agree with
    the winding.

  An assembly-preserving inch STL holds both bodies at 2 × 6000 / 25.4³ in³
  with `units=inch` in its header. A second run into another root writes
  identical bytes.
- **Assembly GLB.** The Bench assembly gives one GLB with one mesh, the
  local definition (maximum 0.010, 0.020, 0.030 m), shared by two leaf
  nodes:
  - the leaves sit under a "Bench" group under a root that declares meters;
  - composed from the scene root and turned back to Z-up millimeters, they
    land at the same world bounds as the 3MF build items: 30–50, 5–15, 2–32
    and −20 to −10, 40–60, 0–30;
  - each encloses 6000 mm³, so no placement is applied twice;
  - the material is the default appearance as linear PBR factors.
- **Project and tag scopes.** A whole-project GLB holds the panel, the pin
  and both copies once each, with a total of 3 × 6000 + π·25·10 mm³. The
  pin's red steel material and the default appearance give two materials.
  The `cnc` tag query selects:
  - the panel and its two copies, without the pin;
  - three drawn nodes and two meshes: the placed Part in world, and one mesh
    shared by the copies.
- **Refusals.** A millimeter GLB recipe is Unsupported because glTF is
  defined in meters. Its Run writes nothing, and the earlier OBJ/3MF bundle
  is unchanged. The existing tests still cover a removable-aid recipe and
  failed runs.

### Blender import

The makerspace scenario (`makerspace_review`) now also defines a "Makerspace
scene" set: the whole project as one GLB. The generated file was 14,064
bytes, SHA-256
`72d45207c18c6f85428dd547627bf9501eea10b474ce3bc2480c097e984c192b`.

The following command imported it with Blender 5.1.0 in background mode:

```bash
blender -b --factory-startup --python-exit-code 1 --python tools/blender/glb_import_report.py -- FILE.glb REPORT.json
```

The report lists 14 objects, of which 7 are meshes with 264 triangles in
total.

- **Scale and orientation.** In Blender's Z-up meters:
  - the walls span −0.1 to 9.1 × −0.1 to 6.1 × 0–3.6;
  - the ground slab spans 0 to −0.2 in Z;
  - the deck is at 2.4–2.6 and the stair rises to 2.6;
  - the CNC router occupies 6.0–8.5 × 1.5–3.0 × 0–1.1, the laser cutter
    0.6–2.0 × 1.2–2.1 × 0–1.0 and the workbench 3.0–4.8 × 1.2–2.0 × 0–0.9.

  These match the model in millimeters.
- **Names and hierarchy.** Each machine is an empty named "Laser cutter",
  "CNC router" or "Workbench" holding its equipment body. It sits under its
  symbol's private definition group, under the scene root.
- **Appearance.** Concrete parts import with base color (0.515, 0.503,
  0.468), the linear form of sRGB (190, 188, 182). The equipment uses the
  default appearance (0.25, 0.423, 0.571).

Each symbol placement has its own private definition, so the three machines
do not share mesh data in this scene. Shared meshes for repeated Instances
of one component are covered by the assembly test above.

## Not verified

- **Blender check.** The import is a developer check run on this Mac; it is
  not part of the automated test suite or CI.
- **Other STL readers.** Slicers and other readers that assume millimeters
  will scale non-millimeter files wrongly. The unit is only stated in the
  header.
- **GLB contents.** No normals, textures, cameras, lights or render settings
  are written. Render mode and the Blender appearance handoff are CAD-94.
- **Hierarchy order.** Symbol placements appear inside their private
  definition group, as the export scope walker reports them, rather than as
  top-level machine groups.
- **Other platforms and the window.** Windows and Linux were not run, and
  the window's export UI was not exercised interactively.
