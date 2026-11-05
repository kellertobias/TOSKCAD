# Spherical-involute bevel gear verification

Commits `4d4e2ec` and `e3f03c4` on `main` implement native straight bevel solids,
canonical recipes, intersecting-axis pairs, UI and MCP controls. Commit `daeea7b`
preserves native convergence across existing solid families. Focused native
checks, actual macOS workflows and exchange readbacks pass. The final locked
workspace suite passes 2,664 tests with zero failures and four intentional
ignores across 304 groups on `daeea7b`.
No installation, push, publication or human acceptance is included.

## Declared construction

The [gear reference](../architecture/gears.md#spherical-involute-straight-bevel-subset)
owns equations, primary source links and limitations. The declared system uses
Ligata/Zhang spherical-involute straight flanks, KG/KHK back-cone heel/toe
boundaries and explicitly custom tangent spherical-circle roots. It supports
20-degree pressure, 90-degree shafts, 24–64 teeth per member and cone face width
at most `min(R/3, 10m)`, with native toe bore/wall checks. These limits define an
engineering subset, not a general undercut theorem. Gleason/octoidal, spiral and
Zerol systems remain unavailable in both interfaces.

The canonical typed dimensions retain mate count, shared shaft angle, bore and
thinning intent. Pair edits update both recipes atomically and preserve existing
assembly relationships. Reader capability `model.gear.bevel.1` prevents older
readers from silently accepting direct, suppressed or template-only bevel source.
The [MCP reference](../architecture/mcp-server.md#gear-control) specifies explicit
input refusal, omitted/null values and coupled pair edits.

## Native evidence

Artifacts are under `artifacts/bevel-review-20261105/`. Six final kernel tests
pass in 23.39 seconds on the correction. Independent probes measure spherical flanks, root
transitions, back-cone boundaries and volume, including native STEP readbacks.
Coverage includes both asymmetric 24/64 orders, zero and positive bores, rigid
poses, large translations and scale. Separate known-volume controls cover an
off-center enclosed void with two closed shells and repeated compound members.
Impossible width, bore, tooth-system and incompatible pair inputs are refused.

Model checks perform whole-solid overlap tests at the current pose and nine
sampled phases. A close pair of actual named native flank faces can establish
proximity; a miss falls back to whole-solid minimum distance. This is an
existence witness, not a global-minimum or loaded-contact certificate. Extreme
64/24 turn-limit fixtures use the reported whole-solid fallback. Collision and
separation negative controls pass without widening budgets. Finite samples do
not prove continuous collision-free motion.
An earlier extreme-pose test incorrectly required the flank-face fast path at
every pose. Its failed log is retained; the corrected test checks the actual
reported witness scope and unchanged distance budget, including fallback.

## Actual macOS workflow

The first 56 captures use locally assembled source `e3f03c4`, binary SHA256
`5edc84322c44c8fa2e5cf7cfca1b34939a8228f08d69fae2621377aa83146f3c`.
`runtime.json` and timestamped `screens/manifest.jsonl` identify that executable
and its actual window captures. The final `daeea7b` bundle has binary SHA256
`8875c3f01547a4a17e95915553c111c45a9e8be16d65ef1c3c2f5b3d35a71b67`,
recorded separately in `runtime-point-retry.json`. Offscreen UI tests are
separate evidence.

| Actual operation | Observed result |
| --- | --- |
| Single bevel | Module 1, 24 teeth, 32-tooth mate, zero bore and 3 mm cone width reach native Preview. Cancel retains revision 0. Repeat Preview/Apply reaches 1; toolbar Undo/Redo reach 2/3 and Save reaches 4. |
| Invalid intermediate typing | Mate count 3 disables Preview/Apply; typing the next digit retains focus and accepts 32. |
| Pair creation | 24/32 teeth, module 1, 2/4 mm bores, shared apex [10, 15, 0] mm and 10-degree drive reach native Preview/Apply. Toolbar history reaches 2/3 and Save reaches 4. |
| Drive-only edit | Preview at 20 degrees followed by Cancel retains revision 4 and the original 10-degree intent. Repeat Preview/Apply reaches 5; toolbar history reaches 6/7 and Save reaches 8. |
| Geometry edit | Enabling Update both gear recipes exposes retained geometry. Shaft angle 60 degrees disables Preview/Apply. Restoring 90, changing module to 2 and first count to 28 derives 41.1859/48.8141-degree pitch cones. Apply reaches 9 and Save 10. |
| Cold reopen | A fresh application session rebuilds the saved edited pair. Its inspector retains module 2, 28 teeth and the declared shaft/width/bore values. Cancelling inspection leaves the source unchanged. |

The four saved TCD files retain actual UI-authored source. Independent readback
finds one physical leaf in `native-single-bevel.tcd`, volume
1426.3168715237296 mm3. The three pair files retain all 14 entity IDs,
assembly/member/joint relationships and source ownership. Drive-only recipes
are identical; their summed native volumes agree within 7.1e-9 mm3. Geometry
editing changes exactly the two modules, first tooth count and reciprocal mate
count, apart from file metadata. Independent cone and phase calculations match
all three receipts, including cone distance 20 → 42.52058325093859 mm.
These literal GUI fixtures do not prove expression editing; typed expression
coupling has separate contract tests.

Pre/post cold-reopen native receipts are exactly equal, including hashes,
recipes, identities, poses, bounds and volumes. Source bytes remain unchanged.
`final-window-inventory.json` records no running ToskCAD QA applications.
Captures 57–62 separately verify cold reopening of the pair, retained editing
and the zero-bore single in the final `daeea7b` bundle.
`final-window-inventory-point-retry.json` records the final cleanup.

The unbundled release executable's read-only MCP stdio session initializes, discovers
the generated tools, inspects this saved pair and evaluates its native model.
`final-read-only-stdio-report.json` retains binary identity, raw protocol and
unchanged input hashes. It reports two physical leaves, two solids in the placed
assembly, the retained cone context, current pose and nine sampled phases with
zero overlap. The owned process exits normally. The initial artifact checker
incorrectly expected solid counts on leaf descriptors; that diagnostic remains,
and the corrected checker reads the actual assembly receipt without repeating
native work. These transport receipts are separate from GUI evidence and are
not uncontended latency measurements.

## Manufacturing exchange

`manufacturing-edited/report.json` records two placed STEP solids, one 3MF and
two STL files generated from the actual saved edited pair. Independent native
STEP reloads each contain one solid. Maximum bounds delta is 6.58e-12 mm and
maximum volume delta is 2.78e-7 mm3, within the unchanged declared budgets.
Raw 3MF/STL readbacks preserve two physical parts; 3MF contains 88,256 triangles.
Mesh volume differs from native by approximately 2.956 mm3 out of
22,494.255 mm3, within the declared tessellation budget. These are placed
occurrences, not a Boolean union or a print arrangement certification.
The package preserves model coordinates; one member extends 33.317 mm below
the build plane, as the export preview warns. Print-bed positioning remains
necessary.

The independent raw topology audit checks all five payload hashes and saved
source bindings. Both STEP shells have a connected face graph and two opposite
bound uses of every edge. Both transformed 3MF objects and both STL objects are
closed and oriented at exact coordinates, with no collapsed triangles and one
connected surface each; no welding is needed. The actual 3MF transforms match
the retained pair layout. These checks do not establish vertex links or absence
of self-intersection. No physical fit, strength, lifetime, cutter compatibility
or fabricated print is certified.

## Shared volume integration and metric cache

Zero-bore testing exposed a stall inside native GK volume integration.
[ADR 0033](../architecture/0033-oblique-volume-integration.md) records the
three-basis flux formulation, pinned shell/reference behavior and limits of
the diagnosis. An earlier single-plane alternative failed the unchanged metric
cache consistency guard; its failure and diagnostics are retained. It was not
installed, and no mass/error budget was widened.

The accepted integration preserves the original point integrand by averaging
three orthonormal fluxes at the same per-shell reference. Independent adaptive
solves can nevertheless report different error estimates. The original native
point integration is retried only when the finite positive basis result exceeds
the existing error limit, and must independently pass that unchanged limit.
It does not rescue topology or nonfinite failures and does not cache rejected
results. The native point retry has no public global cancellation/time budget.

The first full run exposed preferred continuous M6 thread construction being
rejected by the basis error estimate, selecting a bounded alternative whose
nut missed its existing analytic comparison. Retained native comparisons
demonstrate the change of construction. The corrected mechanical-part target
passes all four tests at the original budgets; an opt-in trace confirms that
the continuous cutter is retained. All five transfer tests also pass with an
independently accepted point retry. Six bevel tests pass in 23.39 seconds;
the zero-bore trace passes without entering the point retry. Three cache
ownership tests and five prototype restoration tests pass on the correction.

Final fresh generation covers 19 actual catalog recipes and 15 unique prototypes
in 23.091079 seconds with eight workers. All 15 BRep files remain byte-identical,
totaling 11,294,004 bytes. The initial mass metadata change is at most 1.73e-10
relative; the final point-retry refresh changes only the fingerprint, with all
receipt masses unchanged. The trusted fingerprint is
`6eacf22f8f802c8d4f64240a87c36fd68d5332d07161478dc647015921f19939`.
The raw audit verifies source closure, hashes and recipe mapping; native
generation and Rust restoration tests supply separate geometry evidence.

## Required checks and remaining limits

Formatting, strict locked workspace all-target Clippy and the final release
build pass on the corrected source. Packaged diagnostics and runtime self-test pass.
Three focused UI tests pass in 16.25 seconds on the initial kernel. The full
workspace run on `e3f03c4` completed with 2,661 passes, three failures and four
intentional ignores across 304 groups. Its logs are retained under `checks/`.
The cabinetry failure compared native volumes bit-for-bit across rebuilds;
their four-ULP difference is now checked with the existing absolute `1e-6`
comparison, while every other field and record order remain exact. All nine
cabinetry tests pass. The other two failures were the nut and transfer convergence
cases described above. The complete rerun on `daeea7b` exits 0: 2,664 passed,
zero failed and four intentional ignores across 304 groups. The command adds
`--no-fail-fast` to collect every target result without filtering tests. It uses
the pinned toolchain and lockfile, optimized test code, debug assertions and
overflow checks. `checks/point-retry-workspace/` retains the complete log,
command, environment, source identity, timestamps and summary. It ran from
18:01:53 through 18:53:04 UTC. These are developer verification durations,
not application latency. The full UI target passes on the corrected source.

Actual macOS checks use a 1360×860 logical window. Existing inspector-edge
clipping belongs to CAD-209; compact-window acceptance and Windows/Linux runtime
verification are not established here. Preview and Apply still repeat some
native validation, tracked separately by CAD-232. Focused test durations are
not application latency or p95 measurements. This report records engineering
verification and does not record human acceptance.
