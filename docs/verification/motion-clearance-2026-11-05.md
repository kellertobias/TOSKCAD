# Native Motion, required clearance and sampled poses — 2026-11-05

CAD-240 adds a required minimum clearance to inspection and displays the actual
native assembly pose for a selected range sample. Geometry remains in the same
canonical document. Inspection and sample display do not commit joint values.
This report records agent verification; human acceptance remains separate.

## Implementation and runtime

- `c18e787` adds the requirement, shared sample preparation, native scene display,
  source guards, selection highlighting and regressions.
- `54afadf` wraps and contains result labels inside the viewport after actual
  native use exposed clipping on the longer contact message.
- Both are local Conventional Commits on `main`. No installation, push or
  publication was performed.
- Final native bundle:
  `artifacts/assembly-clearance-review-20261105/optimized-final/MultiCAD.app`.
  Executable SHA-256:
  `3352778fc3fe1a967b7ca81ff27b9fdcdcfc969f9d74d13c8be04c7ef5e7d35b`.
- Captures 01–29 use the first reviewed native build (`c18e787`, executable
  `6faac61cf52e83cc505efe615b3894da015677f2ab4216592d0961f16484c831`).
  Captures 30–37 use the final build. Both were assembled from the corresponding
  release executable with the repository's `assembleRuntime` function.
  Diagnostics are retained under the artifact root's `checks/` directory.
  Diagnostics do not stamp the Git commit; these are locally identified builds,
  not release provenance.

The required minimum is local inspection intent, entered in millimeters or as a
parameter expression. It is separate from contact-classification tolerance and
exact Boolean overlap. It does not add a persistent assembly constraint or move
parts. Nonfinite and negative requirements are refused. A positive separated gap
can fail the requirement; physical overlap always remains interference.

Native range inspection and selected-pose display share
`MotionRangeQuery::prepare_sample`. The display worker receives an uncommitted
candidate. Its actual source hash must match the selected range receipt; the
complete scene, request generation and native leaf placements must also match.
Current source or resource changes, input changes, Return to current pose and
Cancel invalidate the display. Failed requests require an explicit selection to
retry. Native solids stay on their existing owner worker; the UI receives the
detached scene. No continuous collision, dynamics or between-sample safety claim
is made.

## Exact fixture and observed native results

The owned fixture was prepared with public typed Rust transactions and the
`ProjectFile` writer, not by rewriting project JSON. It contains two 10 mm cubes
with identical XY extents, a slider constrained to inclusive Z limits 0–12 mm,
and one placed assembly. The fixed cube occupies Z 0–10; the moving cube occupies
Z `z`–`z+10`. Initial `z` is 10.5 mm.

`native-slider-clearance-named.tcd` has SHA-256
`b67934454247bc05bdc65114cef49cccef62d98ed7b8c75f497fe21ab5064621`.
The preparation source and independent preparation receipt remain beside it.

Actual macOS actions selected the two named physical occurrences, entered a
1 mm requirement and ran the native inspector. The following numbers also agree
with the analytical cube geometry:

| Pose | Native minimum gap | Native overlap | Requirement of 1 mm | Capture |
| --- | --- | --- | --- | --- |
| Current, Z 10.5 | 0.5 mm | 0 mm³ | Below by 0.5 mm | 02 |
| Sample, Z 9 | 0 mm | 100 mm³ | Interference | 05 |
| Sample, Z 10 | 0 mm | 0 mm³ | Below by 1 mm; contact within tolerance | 06, 33–34 |
| Sample, Z 10.5 | 0.5 mm | 0 mm³ | Below by 0.5 mm | 07 |
| Sample, Z 11 | 1 mm | 0 mm³ | Meets requirement, including equality | 08, 10 |
| Saved current, Z 11 | 1 mm | 0 mm³ | Meets requirement after cold reopen | 32 |

The range was explicitly 9–11 mm in 0.5 mm steps. Selecting samples visibly
changed the cube's native placement and witnesses while the canonical revision
stayed r0. Fit result and Reveal targets were operated in capture 10; Return to
current pose restored the saved 10.5 mm placement and removed the markers in 11.
A negative requirement was refused with an explicit diagnostic in 03.

The initial contact label clipped at the viewport boundary in 06. The final
build's 33 shows the complete contained label; 34 shows it wrapped inside the
compact 1100 × 700 logical-pixel window, with the fixed Inspector footer still
reachable. This is not evidence for the planned 1024 × 600 minimum.

## Pose editing, history and persistence

Actual Joint motion actions refused Z 13 against limits [0, 12] without enabling
Apply (12), accepted the inclusive Z 12 endpoint in native Preview (13), and
preserved the pending value and preview through Model/Motion switching (14).
Cancel restored the original geometry at r0 (15).

Applying Z 11 made one transaction, r1 (16). Toolbar Undo restored the original
pose at r2 (19); toolbar Redo restored Z 11 at r3 (20). The separate review file
was saved through the native Save As dialog (21). Capturing the current joint
values as “Clearance 1 mm” added a named pose at r4 without moving the assembly
(22–24). A later Z 9 edit made r5; selecting the saved pose, Preview and Apply
restored Z 11 in one transaction, r6, and Save retained it (25–29).

The final build was cold-launched, reopened the saved file through Recent and
showed Local Z 11 with the same travel limits (30). The named pose remained
selectable (31). Model/Motion switching retained the selected Moving cube
occurrence, its owning component and its definition at r0 (36–37).

`native-motion-edited.tcd` has SHA-256
`1f70ad5eb4039d1cd01583987cdd44a67671c7c386252f89a6b4b8e99c521da2`.
`independent-native-motion-edited-readback.json` checks the saved project with the
exact final runtime and public core reader. All cube geometry, occurrence IDs,
other existing entities and other joint parameters are unchanged. The existing
joint has Z 11 and its type-derived name normalized from “Z slider” to “Slider”;
that name change is recorded, not ignored. One named-pose entity was added and
stores the same joint at Z 11. Native readback reports the two leaves at Z 0 and
11. Generic pair distance is not exposed through MCP, so the audit distinguishes
its analytical gap from the actual native UI distance and witnesses above.
The original named fixture's hash remains unchanged.

## Checks and evidence limits

The final focused native inspection suite passes 13 tests, including policy
classification, invalid requirements, actual differing sample scenes, late and
wrong-scene replies, explicit retry, current-source invalidation and the small
viewport label regression. Required final workspace formatting and strict
all-target Clippy pass. The optimized native build passes.

The final `cargo test --workspace --locked` run exits 0: **2,595 passed,
zero failed, four intentional ignores across 295 result groups**. The final
run uses the pinned toolchain and committed lockfile, with optimized Rust test
code and debug/overflow assertions retained. `checks/workspace.log`,
`checks/workspace.exit` and `checks/summary.json` retain the complete result.
An earlier superseded run was deliberately interrupted (exit 130); it is not
used as complete verification.

The artifact root is `artifacts/assembly-clearance-review-20261105` and contains
37 JPEG/accessibility pairs, their hash manifest, project files, independent
receipts, checks and an empty closed-window inventory. Captures 09, 17 and 18
retain intermediate attempted interactions; the definitive Fit/Reveal and
history evidence is 10, 19 and 20. No keyboard-shortcut success is claimed from
those attempts. Controlled native tests cover stale replies, unavailable/failed
samples, range budgets and nonuniform endpoint inclusion; these are separate
from the interactive paths explicitly listed above. Relationship-kind semantics
and transactional save/reopen also retain the existing native assembly tests.
No physical simulation, manufacturing export or Windows/Linux runtime result is
claimed by this macOS review.
