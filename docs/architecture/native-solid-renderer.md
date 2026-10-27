# Native solid renderer

`viewport::paint_scene` draws an immutable `Arc<Scene>` through an eframe WGPU
callback. Native launch installs the renderer and requests a 32-bit depth buffer.
The callback uses the window's color format, single-sample rendering and
`Depth32Float`. It supports one solid viewport per native window.

The renderer caches each geometry asset by its identity and immutable `Arc`.
Placements sharing an asset use one instance buffer and one draw call. Camera
motion updates a 112-byte camera/section uniform without uploading meshes or placements again.
Scene or selection changes rebuild instance groups; unchanged mesh buffers remain
cached. Assets absent from the new scene are released.

Triangles carry their source face index. B-rep edge buffers use separate topology
tokens and share the same instance placements. Part, face or edge selection changes
their display color. The shader applies rigid placements, the shared camera
projection and simple directional lighting. Depth testing selects the nearest
surface independently of instance order. Perspective clipping uses homogeneous
coordinates so vertices on the eye plane do not require division by zero.

The depth range uses scene bounds and camera position. Viewport dimensions are
logical points, matching camera projection; eframe scales the render viewport for
the physical display. Canonical coordinates and topology intent remain in the
document and geometry modules. GPU positions are display data.

## Verification

`solid_shader_validates_without_a_gpu` parses and validates WGSL with Naga as part
of ordinary workspace tests. No GPU is needed for that CI check.

The separate physical check requires a GPU and runs explicitly:

```sh
cargo test -p multicad-ui gpu_renders_native --locked --offline -- --ignored --nocapture
```

It renders genuine OCCT box meshes to a texture, reads the pixels and checks
orthographic and perspective projection, reversed instance order, part and face
highlighting, an oblique view and cached mesh-buffer reuse. Missing GPU access
fails the check rather than reporting a skipped rendering success. The Mac check
passed on Apple M5 Max through Metal. This is an offscreen rendering check; it
does not establish application input latency or performance on less powerful
hardware.

## Integration still required

The application canvas consumes a per-project `SceneFeed`. The native composition
root registers its backend factory through `Application::set_scene_backends`.
Each backend accepts a source snapshot and request stamp, then polls without
blocking. Source revisions and retries receive new generations. Obsolete success
and failure replies cannot replace a newer result. Failed requests retain the
previous visible scene and expose an explicit Retry action. Pending work requests
periodic repaint; quality debounce schedules a deadline repaint. An idle scene
does not run an animation loop.

The canvas provides filled, wireframe, filled with all edges, and filled with
visible edges. Its Auto selection filter prefers vertices, then B-rep edges,
then faces; explicit Parts, Faces, Edges and Vertices filters are also available.
Visible-edge and filled styles reject occluded edge/point candidates. A fixed
screen tolerance and mesh display error govern picking visibility; neither alters
manufacturing geometry or persisted reference resolution. Selected vertices have
a screen overlay. Part rows are virtualized in the document tree.

Fit uses the selected instance's transformed bounds when a solid is selected.
The floor grid follows the camera and coarsens its displayed lines at distant
zoom. Its configured sketch snap spacing stays unchanged.

`DisplayQuality` selects bounded chord-error levels from 2 mm to 0.001 mm,
targeting less than one logical pixel according to camera scale and perspective
depth. These are display approximations, independent of modeling tolerances.
`SceneFeed` requests finer detail after 150 ms of stable demand and coarser detail
after 600 ms. It schedules a single deadline repaint and polls only while work is
pending. A backend may report unchanged geometry or unsupported refinement.

Refinement uses a new display generation at the same canonical revision. The
previous valid scene stays selectable while it runs or fails; display failures
have a separate warning and explicit retry. Source failures still invalidate
model selection. Semantic selections resolve again after a display replacement.
An older refinement cannot replace a newer source result.

The native review backend exercises this protocol against retained OCCT solids,
reusing unchanged planar assets and resampling curved ones on the worker. Tests
check increased curve detail, unchanged topology and solid keys, shared planar
assets, debounce, retry and obsolete-result rejection. This fixture backend does
not implement production feature compilation.

The section control retains either side of a world X, Y or Z plane with an
editable position and a center-in-model action. A cyan guide and label mark the
active display cut. One extra uniform clips both filled surfaces and B-rep lines
without uploading geometry again. Picking uses the same half-space: triangle
traversal continues past removed front surfaces, and edge segments and vertices
are clipped before screen picking. This is an uncapped display view. It creates
no source faces, changes no document revision and is not a manufacturing section.
Headless tests cover removed-front traversal, parallel rays and clipped selection;
Metal pixel readback checks both half-space removal and revealing a solid behind
another, including line clipping in all four styles.

The production composition root still needs the canonical feature compiler to
supply geometry programs. Registering a display backend does not implement solid
creation commands. Sketch/datum and transform drafts already draw after the solid
callback; solid-feature previews require the upcoming modeling commands. Fuller
physical input-feedback measurements remain CAD-57 work. Visible reference repair
remains unfinished under CAD-55.
