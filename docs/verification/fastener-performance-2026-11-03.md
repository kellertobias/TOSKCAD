# Fastener performance verification, 2026-11-03

At frozen commit `491b359`, the three measured Library fixtures meet the
250 ms repeated-placement engineering target: 117 to 171 ms after import.
A second direct default screw takes 9.092 ms. Correctness, save/STEP export
and independent-session reopen checks passed in all three fixtures, with both
processes exiting zero. The required painted-controller, formatting, strict
Clippy and full workspace checks all passed; the suite contains 2,466 passing
tests. OS-window behavior and human acceptance remain unverified.

## Measured executable and default screw

The measured package is
[build-J31Tfj](/Users/keller/repos/_apps/multicad/artifacts/development/build-J31Tfj),
assembled in 19.96 s according to the
[wrapper log](/private/tmp/cad252-mesh-final-app-build.log). Its executable
SHA-256 is
`c115862729a5535864e9a92bd13cbc2519accab6011d7ea81738d46ede89eb87`.
All current raw reports below identify this executable and successful process
shutdown. These are headless stdio MCP native-gate timings, not OS click-to-ready
measurements.

The direct recipe is the full modeled default M3 socket-cap screw: L10/B10,
pitch 0.5 mm, head diameter 5.68 mm, head height 3 mm and hex socket across
flats 2.55 mm with depth 1.3 mm. The last operation sets L11 while retaining
B10. The second screw has fresh feature/body identities.

| Headless MCP operation | Baseline, ms | `491b359`, ms |
| --- | ---: | ---: |
| First default M3 L10/B10 | 6101.057 | 498.552 |
| Unchanged edit | 6128.318 | 2.408 |
| Second identical screw, fresh source | 12440.054 | 9.092 |
| Edit first to L11/B10 with two screws present | 12455.818 | 361.233 |

Sources: [baseline](/tmp/cad252-metric-perf-5i4rYK/report.json) and
[current default](/private/tmp/cad252-metric-perf-vL7PEZ/report.json). The
baseline executable was unstamped, with SHA-256
`8c39a532dc95b28e3dee53c02a238ed992ec2c364056ccb56be5e3c588196029`.
It ran alongside the CAD-213 workspace suite on an 18-logical-CPU host;
CPU busy fractions during these calls were approximately 19% to 28%.
These single observations do not establish a controlled whole-application
performance comparison.

## Actual Library placement and remaining limits

Each fixture imports an actual shipped pin, places independent instances,
configures parameters, saves, removes the copied library source, reopens the
consumer in another process and exports native STEP. All three workflows
passed; these timings measure placement calls after import.

| Shipped pin | First placement, ms | Second placement, ms |
| --- | ---: | ---: |
| [M3 socket cap L30/B18](/private/tmp/cad252-catalog-perf-Hq5JFv/smoke-report.json) | 148.312 | 171.080 |
| [M2.5 socket cap L12](/private/tmp/cad252-catalog-perf-0FTiZh/smoke-report.json) | 119.272 | 140.769 |
| [M10 external hex L30](/private/tmp/cad252-catalog-perf-sdf2Hk/smoke-report.json) | 99.339 | 117.108 |

Cold work remains visible. Imports took approximately 643 to 951 ms in these
runs. Editing a customized instance after independent reopen took 1107 to
1603 ms, including reconstruction in the fresh process. The first direct
default still took 499 ms. These limits remain explicit even though the
measured repeat-placement target passes. Unsupported asset recipes and any
input/policy/source mismatch use ordinary native construction; no universal
instant-first-generation result is claimed.

## Reuse safety and focused evidence

The implementation combines separate preview/canonical request lanes, bounded
owner-thread feature and template reuse, completed-screw owner rebinding,
literal symbol-node restoration and accepted native measurement receipts.
The symbol path remains restricted to validated one-screw private graphs with
literal placements. Keys retain ownership/pin/private source, current feature
dependencies and evaluated parameters, materials, pose and compiler settings.
Restored topology and leaf descriptors receive the current revision; global
reference, material, budget, cancellation and publication checks remain active.

The final display cache retains numeric arrays only, after all existing Rust
mesh checks. Exact linear/angular bits and triangle limits key reuse. Its
owner-thread LRU holds at most 16 entries, 32 MiB of numeric buffers in total
including a pending candidate, and 4 MiB per receipt. Exact-sized arrays are
allocated only after freeing the required budget. Shapes retain weak numeric
receipts; the LRU retains no native geometry handles. Changed geometry,
nonidentity poses, imports, changed settings and unsupported copies remain
cold. Single-member compound reuse additionally proves exact physical-set,
world-location, face-ordinal and orientation identity. Independent source
reviews found no remaining actionable issue in these cache boundaries.

The [final independent asset audit](/private/tmp/cad252-mesh-final-asset-audit.json)
passed raw inventory, digest and source-fingerprint checks for 19 catalog rows,
15 assets and 11,294,004 BRep bytes. Source fingerprint:
`c4e7bfbc40b67cb4176e6974aaf2e00288b9febbbb2824f6d965269104f015ea`.
The audit did not recompile or rerun native geometry. Compiled assets still
undergo native validity, bounds, guarded measurement and neutral-role adoption;
a mismatch reconstructs normally.

| Focused evidence | Recorded result |
| --- | --- |
| [Final mesh-cache tests](/private/tmp/cad252-mesh-cache-tests.log) | Five passed; 0.24 s |
| [Kernel/model all-target Clippy](/private/tmp/cad252-mesh-cache-clippy.log) | Passed; 16.40 s |
| [Prior real-library symbol-cache tests](/private/tmp/cad252-symbol-node-tests.log) | Four passed; 8.52 s |
| [Prior completed-screw model tests](/private/tmp/cad252-whole-screw-tests.log) | Four passed; 26.89 s |
| [Prior kernel library target](/private/tmp/cad252-whole-screw-kernel-lib.log) | 21 passed; 58.82 s |
| [Prior strict legacy mechanical target](/private/tmp/cad252-whole-screw-legacy-mechanical.log) | Four passed; 5.60 s |
| [Final painted production-controller test](/private/tmp/cad252-required-painted.log) | One passed; 1.12 s execution after 7.74 s compilation |
| [Required formatting](/private/tmp/cad252-required-fmt.log) | Passed |
| [Required strict workspace all-target Clippy](/private/tmp/cad252-required-clippy.log) | Passed; 25.28 s including orchestration |
| [Final packaged native smoke test](/private/tmp/cad252-final-runtime-smoke.log) | Passed; actual STEP round-trip and four bundled examples, exit zero |
| [Required full workspace suite](/private/tmp/cad252-required-workspace.log) | Passed: 2,466 tests, zero failures, four ignored, 275 result groups |

## Final verification and limitations

The painted default-controller regression, formatting and strict workspace
all-target Clippy passed on frozen `491b359`. The
[full workspace suite](/private/tmp/cad252-required-workspace.log) exited zero:
2,466 passed, zero failed and four ignored across 275 result groups. That phase
took 1,616.37 s including compilation and execution. The coordinated
[result](/private/tmp/cad252-required-checks-result.json) records every command,
exit status and tested source commit.

The final painted test operates the production hardware controls and native
backend, while requiring the independent canonical SceneFeed to publish the
committed revision after Apply. It verifies two distinct default M3 features.
First preview was ready in 574.690 ms; the second in 123.168 ms. Unchanged
Preview took 60.841/118.837 ms, and Apply to canonical readiness took
58.955/115.603 ms. These include CPU egui painting and polling; they are
automated controller measurements, not actual macOS-window timings.

The earlier `1727eb7`
[workspace run](/private/tmp/cad252-workspace-final.log) was interrupted when
new source superseded it. The `09ef2e3`
[baseline suite](/private/tmp/multicad-cad213-workspace-final.log) passed
2415 tests with zero failures and four ignored; that result predates this work.
Neither substitutes for the current required suite.

CAD-253 is separately in Review, documented in
[the build-cache report](/Users/keller/repos/_apps/multicad/docs/verification/build-cache-2026-11-03.md)
and report commit `c330b85`. Its build-directory and packaging evidence does
not establish fastener latency or human acceptance. Actual macOS window
interaction and Windows/Linux runtime remain unverified. No installation,
push or publication is recorded here.
