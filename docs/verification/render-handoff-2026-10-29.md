# Render scenes and Blender handoff, 2026-10-29

These checks record agent verification, not human acceptance. The
behavior is described in [render handoff](../architecture/render-handoff.md).

## Source and build

Rust 1.99.1 with the locked lockfile, offline, on macOS (Apple silicon).
Blender 5.1.0 ran in background mode with factory settings.

## Evidence

### Handoff GLB: `crates/multicad-model/tests/render_handoff.rs`

The fixture has a 10 × 20 × 30 mm panel, a pin with a red steel appearance
(metallic 1, roughness 0.3), two placed copies of the panel in a "Bench"
assembly (one turned 90° about Z), and a breakaway tab aid on the pin. The
scene has a 40° perspective camera from (300, −400, 200) mm toward the
origin, a 3 W/m² sun pointing down, a 1000 W orange point light 500 mm up,
a "shaded edges" display and a dark blue background.

Read back from the GLB JSON chunk without the writer:

- four drawn nodes and three meshes: the copies share one mesh, and the
  aid is left out;
- the camera has yfov 40° and sits at (0.3, −0.4, 0.2) m with its local −Z
  toward the target and local +Y up;
- the sun is a directional light of 2049 lux and the point light
  1000 × 683 / 4π cd, with the orange color in linear RGB;
- the scene extras carry the scene ID and the background in linear RGB;
- two materials, including metallic 1 and roughness 0.3;
- the manifest lists four Parts, the excluded aid, the pin's appearance
  and the lights' Blender values;
- rebuilding gives identical bytes; including aids adds a fifth node;
- an orthographic scene of 250 mm gives a ymag of 0.125 m.

Refusals: coincident eye and target, a camera looking straight down, a
179° field of view, duplicate light names and a non-Part scope entry are
refused without a revision change. A scene whose only Part is an excluded
aid, and a handoff name of `../x`, fail without output.

### Blender import

```bash
cargo test -p multicad-model --test render_handoff -- --ignored
```

This writes the handoff and imports it with Blender's own glTF importer
through `tools/blender/render_handoff_report.py`. One run's GLB was 9,332
bytes, SHA-256
`11316d237bea2f13b4fe654f4f9f660f0369c223f18c0daca1a8af19d05813fc`; the
bytes include random entity IDs, so another run differs. Blender reported:

- four mesh objects: "Panel", "Pin", "Panel Rotated copy" and "Panel Moved
  copy";
- scale and placement: the rotated copy at 30–50, 5–15, 2–32 mm and the
  moved copy from −20, 40, 0 mm, so no placement is applied twice;
- hierarchy: both copies under "Bench", sharing one mesh with two users;
- materials: the pin with metallic 1.0, roughness 0.3 and the linear red
  base color;
- camera: perspective, angle_y 0.6981 rad (40°), at (0.3, −0.4, 0.2) m,
  looking along (−0.557, 0.743, −0.371), the direction to the target;
- lights: a SUN of energy 3.0 pointing along −Z, and a POINT of energy
  1000.0 W at 0.5 m height;
- scene custom properties: the scene ID, the revision, "shaded_edges" and
  the linear background.

### MCP: `crates/multicad-mcp/tests/render.rs`

Over MCP dispatch, an extruded block gets a new blue appearance while its
Material stays empty, a render scene is stored, and `blender_handoff`
writes `studio.glb` and `studio.render.json` into the export folder with no
partial files left. The manifest reports revision 6, meters, X-ray, the
blue appearance and a sun of 4 W/m². An invalid scene, a bad name and a
server without an export folder are refused.

### Window: `crates/multicad-ui`

- `render_view` tests: capture and restore round-trip yaw, pitch, scale,
  target and projection for perspective and orthographic views; the
  perspective field of view is 28.0725°; display styles map both ways.
- `render_view_is_captured_into_the_document_and_shown_again`: Capture
  stores an X-ray scene with a default sun as one revision; Show restores
  the camera and display style after the view moved; a second capture
  updates the same scene and Undo restores it; the tools are disabled
  outside Export mode.

## Not verified

- No interactive window session: the Render tab was not clicked in a
  running application.
- No Blender render was made; only the import was inspected.
- Windows and Linux were not run.
