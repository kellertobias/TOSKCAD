# Native UI input latency investigation

Status: in progress for CAD-243. Fresh macOS idle and busy interaction traces remain required. The original reported running build is unknown. No claim that the proposed input-feedback budget is met.

The saved equivalent is `artifacts/equipment-review-20261101/readable/apartment-equipment.tcd`. The tested machine is a Mac17,6 MacBook Pro, Apple M5 Max, 64 GB, macOS 26.6.2 (25G83), with a 1360 × 860 point QA window. Displays were 3456 × 2234 Retina, 1920 × 1200 at 60 Hz, and 5120 × 1440 at 60 Hz. Native QA copies had separate bundle identities; no installed application replacement was performed.

## Findings and changes

Navigation reloaded a clean parameter editor before every command, including a full empty-transaction preparation. Read-only model controls and advanced sketch context also prepared empty transactions. These now use session identity/revision stamps; stamps cannot authorize commits. Actual candidates still use the typed preparation and native validation gates.

Model tree ownership and overlay visibility repeatedly deserialized and validated embedded symbol definitions. Session-only read projections now reuse already validated ownership and typed private overlay metadata. General document validation remains unchanged. Library previews now retain layout while skipping content hashing, decoding and GPU texture creation outside the visible clip rectangle.

Exact canonical hashes are cached per immutable session revision, shared with preparation snapshots and invalidated on source replacement, undo, redo and file-preview capture. First-use sketch fingerprinting uses one bounded worker per sketch feed. The UI shows “Preparing sketches…” and withholds current picking until the exact source fingerprint is ready. Source changes revoke current results immediately. A late worker can populate only its original cache epoch. Retries retain a busy worker instead of creating additional workers. Preparation snapshots share immutable evaluated state to avoid copying the full document on the UI thread.

## Measurement boundaries

Native 15-second CPU samples during matching Weld/Parameters/Model navigation fell from 121 to 24 samples in application UI work, from 60 to zero in clean-editor reload, and from 60 to zero in empty session-guard preparation after the first two fixes. Initial workbenches differed and are recorded in `cpu-before-after.json`. CPU samples are attribution, not elapsed input latency.

The optional native probe's v3 known-action sequence 1–17 contains five pointer interactions and Escape on source `5f31fed` plus `3c00754`, before the later session/hash changes. All 17 inputs were associated with actual Metal presentation times, including recovery through subsequent frames when the first candidate drawable was dropped. Observed p50 was about 74.24 ms and p95/max was 2056.21 ms. The first return to Model was the outlier; it is retained. This sample **fails** the proposed p95 ≤ 100 ms and max ≤ 250 ms budget. Later events in the trace are not attributed to known QA actions. v1 probe output is invalid and excluded; v2 cannot recover dropped candidate presentations.

An Instruments capture contains 666 target-surface frame lifetimes, p95 about 39.36 ms and max about 55.97 ms. Those are not input-to-feedback timings. The capture's raw table of contents includes private device/environment metadata and must not be published wholesale.

The offline `ui_frame_profile` example exercises actual application layout on the same saved source, without a window, GPU presentation or native modeling backend. Application/session construction is excluded. Before these changes, first Model layout took about 1001 ms, the next frame about 613 ms, and a later return about 7 ms. The intermediate exact-hash cache reduced first Model layout to about 178–180 ms and warm frames to 2–3 ms. On `eda6605`, three cold repetitions measured first Model CPU at 99.23–103.22 ms, source fingerprint readiness about 50.10–52.69 ms later, continuation frames at most 5.73–6.86 ms, warm Model at 1.79–1.96 ms, and later Model return at 8.05–8.20 ms. These observations do not establish perceived responsiveness or busy-job acceptance.

## Developer tools

`tools/measure-native-input.mm` is an optional single-window macOS QA probe. Normal application builds do not link or load it. It observes local events without recording key contents or modifying delivery, and joins them to subsequent Metal drawable presentation timestamps. It records clock agreement, dropped/unpresented inputs and bounded-input overflow. A subsequent displayed drawable is a timing proxy; the trace does not prove that a particular pixel reflects the intended feedback. Pair it with actual interaction and visual inspection.

Build only for an isolated, ad-hoc signed QA copy:

```sh
clang++ -std=c++17 -dynamiclib -fobjc-arc -Wall -Wextra -Werror \
  tools/measure-native-input.mm -framework AppKit -framework QuartzCore \
  -framework Metal -o /private/tmp/multicad-input-trace.dylib
codesign --force --sign - /private/tmp/multicad-input-trace.dylib
```

Launch that QA executable directly with child-only `DYLD_INSERT_LIBRARIES` pointing to the dylib and `MULTICAD_INPUT_TRACE` pointing to a new absolute JSONL file. The probe refuses to overwrite a file. Do not attach it to the user's active application or change signing/security settings on an installed application. Quit the owned QA window normally after the measurement.

Analyze a documented measurement interval with:

```sh
node tools/measure-native-input.mjs /absolute/path/trace.jsonl LAST_MEASURED_SEQUENCE
node --test tools/measure-native-input.test.mjs
cargo run -p multicad-ui --example ui_frame_profile --locked -- /absolute/path/project.tcd 3
```

The cutoff excludes only later terminal/unknown actions, never slow samples inside the chosen interval. The analyzer rejects corrupt clocks, mismatched input/presentation records and duplicate records, counts missing/dropped/overflowed inputs explicitly, and cannot pass the budget with incomplete coverage. The offline example reports source-hash preparation wall time and continuation-frame CPU cost separately from the first frame.

## Remaining verification

The host removed native UI-control tooling during this investigation. Previously owned QA processes were absent on the subsequent inventory; no user window was relaunched or replaced. Fresh actual native hover, menus, selection and mode/workbench traces, both idle and under representative native jobs, remain unverified. The earlier interrupted run remains excluded. Frozen-source verification on `7ff339e` completed successfully: formatting; strict all-target workspace Clippy; full workspace tests (2,093 passed, zero failed, three ignored, 243 groups); 32 release-contract tests; five timing-analyzer tests; and strict Objective-C++ probe compilation. Logs and `checks-summary.json` are retained in `artifacts/ui-latency-review-20261101/`. A final process inventory found no old task-owned Latency/Equipment QA or Review process. Agent completion and human acceptance remain distinct.
