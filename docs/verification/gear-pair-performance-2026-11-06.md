# Gear-pair reuse and cancellation verification

On the matched herringbone workload, two unchanged Print previews changed from
47.100/47.286-second observed upper bounds in the original build to
4.229/4.212 seconds in the final build. Both final compiles reused native gear
features and completed cycle evidence while still checking the actual current
pose. The complete final workspace suite passed 2747 tests with zero failures
and four intentional ignores. The engineering evidence is ready for Review;
human acceptance remains separate. Cold generation still takes about 46 seconds
and remains tracked by CAD-256.

[ADR-0036](../architecture/0036-shared-gear-cycle-evidence.md) owns store lifetime,
keys, staged publication, ownership and timing semantics. The implementation
commits are `2f80eca` (shared detached proofs), `66cbc3b` (worker-local native
caches) and `6867b46` (controller cancellation and diagnostic source hashes).
The earlier comparison below retains its two intermediate runtime identities.
The final corrected package and its distinct native checks are recorded
separately below.

## Acceptance status

| Criterion | Completed evidence | Scope limit |
| --- | --- | --- |
| First/repeat/Apply/drive/Print/export/reopen measurements with separated stages | Actual native action intervals and successful compiler diagnostics retain build, workload and load context. Generation, cycle and current-pose work are separate. | The exact final matching Print control is recorded below; two observations are not a five-run distribution or p95. |
| Safe exact cycle reuse across supported lanes | Focused native sharing and Runtime tests pass; exact geometry/frame/policy/document keys skip only completed samples. Current pose still runs. Actual Print/Common first lanes reuse cycles while rebuilding their own native geometry. | Proof sharing does not transfer native solids or remove current-pose work. |
| No publication from failed/cancelled partial work; bounded concurrency/clear behavior | Store and native tests cover staged outer success, nested work, bounds, epochs, sharing and cancellation. The controller now forwards native cancellation; its regression failed before the fix and passes afterward. | Final native OS cancellation repeat passes with the stop-latency limitation below. |
| Responsive native pending/cancel workflow and required checks | Actual pending work and visible Cancel were exercised; formatting and strict Clippy pass. | The earlier Cancel defect is corrected; native calls remain nonpreemptive and exact stop latency is unknown. |

## Measured builds and workload

The fixture is a continuous herringbone pair: normal module 2 mm, 24/36 teeth,
20-degree pressure, 15-degree helix, width 4 mm, bore diameter 8 mm and separate
0.05 mm design thinning per member. Actual saved sources retain zero and
12-degree drive states. The [checked source audit](../../artifacts/gear-pair-performance-20261106/independent-source-audit.json)
retains both complete recipes, member identities and derived placement.

The [first runtime](../../artifacts/gear-pair-performance-20261106/runtime.json)
is `2f80ecaf896c91b20d910c1e23d30efdc45deb74`, binary SHA-256
`c26f26cc9fc0ab7dd5c08c34d2f7aff71fdb8fcef3b205e208bbf67ab788aafa`.
It shares cycle evidence but does not retain Print/Common native feature caches.
The [owner-cache runtime](../../artifacts/gear-pair-performance-20261106/owner-cache-runtime.json)
is `66cbc3b8e123c5eb56a2f5e24c993da6c84738ae`, binary SHA-256
`95b7e221ab807aeb40bf47e56e0edf8c9bd29f34333f34a1e8a60e7a61efd51b`.
These are release packages. The [load receipt](../../artifacts/gear-pair-performance-20261106/benchmark-load-before.json)
records 18 logical CPUs, load averages 2.679/4.092/3.856 and no competing build
or CAD process before the first run. Load averages do not measure CPU use.

The [first action intervals](../../artifacts/gear-pair-performance-20261106/native-action-intervals.jsonl)
and [compiler records](../../artifacts/gear-pair-performance-20261106/native-compiler-timings.jsonl)
remain separate from the [owner-cache action intervals](../../artifacts/gear-pair-performance-20261106/owner-cache-action-intervals.jsonl)
and [compiler records](../../artifacts/gear-pair-performance-20261106/owner-cache-compiler-timings.jsonl).
Native action intervals include automation and polling overhead; they are
observed bounds, not input-to-present instrumentation or p95 measurements.

| Path | Shared-proof runtime | Owner-cache runtime |
| --- | ---: | ---: |
| Cold successful compiler total | 45.745 s | 44.825 s |
| Unchanged editor Preview, observed | 3.764 s | 5.038 s |
| Apply to current display, observed | 3.926 s | Not repeated in this build |
| Drive-12 Preview, observed | 4.688 s | Not repeated in this build |
| Drive-12 Apply to display, observed | 4.378 s | Not repeated in this build |
| Repeated Print preview, observed | 25.979 s | 4.316 s |
| Print write with 3MF/STL, observed | Not measured | 4.582 s |
| Unchanged Common preview, observed | Not measured | 4.263 s |
| Common STEP write, observed | Not measured | 6.805 s |

The first runtime's first Print compile took 25.215 seconds, including 21.047
seconds constructing gears, despite a cycle hit. Its repeat still reconstructed
them. With owner retention, first Print compilation remained 25.141 seconds;
repeat compilation fell to 3.957 seconds with zero gear-construction time and
2.332 seconds of actual current-pose work. Common Export likewise rebuilt on
its first owner use, then retained native features. This comparison measures
an incremental cache change; it is not an original-versus-final application
baseline. Neither implementation removes expensive first construction.

The same running application also loaded a byte-identical saved copy into a new
session at revision 0 while its prior tab remained open. Compilation took
3.930 seconds with a cycle hit and a real current-pose check. The observed
8.940-second interval includes picker automation and is not an application-open
metric or proof of close-tab behavior. The separate fresh-process cold control
remained 44.825 seconds.

Compiler stage durations are successful wall times. Pair validation contains
sampled-cycle and current-pose time; the other buckets are partial and do not
sum to total. Nested compile timings are not additive CPU measurements. Final
source-hash telemetry binds future successful records to their ModelStamp;
it is absent from these earlier records. Failed/cancelled jobs require external
timing. Serialization, file IO and OS paint are outside compiler timing.

## Original-runtime Print baseline

The [independent baseline receipt](../../artifacts/gear-pair-performance-20261106/baseline-d91-print/independent-baseline-report.json)
binds the original `d91ba92` package to executable SHA-256
`b2844e0194b2e2803520d580e32f771da4ef8cb714e3bac64f9acc5989de5650`.
Two explicit unchanged Print Preview clicks used All (one root, two placed
leaves), per-part STL enabled and chord 0.1 mm. Both returned the same 45050
triangles and displayed native volumes 7409.916/17069.515 mm³. Initial automatic
setup previews and their supersession are excluded from these observations.

| Original-runtime unchanged preview | Last observed pending | First observed ready |
| --- | ---: | ---: |
| First explicit click | 46.442 s | 47.100 s |
| Second explicit click | 46.631 s | 47.286 s |

The final control used the exact same 14-entity drive-12 source and unchanged
settings: All/two placed leaves, per-part STL enabled, chord 0.1 mm and the
15-degree angular default. The [launch binding](../../artifacts/gear-pair-performance-20261106/matched-print-final-01/launch-inputs.json)
retains source/canonical hashes, runtime and load context. The canonical hash
is `5e3cf7eb6c1869601f2502894e241b49771fa5cf22ad8056812bb1eef4c656d4`.
It ran after the
workspace command finished; no build or native benchmark ran concurrently.
The setup's cold canonical load took 45.9814 seconds and first Print-owner
compilation took 25.1077 seconds. Those [setup rows](../../artifacts/gear-pair-performance-20261106/matched-print-final-01/setup.json)
are excluded from the two explicit unchanged-click measurements.

| Final matched unchanged preview | Last observed pending | First observed ready | Native compile |
| --- | ---: | ---: | ---: |
| [First click](../../artifacts/gear-pair-performance-20261106/matched-print-final-01/preview1-receipt.json) | 4.0251 s | 4.2289 s | 3.9289 s |
| [Second click](../../artifacts/gear-pair-performance-20261106/matched-print-final-01/preview2-receipt.json) | 4.0096 s | 4.2116 s | 3.9147 s |

Both final native receipts have zero gear-construction and sampled-cycle time,
a cycle hit and a real current-pose check taking 2.3266/2.3262 seconds. They
retain the same source/geometry identity. These individual polling brackets
support the observed 47-second to 4.2-second change for this matched path;
they establish no universal speedup or p95. The earlier intermediate comparison
retains its separate runtime and option/action scope. The original package has
no stage instrumentation, so its wait cannot be split into native generation
and cycle components.

The [final matching summary](../../artifacts/gear-pair-performance-20261106/matched-print-final-01/summary.json)
confirms unchanged source bytes and preferences, the same 45050 triangles and
two placed leaves, normal exit code 0 and no remaining CAD application. No
save or manufacturing write occurred in this matching-control session.

The baseline source SHA-256
`068bc06a417072c438b04ae9851ed23edb01894a3f2d87f2b0ca6db412b1bb59`
is unchanged. Preference snapshots are byte-identical and the final inventory
contains no running CAD applications. No Common export, save or manufacturing
write was performed during this baseline.

## Cancellation defect and correction

On the owner-cache runtime, changing geometry and cancelling while Preview was
pending returned the visible UI in about 0.950 seconds, but native work later
completed in 46.536 seconds, including a new 21.234-second sampled cycle.
The UI controller cleared its form without calling `backend.cancel()`. The
short visible interval therefore did not establish native cancellation.

The correction forwards Cancel, input invalidation and invalid session/resource
context to the backend. Its Runtime supersedes only that preview lane; the
canonical lane stays independent. Interrupted evaluation cannot publish a new
cycle completion after its final checkpoint. An OCCT call already running must
return to Rust before stopping; a successfully completed proof cancelled later
remains valid evidence.

The [counterfactual regression](../../artifacts/gear-pair-performance-20261106/checks/ui-cancel-before.log)
failed with zero backend cancellations where one was required. The first broad
rerun hung because a mock fixture held its mutex while calling Cancel; that
run was stopped. Cloning the fixture value before calling the controller fixes
the test lock lifetime. The [corrected target](../../artifacts/gear-pair-performance-20261106/checks/ui-cancel-after-fixture-correction.log)
passes 35 tests in 0.20 seconds, including late-reply refusal and unchanged
source/history. The separate final native OS repeat is recorded below.

## Final corrected package and native controls

The [final runtime](../../artifacts/gear-pair-performance-20261106/final-runtime.json)
is source `6867b46597ddd5efe9bd447d3f6af754b93ab1c2`, release binary SHA-256
`c84c1886583bf8d27b52845fcf1471dfceb96fbfa459d98d6090377931890567`.
The [successful compiler log](../../artifacts/gear-pair-performance-20261106/final-compiler-timings.jsonl)
records cold loading at 44.9708 seconds, including 20.2927 seconds of gear
construction. The [CPU observation](../../artifacts/gear-pair-performance-20261106/final-cold-load-cpu.json)
measured 1.080 CPU seconds over 1.0118 wall seconds on an 18-logical-CPU machine,
roughly one logical CPU during that interval. It does not establish a native
stack attribution, sustained utilization or latency percentile.

The [final action interval](../../artifacts/gear-pair-performance-20261106/final-action-intervals.jsonl)
records genuine pending Preview and a visible cancellation observation after
0.980 seconds. The [native-stop probe](../../artifacts/gear-pair-performance-20261106/final-cancel-native-stop.json)
began approximately 53 seconds after that visible observation. Two subsequent
one-second CPU observations measured zero CPU use; there was no successful
abandoned-compile record and no saved-source change. This confirms idle work
when probed, not exact cancel-to-native-stop latency. The source hash remains
`be9b9864a9cd8fd546f9f3b5c9acedc42769a76e8032b422b9232f09d175ebbe`.

The [independent app-MCP summary](../../artifacts/gear-pair-performance-20261106/final-app-mcp-02/independent-summary.json)
checks a warm application proof store with a cold new MCP native owner, followed
by a same-owner repeat. The first evaluation took 24.7815 seconds RPC wall time
and 24.7542 seconds native compile time, including 20.6245 seconds of construction
and 2.3242 seconds checking the current pose. Its sampled cycle was reused.
The repeat took 3.9305 seconds RPC and 3.9041 seconds native, with no gear
construction or sampled-cycle work and 2.3225 seconds checking the current pose.
Both retain the source hash, two physical leaves and native pair evidence;
these are protocol evaluations, not GUI Preview or Print latency. The bridge
exited normally with code zero, without forced shutdown. The earlier sandbox
connection attempt failed before RPC with EPERM and remains separate from
these successful measurements.

The [final closure](../../artifacts/gear-pair-performance-20261106/final-package-source-and-closure.json)
records no running final QA process, unchanged saved hashes, byte-identical
export-config/reopened copies and MCP connections restored off. These controls
close the final native cancellation and app-MCP checks; the complete repository
result is recorded separately below.

## Geometry preservation and focused checks

The [independent manufacturing summary](../../artifacts/gear-pair-performance-20261106/independent-manufacturing-final-summary.json)
is attributed to `66cbc3b`. All 14 original identities and member recipes remain;
only the fifteenth ExportSet entity is added. The reopened copy is byte-identical.
Two independently read STEP files are valid single solids, with volumes
7409.916164956880 and 17069.514940050398 mm³ inside the unchanged declared
budgets. Stored-index 3MF and exact-coordinate STL graphs are closed and
consistently oriented, with cyclic vertex fans, one component and Euler zero
per member. Their 22280 and 22770 triangles total 45050; source-derived poses
and ordered cross-format facets agree within the declared serialization budget.
No native geometry is replaced by a foreign worker solid or viewport mesh.

The [focused summary](../../artifacts/gear-pair-performance-20261106/checks/focused-summary.json)
records four native sharing tests, 49 model unit tests and two owner-lifetime
tests passing, with original failed assertions and their corrections retained.
[Final formatting](../../artifacts/gear-pair-performance-20261106/checks/final-format.log)
and [strict workspace all-target Clippy](../../artifacts/gear-pair-performance-20261106/checks/cancel-fix-clippy.log)
pass. The [pre-fix closure](../../artifacts/gear-pair-performance-20261106/owner-cache-pre-cancel-fix-closed-apps.json)
records no running CAD application after the intermediate measured session.
The final package controls above add their own closure. The [independent final source review](../../artifacts/gear-pair-performance-20261106/final-source-review.json)
found no actionable correctness blocker across the three implementation commits.
It is a read-only peer review, not additional execution.

The [complete locked workspace suite](../../artifacts/gear-pair-performance-20261106/checks/final-workspace/workspace-summary.json)
passed on source `6867b46597ddd5efe9bd447d3f6af754b93ab1c2`: 2747 passed,
zero failed, four intentional ignores across 316 result groups, exit 0. The
[runner](../../artifacts/gear-pair-performance-20261106/checks/final-workspace/workspace-run.json)
records 08:19:57-09:35:30 UTC and the command
`cargo test --workspace --locked --no-fail-fast -- --test-threads=4`, with
optimization 3, debug assertions and overflow checks enabled. The ignored cases
are `interrupted_writer_child` (subprocess helper exercised by its parent),
`blender_imports_the_handoff_with_scale_hierarchy_instances_camera_and_lights`
(optional Blender), and the two physical-GPU tests
`gpu_renders_native_solids_with_depth_projection_highlight_and_cached_meshes`
and `gpu_renders_source_face_materials_with_selection_and_edges`. They are not
reported as executed passes. The [final packaged self-test](../../artifacts/gear-pair-performance-20261106/checks/final-package-self-test.log)
also passed separately from actual native interaction.

First construction and custom-profile current-pose checks remain expensive.
[CAD-256](https://plans.tokenet.de/CAD/256) retains the cold-generation follow-up.
There is no near-instant generation, universal speedup, p95, continuous-contact,
load, physical-fit, geometric self-intersection or print-success certificate.
Windows/Linux runtime and human acceptance remain separate.
