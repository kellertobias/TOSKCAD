# Finite rack and pinion verification

The finite straight rack and its coupled spur pinion use the same canonical
source and native preflight through the Mechanical inspector and MCP.
Implementation commit `20fd1e2` on `main` is the source evidence. Focused core,
kernel, model, MCP and UI checks and actual macOS workflows pass. The full
locked workspace suite passes 2,643 tests with zero failures and four intentional
ignores across 300 groups.
No installation, push or publication is included.

## Supported geometry and motion

The [gear reference](../architecture/gears.md#finite-straight-rack-subset)
owns source equations, custom circular-root construction, backing convention,
4-64 whole-cell limit, role-independent angular/linear coupling and conservative
finite travel. The [MCP contract](../architecture/mcp-server.md#gear-control)
owns omission/default and explicit-input refusal semantics. Display meshes do
not supply manufacturing geometry.

Kernel tests measure actual native and STEP cap contours: straight flank
angles, pitch/thickness, tangent root circles, connected backing, bounds,
complete ancestry and independently integrated area/volume. The accepted upper
boundary is 64 cells. An initial 128-cell test exceeded the existing provenance
budget; its failure is retained and that count is explicitly refused.

Native model tests cover both member orders and odd/even rack counts with
independent local/base frames, two physical leaves, eleven sampled phases
including both finite endpoints, source edits, drive cache reuse, history and
saved reopen. Negative controls refuse a separated pair using actual native
proximity and diagnose positive material overlap for a half-pitch misphase.
3MF and STL files are read back from native-derived print packages; STEP bodies
are independently read back through the native kernel.

## macOS evidence

Artifacts are under `artifacts/rack-pinion-review-20261105/`.
Captures 01-22 use the initial runtime retained as `initial-native-binary` with
its digest in `initial-runtime.json`. Capture 18 records an Escape cancelling
the form after menu selection; it is not a successful pair-selection proof.

The single rack reaches native Preview, and Cancel retains revision 0.
Repeat Preview and Apply produce revision 1; toolbar Undo/Redo reach 2/3 and
Save reaches 4. The actual saved `native-single-rack.tcd` independently rebuilds
as one solid, with 21 cells, module 1 mm, width/backing 4 mm, design thinning
0.05 mm and fabrication thinning 0.02 mm. Native volume is
1357.023611195562 mm3; independent raw-source line/circle integration differs by
4.55e-13 mm3. Bounds are [-32.98672286269283, -5.25, 0] through
[32.98672286269283, 1, 4] mm. Input bytes remain unchanged by both audits.

Actual pair entry exposed focus loss while typing 21 through the temporary
invalid value 2. Explicit owner/field widget IDs now cover gear/pair names and
dimensions. The regression checks both roles, individual keystrokes, invalid
intermediate text and changing preview-status widgets without source mutation.
Other tools retain their existing field behavior. Capture 24 verifies normal
21 entry in the rebuilt runtime. Captures 23-61 use committed source `20fd1e2`;
its binary SHA256 is
`1a8e4a9fc809762593c34e74071577d9cf132e8334d37c8d452e2fb47c3d717f`.

| Actual workflow | Observed result |
| --- | --- |
| Rack A / Pinion B creation | Native Preview shows both bodies. Cancel retains revision 0. Repeat Preview/Apply creates revision 1; toolbar Undo/Redo reach 2/3 and Save reaches 4. |
| Finite drive refusal | Entering 1000 degrees diagnoses the allowed finite range and disables Preview/Apply at revision 4. |
| Drive edit | A valid 20-degree preview shows 4.1888 mm rack travel. Apply reaches 5, toolbar history 6/7 and Save 8. |
| Associative geometry edit | Module changes from 1 to 2 mm and rack count from 21 to 20. Native Preview derives 8.3776 mm travel at the retained 20-degree drive. Apply reaches 9 and Save 10. |
| Cold reopen | Capture 55 shows the initial rebuild; capture 56 shows the rebuilt native pair at revision 0. |
| Compact reversed role | At 1100x700 logical pixels, Pinion A / Rack B reaches native Preview and Cancel retains revision 0. |

The actual saved pair files and independent native/raw receipts establish two
physical leaves and all 14 retained entity IDs. `native-rack-pinion-created.tcd`
and `native-rack-pinion-driven.tcd` have identical geometry recipes and
2654.1104975817802 mm3 summed leaf volume; the drive changes from 10 to 20
degrees. Rack X changes from 12.094395102393195 to 14.188790204786391 mm at
base [10, 15, 0] mm; pinion center stays [10, 27, 0] mm. Odd-rack phasing and
finite travel of ±19.98672286269283 mm match independent formulas.

`native-rack-pinion-geometry-edited.tcd` retains those identities and changes
only the two modules and rack count, apart from file preview metadata. Its
summed native volume is 9558.57310379979 mm3. The rack moves to
[18.37758040957278, 15, 0] mm and pinion to [10, 39, 0] mm. Even-count phasing
and finite travel of ±36.83185307179586 mm are recomputed from source.
Independent rack integration differs from the native leaf by 3.18e-12 mm3.
Summed leaf volumes are not a Boolean union or contact certificate.

`export-created-verified/report.json` retains two placed physical-leaf STEP
readbacks and one 3MF plus two STL readbacks from the actual saved creation
file. It checks hashes, native/mesh dimensions, volumes, triangle counts and
format-appropriate ownership. An earlier artifact verifier incorrectly compared
STL object counts with the planned print count, although STL has no object/build
structure. That failed directory and diagnostic are preserved; only the verifier
comparison changed, with geometry budgets unchanged. Production code did not
change for that artifact-tool correction. A separate raw topology audit checks
both STLs and both transformed 3MF objects. Exact-coordinate deduplication gives
two oppositely directed incidences per edge, no boundary/nonmanifold edges,
no collapsed triangles and one connected surface per object. Precision welding
merges no additional vertices. This does not prove vertex links, absence of
self-intersection, physical fit or printability.

The pre/post cold-reopen native receipts match exactly, including file/source
hashes, recipes, IDs, poses, ownership, bounds and native volumes.
All completed QA windows are closed; `final-window-inventory.json` is empty.

## Trusted metric cache and required checks

The kernel export changes the trusted-thread source fingerprint. Generation
with eight workers covers all 19 catalog recipes and 15 unique prototypes in
19.131736 seconds. The independent audit confirms every B-rep is byte-identical
to its shipped predecessor, totaling 11,294,004 bytes. Only the registry source
fingerprint changes to
`3959346449bd6bb9b143f058266d1cb9c93bd600653eefa76e36ff7c62c2235c`.
Producer, audit and before-manifest receipts remain in the artifact directory.

Focused results: core 15, kernel 3, model 7, MCP 6 and UI 8 passed, with zero
failures. Separate native negative-control and flat-print readback tests each
passed. Earlier endpoint/budget failures and strict-lint findings are preserved.
Formatting, strict locked all-target workspace Clippy and the final release
build pass. The first full run stopped at an older all-supported-family fixture
that supplied rotary defaults for Rack. Test-only commit `fc20368` gives Rack
its required backing/zero bore, explicitly selects helical families, and checks
its actual rotated bounds. All three tests in that matrix pass. Production
files and the verified runtime are unchanged. The first run is preserved under
`checks/workspace-first-run/`. The complete rerun on `fc20368` exits 0:
2,643 passed, zero failed and four intentional ignores across 300 groups.
It adds `--no-fail-fast` to collect every target result without filtering tests.
The pinned toolchain and committed lockfile are used with optimized test code,
debug assertions and overflow checks enabled. `checks/workspace-run.json`,
`workspace-summary.json` and the complete log retain command, source identity,
timestamps and results. Packaged diagnostics and the headless runtime self-test
also pass without opening a window.

Finite sampled checks do not prove continuous collision-free motion, contact,
load capacity, hardware fit or manufacturing conformity. macOS interaction does
not establish Windows/Linux behavior or human acceptance.
