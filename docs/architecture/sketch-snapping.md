# Sketch snapping, dwell references and assistive lines

Status: implemented for native sketch draw tools (CAD-136). macOS interactive review pending.

## Scope

While a draw tool (Line, Rectangle, Circle, Arc, Polyline, Spline) is active in an editing sketch, the pointer is resolved against the current solved sketch geometry before the grid is considered. The logic lives in `crates/multicad-ui/src/sketch_snap.rs`, which is pure and has time injected. The canvas bridge and overlays live in `sketch_snap_host.rs`.

These aids are transient. They never create sketch entities, constraints, dirty state or Undo entries. A click places exactly the shown position, or reuses the shown existing point, through the shared `sketch_draw` transaction like any other draw. Persistent construction geometry and automatic constraints are out of scope.

## Targets and ordering

All distances are measured in screen pixels after projecting the sketch-plane candidate with the current camera. This keeps tolerances constant through pan and zoom, and the overlays are re-projected from sketch coordinates every frame.

| Class | Targets | Acquisition distance |
| --- | --- | --- |
| 0 | Existing sketch points: endpoints, vertices, spline poles, circle/arc center points (reused by identity) | 8 px |
| 1 | Line midpoints, shape centers, acquired references, midpoint of two references | 8 px |
| 2 | Intersection of two assistive lines; assistive line crossing an existing line | 8 px |
| 3 | Nearest point on an existing line, circle or arc (arcs only inside their sweep) | 6 px |
| 4 | Horizontal or vertical assistive line through a reference | 6 px |

The lowest class within its distance wins. Within a class, the nearest target wins. The target shown in the previous frame stays eligible up to 1.5× its acquisition distance, which prevents flicker.

Grid rounding (the "Snap sketch points to grid spacing" option) applies only when no geometry target is indicated. A nearby grid point never displaces a geometry target, and along an assistive line the free coordinate is not grid-rounded. Geometry snapping has its own toggle, "Snap to geometry and assistive lines" (default on). It is independent of the grid option.

Construction curves participate like normal curves. B-splines contribute only their poles, with no on-curve or center targets.

## Centers

A center target exists only where it is geometrically unambiguous:

- **Circle and arc:** the explicit center point (class 0, labelled "Center").
- **Rectangle:** any closed loop of four lines with four right angles (relative tolerance 1e-6). The center is the intersection of the diagonals. The loop is still recognized when other lines attach to its corners.
- **Regular polygon:** a closed loop of three or more lines whose points have no other line connections, with equal side lengths and all vertices equidistant from their mean (relative tolerance 1e-6). The center is the vertex mean, which equals the circumcenter.

Any other loop has no center target. No centroid or bounding-box center is guessed for irregular shapes.

## Dwell references

- **Acquiring:** hovering an acquirable target (sketch point, line midpoint, shape center, or point on a line/circle/arc) for **0.5 s** acquires it as a reference, shown as an orange cross at its exact position. The pointer may drift **4 px** from where the dwell started; larger movement, or a different target, restarts the timer. Briefly passing over a target never acquires it. A pending dwell requests a repaint, so acquisition does not depend on further pointer motion.
- **Limit:** up to **2** references are held. Acquiring a third drops the oldest.
- **Midpoint of two references:** shown as a diamond marker and offered as a snap target. For example, references (0,0) and (20,10) place (10,5).
- **Assistive lines:** each reference offers horizontal and vertical lines in all four directions. They are drawn dotted from the reference through the snapped position. Lines from two references intersect, and a line also snaps where it crosses an existing line. Exact alignment with a reference is also drawn when another target wins.
- **Releasing:** dwelling again on a held reference releases it.

Lifetime:

- Point, midpoint and center references follow their source when the solved geometry changes.
- On-curve references keep their acquired position while the curve exists.
- A reference is dropped when its source disappears, for example after Undo or delete.
- Cancelling (Esc), switching tools, finishing or leaving sketch editing, switching sketches and toggling geometry snapping clear all references. `SketchEditor::clear_snap_references` clears them explicitly.
- When the pointer leaves the canvas, references are kept but the dwell restarts.

## Verification

- `sketch_snap_tests.rs` (pure) covers:
  - dwell timing and drift, two-reference midpoint, guides in four directions, guide intersections
  - line midpoints and on-curve snapping, competing targets, grid versus geometry, hysteresis
  - supported and unsupported centers, arc sweep, reference limit, release, revalidation, zoom-invariant pixel tolerances
- `sketch_snap_host_tests.rs` drives the real canvas with injected egui time and covers:
  - dwell without document change
  - the (10,5) commit with the grid on and off
  - line and assistive-line snaps against a nearby grid point
  - rectangle center and corner placement
  - anchoring through pan and zoom
  - cancel, tool switch and finish leaving the document and revision unchanged
  - the geometry-snap toggle

Not verified: interactive macOS review of the feel of the 0.5 s dwell, colors and label legibility, and Windows/Linux runtime behavior.
