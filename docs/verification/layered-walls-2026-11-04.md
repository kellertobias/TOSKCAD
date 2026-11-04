# Layered-wall native interaction, 2026-11-04

Actual macOS interaction verified local layered construction authoring, graphical
wall paths, shared edits, independent types and cross-project pinned reuse.
[CAD-204](https://plans.tokenet.de/CAD/204) is ready for human review. Native
geometry, openings, junctions and drawing measurements remain documented in the
[earlier verification](layered-walls-2026-11-01.md).

The construction and persistence workflow used optimized, uninstrumented source
`ec8eefe` on `main`. The summary corrections and final graphical recheck use
`20001ff`. The final retained bundle is
`artifacts/layered-wall-review-20261104/optimized-review/MultiCAD.app`.
Its executable SHA-256 is
`651ed64fd801b85510dda553998a9524c3ee4accd998bf1c0e512cd9e2cb1f73`.

## Local construction and graphical paths

The session opened a separate copy of the existing material-plan fixture. It
supplied three material definitions and one reference wall; these were prepared
before this graphical check. The new construction began with that editor draft,
which contained paint, two insulation layers, two brick layers, a lining and an
air cavity. It was named `Exterior graphical review` through the native form.

Observed controls and native Preview/Apply operations:

| Action | Result |
| --- | --- |
| Select insulation row, Duplicate, Down, Remove | A distinct temporary layer appeared, moved after brick and was removed. Derived width changed 382.5 to 462.5 to 382.5 mm. |
| Edit cavity and insulation | Cavity width became 40 mm and the first insulation became 90 mm; total became 402.5 mm. Air had no material selector; coating had no finite-thickness field. |
| Flip construction | The editor changed to `Inside` followed by `Outside (flipped)` and mirrored its band preview. |
| Draw an L through viewport picks | Native Preview showed two joined segments, 1950 and 490 mm. Apply committed the wall and new type together. No path coordinates were typed. |
| Select the saved type and draw another wall | A separate 3900 mm straight wall used the same canonical construction. |
| Edit shared construction | The form listed both affected walls. Insulation changed 90 to 100 mm, giving 412.5 mm total. Native Preview and Apply succeeded. |
| Duplicate as a separate type | Only the second wall adopted `Exterior graphical review copy`, an Inside reference, renamed coating and 2600 mm height override. |
| Undo, inspect, Cancel, Redo | Two Undo operations restored the shared 402.5 mm construction. Inspection cancelled at the same revision; two Redo operations restored the later edits. |

Independent saved-container and read-only native MCP inspection confirmed the
shared update retained all seven ordered layer identities and all material
entities. The original fixture type remained 382.5 mm. After duplication, the
first wall retained its original authored type, flip and height; each type's
affected-user list contained only its own wall.

The source receipts are `independent-graphical-source-readback.json` and
`independent-type-source-readback.json` under
`artifacts/layered-wall-review-20261104/`. They retain immutable TCD snapshots,
hashes and canonical inspection responses. These readbacks establish source and
resolved properties; they do not independently measure boundary-representation
solids. The actual graphical operations used native Preview before Apply.

## Library publication and explicit revision update

The native Create symbol command selected the authored wall construction and
published a reusable layered-wall definition. Height was the sole exposed input.
Saving and Project / Add to Library registered the existing file in the single
installation Library. The session then created a blank receiving project,
loaded the registered file, imported revision 1 and used Draw wall.

Viewport picks created an 8000 mm receiving wall. Native Preview and Apply
succeeded. The receiving Inspector then set its public Height override to
2800 mm. The author changed its local default height to 2900 mm and published
revision 2 using the retained symbol and interface identities. Both immutable
definition entities remained in the authoring project.

Loading the revised Library file left the receiving session at r5 with embedded
definition Revision 1. Compare update displayed one changed entity and
one affected instance. The receiving override remained checked, while the
incoming authored default showed 2900 mm. Explicit Preview/Apply changed the
pin in one transaction.

| Definition | Verified pin |
| --- | --- |
| Revision 1 | `6d7b67788a83f86e7a01c947bfb8567852da4bf248d89f8911954b2914880f80` |
| Revision 2 | `49e9282f47977a3bc79b2fb1533fd2c354c4bc16ba8f469d1a4e8f5a44f40e50` |

Independent native inspections confirmed identical author/receiver definitions
at each pin, the unchanged receiving path and all 13 private source identities.
The revision update retained the 2800 mm override and existing output identities.
Both authored revisions remained in the source file. Reports are
`independent-published-receiving-r1-readback.json` and
`independent-published-receiving-r2-readback.json` in the artifact directory.

The final receiving snapshot is `graphical-receiving-r2.tcd`, SHA-256
`ffa81aba03e82f11244bc3147a17271b90c21cc6b82109aefb09e94a7fa8458e`.
The live filename retains an automation focus error:
`Untitled 3.tcdgraphical-receiving-wall.tcd`. Its contents match the snapshot.
After quitting and reopening the actual app, Recent projects reopened that file
at revision 0. Its Inspector showed the revision-2 pin, checked Height override
2800 mm and authored default 2900 mm. Cancel left the document unchanged, and
the live file retained its saved hash.

## Checks, limits and cleanup

Formatting, strict locked all-target workspace Clippy, the repository build
wrapper and the optimized workspace build pass on the final source. All 17
focused symbol UI tests pass. The full locked workspace run exited 0: 2,573
passed, zero failed and 4 ignored across 295 result groups, including
doctests. The test profile used optimization level 3 with debug assertions and
overflow checks enabled. The complete log and machine-readable totals are in
`artifacts/layered-wall-review-20261104/checks/`. These checks establish
correctness coverage, not application latency.

The earlier exact-solid and PDF/DXF fixtures remain separate evidence. This
graphical session did not create new drawing sheets or repeat the complete
opening and T-junction construction sequence. Its construction draft inherited
existing materials; it does not establish material creation through Project.
Native cross-project fence interaction and the broader architectural-symbol
acceptance criteria remain owned by [CAD-206](https://plans.tokenet.de/CAD/206).

Independent review of all six historical PDFs found no clipped plan, title block
or door swing. The 1:10 details distinguish material bands, cavities and T contact.
Thin finishes at 1:100 require that larger detail. No material legend or thin-layer
warning is printed on these sheets. The warning exists in evaluation source;
these exports do not establish its delivery to the operator.
`exported-sheet-review.json` retains file hashes, bounds and review limits.

Actual interaction exposed two incorrect summaries, corrected in `20001ff`.
Library cards count architectural dimension/material/option inputs alongside
numeric parameters and identify path-driven walls and fences. Update review
separates numeric parameters from named construction inputs, with cached before
and after authored defaults. Existing override and native validation controls
remain active.

The new optimized native app reopened the immutable revision-1 receiver and
displayed its typed input count. Comparing revision 2 showed Height, Before
default 2700 mm and After default 2900 mm. Cancel left revision 0 and the saved
file hash unchanged. Painted UI regressions also cover ordinary-symbol summaries
and retained 2800 mm receiving overrides through native Preview/Apply and Undo.

File-picker focus required raising the native modal before dismissal; no product
geometry failure was inferred from the automation attempts. All review windows
were closed through native controls. Final app inventory showed no running CAD
application. No installation, push or publication occurred. Human acceptance and
Windows/Linux runtime results remain separate.
