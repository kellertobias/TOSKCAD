# Native solid renderer

`viewport::paint_scene` draws an immutable `Arc<Scene>` through an eframe WGPU
callback. Native launch installs the renderer and requests a 32-bit depth buffer.
The callback uses the window's color format, single-sample rendering and
`Depth32Float`. It supports one solid viewport per native window.

The renderer caches each geometry asset by its identity and immutable `Arc`.
Placements sharing an asset use one instance buffer and one draw call. Camera
motion updates a 96-byte uniform without uploading meshes or placements again.
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
previous visible scene and expose an explicit Retry action. Only pending work
requests periodic repaint; an idle scene does not run an animation loop.

The canvas provides filled, wireframe, filled with all edges, and filled with
visible edges. Its Auto selection filter prefers vertices, then B-rep edges,
then faces; explicit Parts, Faces, Edges and Vertices filters are also available.
Visible-edge and filled styles reject occluded edge/point candidates. A fixed
screen tolerance and mesh display error govern picking visibility; neither alters
manufacturing geometry or persisted reference resolution. Selected vertices have
a screen overlay. Part rows are virtualized in the document tree.

The production composition root still needs the canonical feature compiler to
supply geometry programs. Registering a display backend does not implement solid
creation commands. Section and tool-preview layers and the full physical
performance scenes remain CAD-57 work. Visible reference repair remains unfinished
under CAD-55.
