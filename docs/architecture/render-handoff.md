# Render scenes and the Blender handoff

Export → Render prepares a model for rendering in Blender. MultiCAD has no
renderer of its own. A render scene is a record in the canonical document;
the handoff is built from the same native model the export sets use.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/model/render.rs` | Render scene source, validation and transaction |
| `crates/multicad-model/src/export/render.rs` | The handoff GLB and its manifest |
| `crates/multicad-model/src/export/mesh.rs` | GLB cameras and `KHR_lights_punctual` lights |
| `crates/multicad-ui/src/render_view.rs` | Viewport camera capture and restore |
| `crates/multicad-mcp/src/tools/render.rs` | MCP tools |
| `tools/blender/render_handoff_report.py` | Developer-only Blender import report |

## Render scenes

A **render scene** is a `render.scene` extension. It holds no geometry:
it names Parts of the same project, so render settings never become a
second model. A scene records:

- **Camera.** An eye and a target in world millimeters, with +Z kept up.
  The projection is either perspective, with a vertical field of view, or
  orthographic, with a visible height.
- **Background.** An sRGB color.
- **Lights.** Up to 16, each with a distinct name:
  - a **sun** with a direction (the way the light travels), a strength in
    W/m² and a color;
  - a **point** light with a position, a power in W and a color.

  These are Blender's own units, so the values arrive in Blender as
  authored.
- **Display style.** Shaded, shaded with edges, X-ray or wireframe. This is
  the viewport style shown with the scene.
- **Scope.** An optional list of Parts. With no list, the scene covers
  every Part and placed occurrence of the project.
- **Aids.** Whether removable [FDM aids](fdm-aids.md) are left out. They
  are left out by default.

Creating or replacing a scene is one undoable transaction. The transaction
refuses:

- a camera whose eye and target coincide, or that looks straight up or down;
- a field of view outside 1–170°, or a non-positive orthographic height;
- duplicate or empty light names, a zero sun direction, or negative or
  out-of-range strengths;
- a scope entry that is not a Part.

## Appearance

Render appearance is the existing materials **appearance** assignment:
base color, roughness and metallic. It is separate from the manufacturing
Material, stock and section hatch, and it is stored per Part identity in
the Part's properties. The MCP tools create appearance definitions and
change only a Part's appearance, keeping its Material, stock and hatch.

## The handoff

`Exporter::render_handoff` builds a package for one scene:

1. It resolves the scene scope to Part references with occurrence paths,
   and leaves out aid Parts when the scene says so.
2. It compiles the current document natively and checks the source hash.
3. It tessellates the exact solids into one GLB scene, as the GLB export
   recipe does (see [export sets](export-sets.md#glb-scenes)):
   - meters, with one root node turning the Z-up model into glTF's Y-up;
   - one group node per assembly and enclosing occurrence;
   - one mesh shared by every occurrence of the same component;
   - each Part's effective appearance as a PBR material.
4. It adds the scene:
   - the camera, as a glTF camera node looking from the eye to the target;
   - each light as a `KHR_lights_punctual` light. A sun becomes a
     directional light of strength × 683 lux, and a point light becomes
     power × 683 / 4π candela. Blender's importer divides these back in its
     default (Standard) lighting mode;
   - scene extras with the scene ID, the revision, the display style and
     the background as linear RGB. Blender imports scene extras as custom
     properties of its scene.

The package also has a **manifest** (`NAME.render.json`). It records:

- the document, revision and source hash;
- the scene, the GLB file name and its SHA-256;
- the units and axis convention;
- the camera, background, display style and lights, with each light's
  Blender value and glTF intensity;
- every Part handed off, with its occurrence path, node name and
  appearance;
- the aids and Parts left out, and why;
- the triangle count and chord deviation;
- notes on what glTF does not carry.

The same source gives byte-identical output. The handoff fails, without
writing anything, when the name is not a plain file stem, the scene is
invalid or stale, or no Part in scope has a current solid.

## Access

- **Window.** Export → Render shows the 3D viewport with a **Render
  scene** sidebar (`crates/multicad-ui/src/application/render.rs`). The
  sidebar lists the stored scene's projection, eye, target, display style,
  light count and scope, and says whether the visible viewport currently
  shows the stored camera. Common and Print keep their own forms.
  - **Capture render view** stores the render viewport's camera and display
    style in the document's render scene, creating one with a default sun
    if there is none. It is one undoable transaction.
  - **Show render view** sets the render viewport to the stored camera and
    display style.

  Either command switches to the Render tab first, so its receipt always
  describes the viewport in front of the user. Both only set camera state;
  nothing renders or animates while idle.
- **Blender handoff in the window.** The Render sidebar states plainly
  that the window does not write the handoff yet, and that an automation
  client connected through external control can write it. The ribbon's
  Blender handoff button stays disabled.
- **MCP.** `set_render_scene`, `create_appearance`, `assign_appearance` and
  `blender_handoff`. The handoff writes `NAME.glb` and `NAME.render.json`
  into the server's export folder, through partial files that are renamed
  into place.

### Viewport camera mapping

The viewport camera orbits its target at twice the viewport height in
millimeters. Its perspective view therefore has a fixed vertical field of
view of 2·atan(¼) ≈ 28.07°, and its orthographic view shows the viewport
height divided by the zoom scale. Capture uses these values; Show restores
the direction, target and framed size. A scene authored with another field
of view is shown at the viewport's own angle, framing the same target
distance.

## Verification

`crates/multicad-model/tests/render_handoff.rs` reads the GLB JSON
independently. Its ignored test runs Blender's own importer through
`tools/blender/render_handoff_report.py`:

```bash
cargo test -p multicad-model --test render_handoff -- --ignored
```

Set `MULTICAD_BLENDER` when Blender is not on `PATH`, and
`MULTICAD_KEEP_REPORT` to a folder to keep the GLB, the manifest and the
report. See the [verification record](../verification/render-handoff-2026-10-29.md).

## Limits

- **No renderer.** There is no photorealistic or offline renderer and no
  online service. Blender renders the handoff.
- **Window handoff.** The window does not write the handoff yet; the native
  export backend has no render job. Use `blender_handoff` over MCP.
- **One scene in the window.** Capture and Show use the document's first
  render scene. Lights, background and scope are edited over MCP.
- **Background.** glTF has no world background. The background is in the
  manifest and the scene extras; Blender does not apply it to its World.
- **Display style.** Blender uses its own shading; the style is a MultiCAD
  viewport setting.
- **Appearance.** Base color, roughness and metallic only, without
  textures, transparency beyond alpha blending, or per-face appearances.
- **Lights.** Sun and point lights only; no spot or area lights.
- **Light units** assume Blender's default Standard lighting import mode.
