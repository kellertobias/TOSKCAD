# Architectural symbol verification, 2026-11-02

> Historical checkpoint. Recovered graphical wall/fence placement, pinned updates,
> receiving details and local-author-file-unavailable evidence are recorded in
> [Architectural symbol verification, 2026-11-04](architectural-symbols-2026-11-04.md).

CAD-206 adds project-authored layered-wall and fence blueprints, selected typed
public inputs, real receiving architectural outputs, embedded pins and explicit
revision changes. The implementation source is `a48357b` on `main`.

Actual macOS interaction for authoring, Library registration, drawing, editing
and updating remains unverified.
Automated evidence below does not establish that acceptance criterion or human
acceptance. No application windows were opened during this verification.

## Implementation record

| Commit | Change |
| --- | --- |
| `b0fb90a` | ADR-0029 ownership and source-capture design |
| `421bdb4` | Typed construction blueprint and audited source closure |
| `39f2801` | Selected dimension/material/option and numeric inputs |
| `a2990b6` | Atomic parameter-driven physical fence reconciliation |
| `0f857e0` | Real pinned wall/fence placement and editing |
| `03d75e3` | Composed receiving path/context repairs |
| `684353f` | Explicit pin comparison/update and identity/override repairs |
| `c5b8ca7` | Atomic receiving wall openings and new canonical opening types |
| `0ebf914` | Graphical authoring, public controls, Library draw actions and native Apply gates |
| `a48357b` | Typed MCP control, discovery and native joined-wall/hatch/equipment fixture |

The current contract and operator flow are described in
[architectural symbols](../architecture/architectural-symbols.md).

## Checks

The repository's pinned Rust 1.99.1 toolchain and committed lockfile were used.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, 3m16s on the final implementation |
| `cargo test --workspace --locked` | Passed: 2243 passed, 0 failed, 4 ignored across 254 groups, including doc tests |
| `./build` | Passed; native development bundle assembled |
| Documentation local links and `git diff --check` | Passed |

Focused checks before the final workspace run included 26 core architectural
source tests, 17 symbol UI tests, all 861 UI tests (two intentionally ignored),
all six native architectural-symbol tests and three architectural MCP tests.
The complete MCP suite passed 62 tests across 25 groups before the final result
metadata assertions; the final workspace run includes those assertions.

The first sandboxed workspace attempt stopped at three app-control tests because
their local Unix socket listeners were denied with `Operation not permitted`.
Those failures occurred before their control operations. The final workspace run
uses local socket access; no product source change was made for that restriction.

Development artifact: `artifacts/development/build-elpRc1/MultiCAD.app/Contents/MacOS/multicad`. The packaged binary's
`--version` and `--diagnostics` commands both exited successfully. No GUI window
was opened.

No install, push or publication was performed. The native build and tests are
macOS evidence; Windows/Linux runtime and CI results were not observed here.

## Observable behavior covered

| Contract | Evidence |
| --- | --- |
| Construction-only source capture | Core rejects sample Parts, sketches and unrelated geometry in a blueprint template; role/provider and ownership validation precede placement |
| Stable graphical author interface | Source authoring retains blueprint/slot/symbol identities across explicit revisions; painted exposure controls leave the document unchanged until native preview and atomic Apply; Undo restores the source |
| Independent receiving geometry | Native tests use different wall paths/heights and fence paths, independently measure solids and reopen TCD files without the source file or Library |
| Physical fence regeneration | Parameter changes and public edits reconcile posts/infills before native evaluation; retained semantic members keep IDs, while removed or changed families require valid reference handling |
| Wall openings | Pinned walls keep real windows through path/base/flip edits; placement can create a canonical opening type and wall together, then Undo the complete change |
| Joined faces/materials/equipment | A pinned branch meets the host's real solid face; sections emit authored material hatches; a wall-mounted outlet resolves to that face and 400 mm world height, retaining its pose through a public wall-height edit |
| Explicit updates | Tests cover compatible overrides, construction replacement, named layer changes, panel-to-rail member replacement, removed/retargeted inputs, material repairs, blocked references, cancellation, Undo and offline reopen |
| External control | MCP tests author/draw/edit, reject stale/cancelled/private input requests without document changes, compare without mutation, update with Undo and import/draw in a separate receiving project |
| Typed receiving inputs | MCP wall inputs select a project material and authored wall-reference option; a height expression retains the receiving parameter ID and recomputes actual native volume after the parameter changes |

The joined-wall fixture measures a 4000 mm × 120 mm × 2700 mm host at
1,296,000,000 mm³. A public height edit to 3100 mm produces
1,488,000,000 mm³. The MCP wall fixture independently measures
1,092,000,000 mm³ at 2600 mm height and 1,260,000,000 mm³ after changing its
receiving parameter to 3000 mm. These are native solid measurements, not display
mesh bounds.

Source comparison and source-only author/import results do not certify placed
geometry. Native preflight checks the complete prepared candidate before commit.
Graph-wide wall joins/openings keep the existing conservative wall-cache policy;
this change does not enable an unsafe wall evaluation cache.

## Remaining acceptance and provider limits

The actual macOS workflow still needs to demonstrate graphical source authoring,
existing file metadata/Library registration, two receiving wall paths with
independent heights, openings/joins/equipment and material sections; repeat with a
fence, edit its public inputs and post positions, explicitly compare/update the
Library revision, cancel and apply, use history, and reopen after the Library is
unavailable. Painted egui events and native compiler tests are separate evidence.
A native window controller was unavailable for that demonstration.

Current reusable providers are layered wall schema 2 and fence construction
schema 1. Uniform wall blueprints, arbitrary providers and role conversion are
refused. Existing supported wall-junction and bounded flat fence-path limits
continue to apply. MCP wall context edits preserve openings/cutouts/junctions;
existing wall tools edit those details separately, while the core API can compose
those repairs and new opening types in one transaction.

The work remains short of its complete acceptance criteria until the actual
macOS interaction is demonstrated. No Done or human-acceptance claim is made.
