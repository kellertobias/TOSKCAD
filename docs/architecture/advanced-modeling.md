# Advanced parametric solids

The Model/Common ribbon exposes Shell/Thickness, Draft, Sweep and Loft. Its
Pattern menu contains Mirror, Linear pattern and Circular pattern. The
[shared controller](../../crates/multicad-ui/src/modeling/controller/advanced.rs)
prepares native previews through the same typed commands available to external
callers. Apply commits only the matching successful preview as one transaction;
Cancel discards the draft. Existing features reopen for editing.

## Sources and selection

The [canonical source](../../crates/multicad-core/src/model/advanced/source.rs)
stores `model.advanced` intent, bound dimensions and persistent input identities.
Part modifiers freeze their preceding producer. Linked Instances follow the Part
definition; these tools do not modify an individual Instance independently.

Shell and Draft accept explicit native face captures. Selection displays the
frozen input, while validation uses the complete current document. Tool-first
entry acquires a Part before opening its controls. Shift-click toggles faces.

Sweep and Loft create a new Part from explicit sketch inputs. The
[host](../../crates/multicad-ui/src/sketch_advanced_host.rs) offers only current
solved profiles and nonconstruction straight path curves. Document identity,
revision, source hash and evaluation stamp must agree. A source edit invalidates
an old preview. The compiler resolves saved sources again rather than treating
the display cache as geometry.

## Supported operations

| Tool | Authored controls and current boundary |
|---|---|
| Shell/Thickness | Explicit opening faces and signed wall thickness. Positive values hollow inward; negative values offset outward. At least one opening is required. An empty face list does not produce a closed hollow solid. |
| Draft | Selected planar, cylindrical or conical faces, signed nonzero angle strictly between -90° and 90°, neutral plane and pull direction. Results that change topology counts are rejected. |
| Sweep | One closed outer section without holes, following an ordered open chain of nonconstruction straight segments from one sketch. The first path point lies in the profile plane and its first segment is perpendicular to that plane. Endpoints and traversal are authored explicitly; arc and spline paths are unsupported. The native path budget is 2–65 points. |
| Loft | 2–16 ordered closed sections without holes, with matching boundary edge kinds/counts. Every section stores a start curve for correspondence; the Inspector supports reorder and seam selection. Ruled and smooth surfaces are supported. The kernel does not guess or reorder correspondence. |
| Mirror | An explicit plane reflects the Part's native shape. Reflection remains separate from positive-determinant Scale. |
| Linear pattern | Bound integer count and translation step. The original is member zero. |
| Circular pattern | Bound integer count, step angle, axis origin and direction. The original is member zero. |

Patterns contain up to 128 members, subject to native budgets of 1,024 solids and
100,000 topology elements. An unmerged pattern is an exact compound; merging is
an explicit native Boolean. Members share the Part's properties and placement.
The [pattern decision](0019-named-compound-patterns.md) describes stable member
names and the boundary with independently placed assemblies.

The separate [Fillet/Chamfer contract](edge-treatments.md) covers constant radius,
symmetric distance, two-distance and distance/angle profiles, including explicit
adjacent-face selection.

## Failures and persistence

The [native adapters](../../crates/multicad-kernel/src/advanced_operations.rs)
validate exact geometry, material side and bounded results. The
[profile adapter](../../crates/multicad-kernel/src/advanced_profiles.rs) checks
path alignment and section correspondence before adopting a result. Unsupported
geometry reports a diagnostic and leaves the draft editable. Failed previews do
not mutate the document or replace its last valid model.

Stable authored origins identify pattern members and input topology. Removing a
referenced member requires repair. Native ordinals, nearest-face guesses and
display triangles do not replace missing references.

Native project containers persist the sources and expressions, so reopening
rebuilds the geometry. Manufacturing exports consume those exact solids. A valid
solid does not establish that a design is manufacturable or collision-free.

## Verification entry points

- [Compiler regressions](../../crates/multicad-model/tests/advanced.rs) cover
  parameter changes, linked Instances, saved inputs, removed references,
  Undo/Redo, source-neutral failure and native STEP round trips.
- [Native profile tests](../../crates/multicad-kernel/tests/advanced_profiles.rs),
  [solid tests](../../crates/multicad-kernel/tests/advanced_solids.rs) and
  [concurrency tests](../../crates/multicad-kernel/tests/advanced_concurrency.rs)
  exercise supported and rejected native results.
- [Inspector tests](../../crates/multicad-ui/src/modeling/controller/advanced_tests.rs)
  run actual controls against the production compiler.
- `cargo run -p multicad-app --example advanced_review -- NEW_DIRECTORY`
  creates 18 editable input/result projects and a metrics manifest. It refuses
  an existing directory. Generated fixtures support review; they do not replace
  interactive platform checks.
