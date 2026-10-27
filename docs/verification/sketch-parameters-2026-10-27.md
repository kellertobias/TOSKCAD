# Native parameter-driven sketch check

This check covers a face-mounted sketch in the generated reference-review
application on macOS with WGPU. It does not establish completion of CAD-62,
general solid authoring, export readiness or Windows/Linux behavior.

## Formula preview, apply and parameter edits

The review executable at
`artifacts/development/MultiCAD-reference-review.app/Contents/MacOS/reference_review`
had SHA-256
`db92ef87e163a958208ae6b1a8bea12526d59114520c18cbca1aada0fbb51429`.
It included the parameter-binding and native UI increments through `1163e76`.

The existing face-mounted circle started at radius 3 mm. Adding a radius
constraint and previewing `=$panel_width / 20` produced radius 5 mm with
`panel_width=100 mm`. Discard restored radius 3 mm and left revision 0.
Repeating the draft and applying it produced one source transaction, revision 1.

Changing width to 140 mm produced radius 7 mm at revision 2. Changing it to
160 mm produced radius 8 mm at revision 3. The native viewport also displayed
the corresponding changed hole in the reference solid. Both revisions were
saved as separate native projects:

- `artifacts/sketch-review/parameter-width140.multicad`
- `artifacts/sketch-review/parameter-width160.multicad`

Reopening the first file in a new tab restored the formula and radius 7 mm.
Switching to the other tab restored radius 8 mm. These tabs contain revisions
of the same canonical document identity and retain separate evaluated results.
Previewing `=$missing_radius` displayed an unknown-parameter error, supplied no
Apply action, and left the visible radius and source revision unchanged.

## Saved source comparison

The native container reader wrote `parameter-width140-source.json` and
`parameter-width160-source.json` beside the projects. Comparison found exactly
one changed entity: parameter `panel_width`, from `140` to `160`.

The entire sketch entity, including its formula, bound parameter identity,
geometry IDs, authored coordinates and face reference, remained identical.
Its authored circle radius remains 3 mm; evaluated radii are separate data.
The normalized comparison JSON hashes the sketch entity as
`5fdb3a8e9d5ee9df3c529478b8e3a9ec2df83abc0837e4fb231cf9eef8c3ebda`.
The comparison report is `artifacts/sketch-review/parameter-comparison.json`.

## Literal-preview regression

The first build exposed a display guard that also rejected a valid literal
constraint preview. Commit `a3179fa` limits the exception to the solver-checked
preview document and its own sketch. Pending or failed canonical solves remain
unavailable for interaction.

The replacement executable had SHA-256
`bb6e997f716745ee250f6ea2f164dcff881320f1a8fa2814deddd8e09f72cc64`.
Adding a literal radius constraint kept the circle visible, showed two degrees
of freedom and an Apply action, and left revision 0. Discard restored three
degrees of freedom at revision 0. The unchanged generated fixture was saved as
`artifacts/sketch-review/literal-preview-baseline.multicad` before quitting.

## Automated evidence and limits

Workspace formatting, Clippy with warnings denied, and workspace tests passed
for the integration and preview-fix snapshots. Tests also cover bounded
background evaluation, stale replies, failures retaining display-only geometry,
cancel/apply/undo, and genuine OCCT volume changes driven by sketch parameters.
The upstream OCCT deprecated `sprintf` compiler warning remains.

This was a functional check on the available Mac. It supplies no low-power
hardware benchmark, cross-platform interactive result or human acceptance.
