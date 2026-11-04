# Native UI feedback verification, 2026-11-04

New thread and gear candidate preparation runs in a bounded worker. Part selection
uses a session identity/revision guard instead of preparing an empty transaction.
Actual macOS interaction verifies successful previews, responsive cancellation and
unchanged source after cancellation. The development build still misses the
complete feedback budget. An optimized build passes the measured navigation workload; optimized busy-work presentation
coverage remains incomplete, so the complete target is unverified.

Tracking: [CAD-243](https://plans.tokenet.de/CAD/243). Human acceptance is pending.
The [earlier investigation](ui-input-latency-2026-11-01.md) retains the previous
CPU profiles, interrupted runs and original native failure.

## Source and execution conditions

Implementation evidence is committed on main:

- `3efccaf`: strict workload intervals in the optional native timing analyzer.
- `5908893`: bounded preparation for new threads and gears, including cancellation
  and session-ownership regressions.
- `c5bc1fa`: read-only Part inspector guard without empty transaction preparation.
- `d271cac`: unique output-folder probe identities after a reproduced parallel
  check collision blocked the workspace suite.

The real project is a copy of
`artifacts/equipment-review-20261101/readable/apartment-equipment.tcd`, SHA-256
`ea785fbde6acb488e60eb6e1089cdf1da9ec8a09eb0c5e9e87a366796f7c9aa2`.
The final copied file retained that hash after graphical checks.

The measured development package is `artifacts/development/build-RJXX0l/MultiCAD.app`.
Its original executable SHA-256 is
`b04599fe29e87298eafb03446a5c70fe9d16b7a8abfe30d061addbfdcf03bd87`.
The separate temporary QA copy, including ad-hoc signing and an optional probe
launcher, has executable SHA-256
`f77ee313ff71424d92143e10dcdb9099e55a5c053f4b4ecfe3203deac16dbce2`.
No installed application was replaced. The originally reported executable was
unknown, so these results reproduce the affected workflow on identified builds
and do not identify or retest that original executable.

A separate optimized build uses source `d271cac`, built with
`cargo build --workspace --release --locked` and the existing native packaging
helper. Its original executable SHA-256 is
`bb2cda53b64c2b03b549cd71709d63943a107317f8c466d70aadff051757bd37`.
`optimized-visible-metadata.json` records its temporary signed QA copy and
probe launcher. Release diagnostics and the correctly invoked self-test pass.
An ordinary, uninstrumented review bundle is retained at
`artifacts/ui-latency-review-20261104/optimized-review/MultiCAD.app`;
its identity is recorded in `optimized-review/review-source.json`.
Strict bundle signature verification passes.
The initial self-test invocation omitted its required output directory and
exited with usage status 2; that invocation is retained separately.

Live machine inspection records Mac17,6, Apple M5 Max, 64 GB, macOS 26.6.2
(25G83), and the built-in 3456 x 2234 Retina display at 1728 x 1117 points,
120 Hz. Native screenshots show a 2720 x 1720 pixel QA window. The earlier
multi-display conditions are historical and are not assumed identical.

## Actual input measurements

Artifacts are retained locally under `artifacts/ui-latency-review-20261104/`.
The v3 optional probe observes native events without changing their delivery.
It correlates occurrence times with subsequent actual Metal presentations.
This is a feedback timing proxy, not proof of a particular changed pixel.
Coalesced events share frames; event counts are not independent action counts.

| Workload and source | Native inputs | p50 ms | p95 ms | Maximum ms | Budget |
| --- | ---: | ---: | ---: | ---: | --- |
| Initial aggregate, `9930e82` | 218 | 43.39 | 115.77 | 2191.74 | Fail |
| Intermediate aggregate, `5908893`, tests running | 102 | 51.70 | 110.19 | 841.17 | Fail |
| Final idle, events 1-26, `c5bc1fa` | 26 | 57.03 | 120.75 | 318.44 | Fail |
| Final body selections, events 4-9 | 6 | 45.53 | 70.54 | 70.54 | Pass |
| Final busy, events 35-45, tests running | 11 | 51.57 | 83.13 | 83.13 | Pass |
| Final combined, events 1-45 | 45 | 57.03 | 106.02 | 318.44 | Fail |
| Optimized navigation, events 1-26, `d271cac`, tests running | 26 | 27.59 | 44.29 | 57.27 | Pass |

The proposed budget is p95 at most 100 ms and maximum at most 250 ms.
Every interval in the table has complete presentation coverage and zero overflow.
The initial aggregate includes recovery through a later displayed frame when
the first drawable candidate was dropped. Its slow samples remain included.
Workloads and display conditions differ, so aggregate rows do not establish a
controlled speedup ratio.

The final idle sequence covers first Model opening, two body selections,
Weld/Common, File/Escape and Project/Model. No owned Cargo or test workload ran
during that sequence. Its largest sample is first Model opening; the next
slowest is the Project transition. Both remain in the aggregate.
The subsequent first Mechanical activation during setup accounts for the
combined sample's p95.

The busy sequence starts an uncached native spur preview, opens File immediately,
dismisses the menu and cancels. The accessibility observation contains both
“Building native preview…” and the open File menu. A subsequent observation
reports “Modeling cancelled · document unchanged · r0”. A later full observation
contains no stale preview. Full workspace tests ran concurrently during this
busy sequence, so it includes external CPU contention.

`final-idle.json`, `final-selections.json`, `final-busy.json` and
`final-aggregate.json` contain analyzer output. `before-native-input.jsonl`,
`intermediate-native-input.jsonl` and `final-native-input.jsonl` retain raw
events. The final aggregate includes every recorded input, and the trace retains
the normal-quit exit record. Bounds separate documented workloads and never
remove an interior outlier.

The optimized navigation sequence repeats the nine documented navigation and
selection actions while workspace tests run. An early visibility checkpoint
before the first input confirmed at least
1,282 nonzero Metal presentation records; the retained trace ultimately contains
2,519 before its first input. This sample passes, but profile and load conditions differ from the development build; it does not
establish a controlled speedup ratio.

Later optimized setup and busy-work inputs do not have complete presentation
coverage: events 51-98 have zero-time drawable candidates with no subsequent
displayed frame before exit. The complete 98-input aggregate therefore fails
the analyzer even though its 50 measured inputs are fast. The explicit busy
interval, events 88-98, has 11 dropped samples and no valid timing distribution.
It is not an optimized busy-work pass. Accessibility observations nevertheless
showed an open File menu alongside “Building native preview…” for an uncached
herringbone gear with a changed bore, followed by cancellation at unchanged r0
and no stale result in a later observation. An earlier optimized attempt also
had interrupted visibility/presentation conditions. Both complete traces and
analyzer failures are retained; the cause of the presentation gaps is unproven.
`optimized-visible-{navigation,busy,aggregate}.json` and
`optimized-visible-native-input.jsonl` retain the fresh attempt.
`optimized-interrupted-*` retain the earlier attempt.

## Profile attribution and preserved guards

The native preview profile records main-thread `ModelRequest::prepare`,
`prepare_thread`, `prepare_bodies` and `Session::prepare_scoped`, including full
document validation. `before-preview-sample.txt` retains the sample. The
selection profile records `PartEditor::load`, its empty-transaction guard and
`Session::prepare`; see `intermediate-part-selection-sample.txt`.
CPU samples identify executed code, not geometry wall time.

[Candidate preparation](../../crates/multicad-ui/src/modeling/controller/preparation.rs)
shares an immutable snapshot with the originating session identity and revision.
One running request and one replaceable queued intent bound resource use.
Cancel revokes publication without joining validation. The controller rechecks
the live session, source, resources and form before submitting native work.
Existing native scene proofs, prepared-transaction validation and Apply checks
remain active. Editing existing features and other modeling tools retain their
existing preparation paths.

The [Part inspector](../../crates/multicad-ui/src/materials/part.rs) validates its
read-only guard through `SessionStamp`. Actual property and tag changes retain
fresh transaction preparation and commit validation. Pending tag edits still
reject changed selections, revisions and foreign sessions.

Actual intermediate macOS checks reached “Native preview ready” with Apply
enabled for the default modeled ISO metric M8 x 1.25 thread and a native spur
gear. An M8 thread with length 50 mm returned a geometry error and kept Apply
disabled. The inspector clipped that error, so its full text was not captured.
No geometry correctness or total generation-speed claim follows from that case.

## Verification and remaining review

Formatting, strict locked workspace Clippy and the repository build wrapper pass
on the current source. Five preparation regressions, thirteen native thread
workflow regressions and eight Part inspector regressions pass. The timing
analyzer passes eight tests. The complete UI group passes 940 tests with zero
failures and two ignored.

The full suite on the measured source found a writable-folder probe collision:
parallel checks used the same process ID and repeated clock timestamp. An
independent exchange-test repetition reproduced `File exists (os error 17)`.
`probe-collision-reproduction.log` retains that failure. Probe names now use an
existing UUID type while preserving exclusive creation and cleanup. The new
regression performs 2,048 concurrent checks with no false error or leftover file.
PDF lifecycle assertions remain active and include failure diagnostics.
Thirty independent exchange-suite repetitions pass 450 tests, including
61,440 concurrent folder checks.

The complete `cargo test --workspace --locked` run on `d271cac` exits 0:
2,570 passed, zero failed, four ignored across 295 reported groups.
`d271cac-workspace.log` and `checks-summary.json` retain the result.
The earlier interrupted and failed runs remain retained.
The recorded native performance source remains pinned above; the probe change
affects output-folder checks, outside the measured modeling interactions.

All completed QA windows were closed through native controls. The final application
inventory shows the known ToskCAD variants stopped. Source hashes confirm the
copied project is unchanged. No push, installation or publication occurred.

Development first Model opening and navigation transitions miss the feedback
target. Optimized navigation passes its documented sample, while optimized
busy-work timing remains unverified because presentation coverage is incomplete. Total native geometry generation time is not measured
by these input traces. These limitations remain for product-owner review; agent evidence
does not establish human acceptance.
