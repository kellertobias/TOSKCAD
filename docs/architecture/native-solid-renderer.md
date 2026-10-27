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

Triangles carry their source face index. Part or current-face selection changes
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

The application canvas does not yet submit document solids to this callback.
The host must adopt accepted scene results and pass its camera and selection to
`paint_scene`. B-rep edge and vertex layers, their picking, visual styles, section
and tool overlays, and the full physical performance scenes remain CAD-57 work.
Visible reference repair remains unfinished under CAD-55.
