# Architecture navigation verification, 2026-11-02

> Historical evidence. The [native review of 2026-11-05](architecture-navigation-2026-11-05.md)
> records actual macOS interaction and the subsequent Library highlight correction.

CAD-223 reorganizes Architecture commands and placement categories on `main`.
The canonical Junction box foundation is `35802e0`; navigation and typed forms
are `7d3b5a2`; the visual corrections are `b81dbf8`; native offscreen review and
scale tests are `23d881b`. See the
[current contract](../architecture/architecture-navigation.md).

Actual macOS window interaction and human acceptance remain unverified. The
painted pointer tests and offscreen Metal images below do not establish that
gate. No OS windows were opened or applications installed for this item.

## Behavior and source checks

| Requirement | Evidence |
| --- | --- |
| Root commands | Wall, Door, Window, Cutout, Levels and Stairs; no separate Shaft tile; Stairs excluded from Common |
| Nested layout | Actual parent controls move into a 24-point cue; Back starts the single full child row; Close returns to the parent |
| Placement categories | Water, Electrical, Heating, Furniture and Landscaping expose real commands; Electrical starts with Switch, Socket and Junction box |
| Existing families | All 22 canonical equipment families remain reachable; connections, lighting, fixtures and appliances have separate groups and domain glyphs |
| Typed entry | Family shortcuts seed the existing form; the generic Equipment command remains available; Stairs reuses existing native stair authoring |
| Source-neutral navigation | Pointer tests check repeated entry, switching, Back/Close and pending-form preservation; Cancel leaves source and revision unchanged |
| Context and search | Per-project state and saved subpaths restore without executing commands; unknown children restore Architecture; Wood defaults and the hardware washer alias are preserved |
| One Library | Category entries use the existing included/user browser; no duplicate definitions or catalog store |

The focused source/native foundation checks passed: core catalog 2, native
all-family envelope/plan test 1, MCP equipment 3, and Library pin equality 1.
The native all-family export test passed in 65.42 seconds, checking scaled
footprints, fixed-paper marks and category visibility across all 22 families.
All 21 previously published equipment files were byte-for-byte unchanged.
The new Junction box has a separate enclosure and cover and an authored JB mark.
Its generated Library preview was inspected.

The UI checkpoint passed 873 tests with 0 failures and 2 intentional ignores in
23.79 seconds. After the visual corrections, three focused navigation tests
passed in 0.41 seconds, including actual egui pointer input at 1100 × 700 and
1360 × 860 with native 1×/2× scale, saved context and pending-form cancellation.
The final workspace results below supersede these intermediate checkpoints.

## Visual review

`artifacts/architecture-navigation-review-20261102-final/` contains 18 images:
root plus all five categories at 1100 × 700 / 1×, 1360 × 860 / 1× and
1360 × 860 / 2×. All were inspected. Tool labels, category controls and Back/Close
fit. The compact cue retains the real parent commands; there is one full child
row, rather than a permanent second full ribbon. The actual cabinet geometry
remains visible through the production native backend and viewport renderer.

`report.json` records the source project, GPU, logical sizes, native scale and
scene stamps. PNG dimensions were checked against those sizes and scales. The
read-only renderer asserts unchanged canonical source/revision and equal native
scene stamps before and after navigation. Every final image retained revision 0
and native scene generation 1. This is a consistency check, not a measured latency
or backend-request-count claim.

`artifacts/architecture-project-regression-20261102/` contains three inspected
Project-mode images at the same sizes/scales. The parameter table, left actions,
file metadata and preview remain present; Project has no ribbon.

Reproduce the Architecture matrix with:

```sh
cargo run -p multicad-ui --locked --example project_shell_review -- \
  examples/cabinet.tcd artifacts/architecture-navigation-review --architecture
```

Omit `--architecture` for the original Project review. The adapter rejects project
and Library writes. It installs the same viewport resources as native windows,
uses a Depth32Float attachment and supplies native input scale. Pointer positions
use egui logical points. Initial trials omitted viewport resources and used the
wrong 2× pointer conversion; those incomplete captures are superseded by `final`.
The review also led to clearer appliance/lighting/plant glyphs and removal of the
unintended Common/Stairs entry.

## Final required checks

The pinned Rust 1.99.1 toolchain and committed lockfile are used. Source is frozen
at `23d881b` for the first full pipeline. That run compiled in 19m 21s
and found one stale Library integration count: 27 bundled entries instead of
28 after Junction box. The test now expects 28 included entries and derives
register/remove counts from that checked included list. Its original-file/index
isolation assertions remain intact. The focused regression passed in 1.67 seconds.
Correction `5f75365` is the source freeze for the corrected full pipeline.

| Check | Status |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Corrected freeze passed in 7.22s |
| `cargo test --workspace --locked` | Passed: 2,269 / 0 failed / 4 intentional ignores across 255 groups |
| `./build` | Passed; native compile 38.09s |
| Built binary `--version` / `--diagnostics` | Both exited 0; macOS/aarch64, OCCT 7.9.3, native Rust runtime |
| Documentation links and `git diff --check` | Passed; no broken local links |

The final UI suite passed 873 tests with 0 failures and 2 intentional ignores
in 25.31 seconds. The built application is
`artifacts/development/build-Pnptj0/MultiCAD.app`. Diagnostics report
`Source commit: not stamped`; this development bundle is not release provenance.
It was not installed, published or opened as an OS window.

## Remaining acceptance

Actual macOS review still needs normal/compact navigation, family placement and
field changes, Back/Close, cancellation, project recall and the shared Library.
Windows/Linux runtime evidence is separate. This item does not establish the
Model 2D/3D and level workflow (CAD-246), compound box cuts (CAD-247), Wood grouping
(CAD-248) or the configurable-face Box assistant (CAD-249).
