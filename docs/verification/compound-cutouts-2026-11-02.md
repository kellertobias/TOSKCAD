# CAD-247: Compound cutout verification

Historical implementation evidence. The
[2026-11-05 native-window report](compound-cutouts-2026-11-05.md) closes the
macOS interaction gap recorded below and adds actual saved geometry/PDF checks.

Status: implementation, full workspace tests, visual checks and desktop build/CLI
verification complete. Actual macOS window interaction remains unverified.

## Implementation

The existing Cutout tool now offers **Use multiple boxes**. One retained source
owns editable boxes and explicit receiving wall/slab Parts. Generated cuts append
to those Parts' histories. Native evaluation proves every receiver before the
transaction can commit, including when only one generated member was requested.

The editor supports graphical add/move/resize/remove, typed dimensions and
vertical overrides, named receivers and shared height, Preview/Cancel/Apply,
Undo/Redo and reopening from a generated receiver. The rectangular path remains
available. See the [source and MCP contract](../architecture/compound-cutouts.md).

| Local main commit | Change |
| --- | --- |
| `b9a3af6` | Canonical source, stable receiver histories, exact native cuts and tests |
| `14165f4` | MCP preview/set/inspect/remove and contract documentation |
| `314e60f` | Compound editor, native receipt gate and graphical-control tests |
| `c958087` | Visible outline overlays, separated labels and compact inspector |
| `3c1800a` | Explicit floor-range native acceptance case |
| `523d382` | Read-only preview discovery and attributable Previewed audit history |

## Automated evidence

Focused checks and the final workspace run pass four core transaction/persistence
tests, seven native compound tests, one layered-wall material/STEP test, one MCP
workflow test and two UI tests. Native coverage includes L-shaped slab holes, wall-only/
slab-only/combined selection, unchanged non-targets, native refusal without source
commit, scoped evaluation proving another receiver, downstream rebuild/removal,
persisted linked plan/section views and receiver-height changes.

The layered-wall case retains five physical material components, assigns every
new cut face to its source layer and reopens the exported STEP solid. The MCP
case retains proposed identities from Preview through Apply, checks every output,
and covers inspect/edit/remove, stale revisions, native failure and Previewed
audit history without a committed revision. UI tests use
painted controls, viewport points and the production native worker's receipt.

| Required check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed on final implementation `523d382` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed on `523d382` in 2m51s |
| `cargo test --workspace --locked` | Passed: 2305 passed, 0 failed, 4 intentional ignores across 258 groups |
| `./build` and desktop CLI diagnostics | Passed: build 1m05s; `--version` and `--diagnostics` both exit 0 |

The four intentional ignores are the subprocess-only interrupted-writer helper,
the optional Blender import and two opt-in physical-GPU tests. The parent writer
test runs normally. The six production Metal captures below provide separate GPU
evidence for this editor; they do not claim those ignored tests were executed.

The development bundle is
`artifacts/development/build-pfbYDr/MultiCAD.app`. Diagnostics identify macOS
aarch64, OCCT 7.9.3 statically linked, eframe 0.36.2/wgpu, and compiled Rust/native
runtime without Python or Node.js. The development binary reports its source
commit as **not stamped**; this is not release-provenance evidence. Source checks
and build used implementation commit `523d382`.

## Production visual review

The Rust review fixture writes
`artifacts/compound-cutout-review-20261102/compound-cutout.tcd`, three STEP receiver
files and `native-report.json`. It compiles the complete candidate before commit
and independently reads each STEP file back. Two 6000 × 4000 × 200 mm floor slabs
lose the same 0.64 m² L-shaped footprint. A 4000 × 200 × 2500 mm wall receives a
50 mm-deep chase. Result volumes are 4,672,000,000 mm³ per slab and
1,875,000,000 mm³ for the wall. The cutter is not a fourth final Part.

The production Application, native Backend and Metal renderer produced six
inspected images in
`artifacts/compound-cutout-review-20261102/captures-final/`: 3D and plan, each at
1100 × 700 @1×, 1360 × 860 @1× and 1360 × 860 @2×. PNG dimensions match the DPI
settings. The GPU is Apple M5 Max / Metal. The capture harness checks that box
labels follow the native callback, do not overlap, and that all four box actions
are unclipped. It clicks Preview and waits for the native receipt to enable Apply.
Source stays unchanged, with reopened revision 0 and scene generation 1.

The first captures exposed outlines hidden behind the native solid pass and
small-window clipping. The final captures verify the paint-order fix, contrast
labels, selected-receiver summary and compact controls. Earlier capture folders
are retained as superseded local artifacts.

## Remaining acceptance

These checks create no OS window. Native macOS pointer/keyboard interaction is
unavailable and has not been replaced by offscreen evidence. Human acceptance
and Windows/Linux CI runtime evidence remain separate. No new QA windows were
opened; the user's existing window was untouched. No installation, push or
publication was performed. All changes are local on main, and unrelated
`.claude/` content is preserved.

Remaining native-window acceptance covers graphical creation of the L-shaped
cutout, editing each box and receiver/level choice, Apply/Cancel, history and
save/reopen in the application. The implementation is handed off for that
verification; the agent has not marked it accepted.
