# Native solid tools checkpoint, 2026-10-28

The production macOS application created a Box, subtracted a Cylinder and added
a separate revolved part. Saved canonical projects regenerated with the production
native compiler. This records implementation evidence, not human acceptance or
completion of the modeling workspace.

## Reviewed executable

The local bundle was `artifacts/development/build-nKoVNx/MultiCAD.app`.
The executable SHA-256 was:

```
5168897205afc1054688efbdc7534d5038afe77064e99742cab4b52ebbbd2bd8
```

The bundle includes the primitive, revolve and explicit Boolean forms. It
predates the preview Fit correction and the source-bound query metadata adapter.
Nothing was installed, pushed or published.

## Primitive and cut interactions

In a fresh document, the native Inspector previewed and applied an 80 x 60 x 20 mm
Box, then a Cylinder with 5 mm radius and 40 mm height at (20, 20, -10) mm.
The explicit Boolean form selected the Box as target, the Cylinder as tool and
Cut with Keep tools disabled. Preview showed the hole; Apply produced revision 3.
Undo restored both uncut bodies at revision 4, and Redo restored the cut and tool
consumption at revision 5.

Save As wrote `artifacts/model-review/direct-solids-cut.multicad`. An independent
production compiler report, `direct-solids-cut-report.json`, found one final
solid with bounds (0, 0, 0) to (80, 60, 20) mm and volume
94,429.20367320509 mm³. This matches 96,000 minus 500π mm³. The Cylinder remains
canonical history and is absent from the final displayed bodies.

## Sketch and revolve interactions

The same session created an XZ sketch with normal offset 100 mm, placing its
origin at (0, -100, 0) mm. The command line created the rectangle
`rect 10 0 30 20`. Its 400 mm² profile was selected for Revolve around the local
V axis through (0, 0), with a signed sweep of -270 degrees.

Preview succeeded. Home and Fit exposed a camera defect: Fit still framed the
small source sketch while the candidate solid was visible. After Apply, Fit
selection framed the revolved body correctly. The preview defect is covered by
the later real-native-vertex regression in
[`sketch_editor` tests](../../crates/multicad-ui/src/sketch_editor.rs); a physical
recheck of that correction remains separate evidence.

Save As wrote `artifacts/model-review/direct-solids-and-revolve.multicad`,
preserving the cut-only file. Its independent compiler report contains:

| Final part | Solid count | Volume, mm³ | Bounds, mm |
|---|---:|---:|---|
| Cut Box | 1 | 94,429.20367320509 | (0, 0, 0) to (80, 60, 20) |
| Revolved part | 1 | 37,699.11184307742 | (-30, -130, 0) to (30, -70, 20) |

The revolved volume matches 12,000π mm³. The Box's identity, producer and geometry
remain unchanged; the Cylinder is still consumed. All review documents were
saved and the application quit cleanly. This combined file has been regenerated
headlessly; a later interactive reopen is not claimed here.

## Evidence boundaries

The reports come from saved canonical source through
[`model_file_review`](../../crates/multicad-app/examples/model_file_review.rs),
which uses the genuine native compiler. Display triangles are report data and
are not manufacturing input. These files and the bundle are ignored local
verification artifacts.

Applied-feature editing, viewer-picked termination, extrusion target discovery
and independent Split outputs require further UI evidence. Exact native taper
and termination tests do not establish their interactive workflow. Windows/Linux
physical runtime, low-power performance, general manufacturing exports and Room
workflows remain unverified by this checkpoint.
