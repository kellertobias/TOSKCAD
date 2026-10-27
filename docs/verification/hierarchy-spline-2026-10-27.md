# Native hierarchy and spline editing check

This check uses the generated reference-review application on macOS with WGPU.
It covers component ownership, visibility and semantic sketch editing. It does
not establish general solid authoring, manufacturing exports or Windows/Linux
interactive behavior.

## Hierarchy and viewport

The first hierarchy review created component `Cabinet A`, made it active,
created group `Panels` inside it and moved `Drilled panel` into that group.
Hiding the group was saved as
`artifacts/hierarchy-review/component-hidden.multicad`.

That build exposed a tree layout defect: repeated row layout expanded the left
panel until the viewport had no width. Commit `baef2da` bounds rows and their
trailing controls to the parent panel. Its regression test runs 720 frames at
standard and Retina scale with long names, deep groups and feature history.

The replacement executable at
`artifacts/development/MultiCAD-reference-review.app/Contents/MacOS/reference_review`
had SHA-256
`d70969eaf843d98239c8e430cc149fe677fc57244822ac6155130400f2f09bab`.
It included implementation through `3891319`.

Opening the hidden checkpoint kept the left panel at 225 logical pixels, with
a visible central viewport and empty Inspector. Selecting the hidden body
showed its owner, material and tags. Unhiding `Panels` at revision 1 restored
the actual drilled solid, its selection highlight and instance actions.

## Pointer spline and checked edits

Before New Sketch, Sketch mode showed its creation command without drawing or
editing tools. Selecting the XY datum and using it created `Sketch 2` and its
plane in active component `Cabinet A` at revision 2.

Four pointer-defined control poles remained an uncommitted gesture. Clicking
the first pole closed the cubic spline at revision 3. The Inspector showed
five pole references, four unique points, eight degrees of freedom and one
exact closed region of 1800 mm². An interior viewport click selected that region
through the native region-query worker.

Selecting the spline and using Edit → Move previewed a 10 mm displacement.
The ghost moved while revision remained 3. Discard restored its original
position. Repeating Preview and Apply created revision 4; native Undo restored
the original poles at revision 5. This state was saved separately as
`artifacts/hierarchy-review/component-spline.multicad`.

Linear Pattern with 70 mm spacing and three instances previewed three splines.
Apply created revision 6 and rebuilt three independently selectable regions,
each of area 1800 mm². This state was saved as
`artifacts/hierarchy-review/component-pattern.multicad`.

## Saved source comparison

The native container reader produced `component-hidden-source.json`,
`component-spline-source.json` and `component-pattern-source.json` beside the
projects. `hierarchy-spline-comparison.json` records the checked identities.

The hidden-to-spline comparison preserves every original entity, including the
body, instance placement, feature history and existing face-mounted sketch.
The new plane and sketch both belong to active component `Cabinet A`; the body
remains inside `Panels`. The spline uses source schema 3 and reuses the first
pole UUID at closure. Its normalized entity SHA-256 is
`1c68c370245c2f7f9c384d144c2732856d695780b54e3f4872273ee3e6d87597`.

The pattern comparison changes only that sketch entity. The original point
and curve prefixes remain identical; three curves use twelve unique points.
Hierarchy and active ownership are unchanged.

## Reopen, selection and analytic edits

The selection-fix build through `fb69c53` had executable SHA-256
`0313723160ff89a3e0356a6a4e2b3ba1cb1b78b1cdc4352029f2059662746345`.
Reopening `component-pattern.multicad` restored active `Cabinet A`, its group
and body ownership, and all three source-schema-3 spline regions. Double-clicking
the sketch opened it for editing. Shift-click added a second interior region;
another Shift-click removed the first while retaining the second. Revision
remained 0 throughout these selection operations.

Rotate at 45 degrees, vertical Mirror and a three-instance Circular Pattern
each produced a checked preview and the corresponding Apply action. Discard
restored the saved source without creating a source revision.

A separate XY sketch used exact command-line entries:

```text
line -120 10 -20 10
line -100 -5 -100 25
line -50 -5 -50 25
```

Trim at `(-75, 10)` against the two vertical lines previewed two surviving
horizontal fragments of 20 mm and 30 mm. Apply committed revision 5. Extending
the left fragment to the vertical boundary at `x=-100` reported that no boundary
lay beyond its endpoint and offered no Apply action. Choosing the boundary at
`x=-50` instead previewed a 70 mm line; Apply committed revision 6. The result
was saved as `artifacts/hierarchy-review/analytic-edits.multicad`, preserving
the earlier patterned-spline asset. The review application then quit cleanly.

## Verification scope

Formatting, workspace Clippy with warnings denied and workspace tests passed
for the integration snapshot: 410 passed and two ignored. The upstream OCCT
deprecated `sprintf` compiler warning remains. The selection-fix snapshot passed
the same checks with 413 tests passed and two ignored.

Native multi-selection exposed a second defect: Shift release later in the same
input frame replaced the selection. A regression reproduces the failure; the
shared click-modifier correction uses modifiers captured at mouse release and
passes the native recheck above. Downstream profile retrieval also refuses an
unapplied pointer query and validates the canonical source at its action
boundary. CAD-62 and CAD-114 remain in progress: numerical point/pole editing
and a richer hierarchy fixture are still being completed. This report records
no human acceptance or low-power performance benchmark.
