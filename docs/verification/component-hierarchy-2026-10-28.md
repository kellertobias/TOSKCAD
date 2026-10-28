# Native component hierarchy check

CAD-114 supplies containment, active ownership and visibility for subsequent
solid-operation cut scoping. These checks use native OCCT solids and the shared
transactional document API. The fixture is an isolated acceptance example;
it does not implement production box or cutting commands.

## Reproducible fixture

Commit `a0c0b93` adds the dedicated `hierarchy_review` example. Its six canonical
bodies each have a distinct producer and a separately built native B-rep.
One additional explicit Instance reuses `Visible side A`.

| Body | Initial owner | Initial visibility |
| --- | --- | --- |
| Visible side A | Cabinet A | Visible |
| Grouped shelf A | Cabinet A, through group Panels | Visible |
| Hidden side A | Cabinet A | Locally hidden |
| Ancestor-hidden back A | Cabinet A, through Hidden stock | Hidden by group |
| Drawer bottom | Cabinet A / Drawer module | Visible |
| Sibling top B | Cabinet B | Visible |

The baseline therefore has six bodies, six native producers, seven occurrences
and five visible occurrences. Active component `Cabinet A` has two default
visible source-body cut candidates: its visible side and grouped shelf.
The nested drawer and sibling cabinet each have their own candidate population.
Hidden bodies are excluded. Reusing an Instance does not add another canonical
body to this population.

The example can generate and reread a native project without opening a window:

```sh
cargo run --locked --offline -p multicad-app --example hierarchy_review -- \
  --headless --write-project artifacts/hierarchy-review/independent-bodies.multicad
cargo run --locked --offline -p multicad-app --example hierarchy_review -- \
  --headless --project artifacts/hierarchy-review/independent-bodies.multicad
```

The write uses no-clobber creation: select a new path if the artifact exists.
The saved baseline SHA-256 is
`a74df0a360b69516d52528e13ce9cdf21a879ae7529b2d46e0a1a7e9a7e2b2c7`.
The generated and reopened reports in `artifacts/hierarchy-review/` match.

## Physical macOS check

The executable
`artifacts/development/MultiCAD-hierarchy-review.app/Contents/MacOS/hierarchy_review`
through `6db1f0f` had SHA-256
`d93093274edd3b86194d3dd719fbbfe1d043814bf65a86ec58d271dff689ca77`.

The viewport displayed five actual solid occurrences. The model tree showed
the nested and sibling components, groups, body histories and both visibility
cases. Selecting the ancestor-hidden back showed its owning component and
material. Unhiding `Hidden stock` restored the additional solid and its
selection highlight at revision 1. Native Undo hid it again at revision 2.

Renaming `Panels` to `Panel stock` through the Inspector committed revision 3.
Moving `Visible side A` into `Cabinet A / Drawer module` committed revision 4.
Its owning-component label changed; both its original occurrence and explicit
reused Instance stayed visible and selected. This state was saved separately as
`artifacts/hierarchy-review/independent-bodies-edited.multicad`.

The native container reader and fresh headless OCCT evaluation then confirmed:

- Six canonical bodies, six native producers, seven occurrences and five
  visible occurrences remain.
- Body and producer identities, native volumes, native bounds and local and
  effective visibility remain identical.
- Every source entity except the renamed group remains identical, including
  body feature histories, material references and the reused Instance placement.
- Active Cabinet A now has only the grouped shelf in its default cut population.
- The nested drawer now owns both its bottom and the moved side. Cabinet B's
  population remains unchanged.

The comparison report is
`artifacts/hierarchy-review/independent-bodies-edited-comparison.json`.

## Final native recheck

This check exposed Inspector width growth with a long nested-owner path and
multiple occurrence actions. Commit `64b0bb7` adds a shared wrapping policy and
bounded, truncated container selectors. Its regression runs 960 frames at
180/250 logical pixels and standard/Retina scale, including selection of a long
parent and ordering entry, Apply, and the resulting shared-body summary.

The replacement hierarchy-review executable had SHA-256
`1cd609a9290709d904e85a8a8643fe68225d5f34425d5a6a7f2892f0de1809eb`.
Physically reopening `independent-bodies-edited.multicad` restored the renamed
group, moved body, active component and both hidden cases. Selecting the moved
body displayed its nested owner and two occurrences without widening the
250-pixel Inspector. The long nested path also stayed within that width in the
Move form, with its full value available in the popup. Cancel preserved revision
0. Clear selection and Fit displayed all five visible occurrences.

CAD-114 can now request Review / test. This is agent verification, not human
acceptance.

The integration through `6db1f0f` passed workspace formatting, Clippy with
warnings denied and workspace tests: 428 passed, two ignored. The upstream
OCCT deprecated `sprintf` warning remains. The final integration through
`64b0bb7` also passed the required checks with 430 tests passed and two ignored.
This is macOS functional evidence,
with no Windows/Linux interactive result, low-power benchmark, production cut
command or human acceptance claimed.
