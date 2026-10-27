# Native sketch drawing increment — 2026-10-27

CAD-62 remains in progress. This increment adds shared drawing commands and
native gestures for lines, rectangles, circles, circular arcs, connected
polylines and construction geometry. It does not certify the complete sketch
workbench, spline tools, trim/extend, patterning or parameter-expression editing.

## Code and automated evidence

- `91d14ed` extracts exact selectable regions from authored lines/arcs/circles.
  Every bounded nested area is independently available. Unrelated open geometry
  reports diagnostics without removing valid closed regions. Sixteen core and
  four genuine OCCT integration tests cover holes, arcs, provenance, disjoint
  areas and native volumes/intersections.
- `f2366ce` adds typed source-neutral drawing drafts and guarded solver commands.
  Eight core and five solver tests check real point-ID reuse, source preservation,
  invalid geometry, stale revisions, attachment guards and Undo/Redo.
- `1154dc8` adds core transform drafts and eleven tests. This is a tested core
  contract; native transform selection and controls remain subsequent work.
- `cd48e85` and `2394537` provide shared tool metadata, vector symbols, exact
  command parsing, context shortcuts, native gestures and dashed construction
  display. Pointer tests cover radius entry on XZ, actual shared center IDs,
  three-point arc projection, cancellation, closed polyline topology with snapping
  disabled, construction state and Undo/Redo identities.

At `2394537`, workspace formatting and all-target Clippy passed. The workspace
suite reported 262 passed, zero failed and two intentionally ignored tests.
Later schema and hierarchy increments have separate verification.

## Native macOS interaction

Review application:
`artifacts/development/MultiCAD-reference-review.app/Contents/MacOS/reference_review`

Executable SHA-256:
`a827451fdf02165efe70f13955542396ac66f444ff54dfdcc7aa2677b736397f`

The fixture renders an actual OCCT drilled panel. Its limited feature compiler
is separate from production CAD-64 feature authoring.

1. Sketch mode exposed only New Sketch before entry. Selecting the real top face
   and Use plane created Sketch 2 at Z=18 mm, then revealed all drawing controls.
2. F activated Circle. The center click kept revision 1 and zero elements; the
   radius click committed a circle with R=10 mm at revision 2.
3. G activated Arc. Center/start clicks retained revision 2 and one element.
   The end click committed a counterclockwise R=10 mm arc at revision 3.
4. T activated Polyline. Pointer clicks created two connected segments, then a
   click on the first vertex closed a triangle with lengths 20/14.14/14.14 mm.
   Revision 6 contained five elements and 14 degrees of freedom.
5. E and Construction switched to construction-line drawing. Two pointer clicks
   added a 20 mm line, visibly dashed, at revision 7. The Inspector checkbox was
   enabled and the source had 18 degrees of freedom.
6. Header Undo removed that line. Native keyboard Undo/Redo also restored five/six
   elements and 14/18 degrees of freedom. The computer-use key probe established
   a German Z/Y layout; input was mapped using that observed layout before making
   the keyboard claim.
7. Finish sketch returned to Model. Save As created
   `artifacts/sketch-review/drawing-tools.multicad` at revision 11.
8. The checked native-project reader validated the container and emitted
   `artifacts/sketch-review/drawing-tools-source.json`. It contains one circle,
   one arc, four lines, exactly one construction curve and nine authored points.
   The three triangle lines share exactly three vertex IDs, including closure.

The gestures create canonical constrained sketch entities. Display tessellation
is not the manufacturing profile. Windows/Linux interaction, low-power hardware
performance and the broader CAD-62 acceptance criteria remain unverified here.
