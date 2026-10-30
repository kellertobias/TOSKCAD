# Release budgets, recovery and regression hardening, 2026-10-30

These checks record agent verification for CAD-97, not human acceptance. All
numbers below are **headless** measurements on one development Mac. The native
window was not launched: interactive frame times, selection latency and idle
CPU are listed as pending operator measurements. Windows and Linux results do
not exist yet; the CI workflow added here has only been validated locally.

## Machine, toolchain and source

| Item | Value |
| --- | --- |
| Hardware | Apple M5 Max, `Mac17,6`, 18 cores (6 performance, 12 efficiency), 64 GiB RAM |
| OS | macOS 26.6.2 (Darwin 25.6.0), AC power |
| Toolchain | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, pinned by `rust-toolchain.toml`, committed `Cargo.lock`, `--locked --offline` |
| Profile | `release` (thin LTO, one codegen unit). OCCT 7.9.3 is always built Release and its cache is profile-independent, so the release build reused the existing native cache. |
| Source | commit `1cb3e567dfdf53082aa49c27f456c006ec9b9251`, clean tracked tree |
| Background load | Other agent sessions shared the machine: load average 8–12 during the run. Medians are reported; the spread shows the noise. |

This is a high-end workstation, **not** the low-power target Mac that the
acceptance criterion names. The proposed budgets below assume a target about
three times slower and must be re-measured on that hardware before they are
agreed.

## Reproduce

```sh
node tools/release-budgets.mjs --profile release --repeat 3 --label release-budgets-2026-10-30
```

The developer-only driver builds `crates/multicad-app/examples/release_budgets.rs`,
runs `release_budgets generate DIR` once (the four scenario projects are built
through their typed commands and written as native `.multicad` files), then runs
`release_budgets measure SCENARIO DIR` three times per scenario, each in a fresh
process under `/usr/bin/time -l` for peak resident memory. The JSON report,
the four projects and every generated export are under
`artifacts/benchmarks/release-budgets-2026-10-30/` (git-ignored).

Each measure process:

1. reads and decodes the project file and creates a session (cold),
2. runs the first full compile in the process (cold, includes OCCT start-up),
3. runs three more full compiles (warm),
4. evaluates every drawing page,
5. counts the compiler's cooperative interruption checks and the longest span
   between two checks,
6. edits one parameter (one the scenario's own test applies) and recompiles,
   comparing every evaluated shape before and after,
7. interrupts a compile at 25 %, 50 % and 75 % of the warm compile time and
   measures how long the compiler takes to return,
8. previews and runs every export set into a fresh directory,
9. saves the opened project, reopens and recompiles it, and writes and reads a
   recovery snapshot, comparing documents and resources.

## Model sizes

| Scenario | Entities | Features | Parts | Leaf occurrences | Part faces | Part triangles | Leaf triangles (shared meshes) | Evaluated triangles incl. intermediates | File bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Makerspace | 63 | 7 | 4 | 7 | 65 | 228 | 264 | 300 | 58 478 |
| Cabinetry and table | 136 | 26 | 23 | 26 | 211 | 1 424 | 1 916 | 1 460 | 115 722 |
| Tool case | 104 | 28 | 9 (+4 consumed tools) | 19 | 66 | 1 116 | 4 340 | 8 440 | 72 109 |
| Control desk | 71 | 17 | 7 (+1 consumed tool) | 14 | 142 | 6 488 | 8 176 | 9 920 | 72 026 |

These are small models. Display tessellation uses the default `MeshOptions`;
the triangle counts are display meshes, not manufacturing geometry.

## Measured results (median of 3 processes, range in parentheses, milliseconds)

| Scenario | Cold open: read + decode | Cold first full compile | Warm full compile (9 runs) | Edit commit | Rebuild after one parameter edit | Drawings | Save | Reopen | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Makerspace | 4.7 (4.5–9.6) | 106 (106–203) | 107 (105–199) | 1.8 | 107 (107–199) | 203 | 7.0 | 8.6 | 40 MiB (38–44) |
| Cabinetry and table | 6.4 (6.2–9.1) | 529 (455–665) | 519 (452–655) | 2.6 | 536 (462–643) | 50 | 6.8 | 5.8 | 48 MiB (48–48) |
| Tool case | 4.9 (4.8–7.8) | 660 (660–912) | 663 (657–917) | 2.4 | 665 (665–983) | 2.6 | 5.7 | 4.4 | 65 MiB (62–77) |
| Control desk | 3.8 (3.8–3.9) | 941 (937–942) | 938 (934–969) | 1.5 | 942 (938–944) | 99 | 5.8 | 3.4 | 63 MiB (61–68) |

Generating all four projects through their typed commands (every command is
natively preflighted before it commits) took 1.1 s, 16.4 s, 16.6 s and 17.2 s,
with a 144 MiB peak for that one process.

### Export sets (preview / run, median ms)

Preview and run each perform their own full compile today; that compile
dominates both numbers.

| Scenario | Export set | Preview | Run | Files | Bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| Makerspace | Makerspace delivery (PDF, 2 DXF, 3 STEP) | 1 086 (504–1 097) | 1 127 (543–1 144) | 6 | 129 652 |
| Makerspace | Makerspace scene (GLB) | 219 (110–226) | 223 (122–233) | 1 | 14 064 |
| Cabinetry and table | Panel cutting (DXF, STEP) | 505 | 667 | 18 | 226 187 |
| Cabinetry and table | Table frame and drawings (PDF, DXF, STEP) | 665 | 827 | 18 | 586 115 |
| Tool case | Panel drawing (PDF, DXF) | 669 | 683 | 2 | 38 880 |
| Tool case | Print package (3MF) | 689 | 693 | 2 | 248 342 |
| Control desk | Control desk drawings (PDF, DXF) | 1 143 | 1 177 | 5 | 896 533 |
| Control desk | Fabricated parts STEP | 938 | 1 001 | 3 | 207 543 |
| Control desk | Printed guide 3MF | 943 | 953 | 1 | 80 819 |

Every save/reopen and every recovery write/read returned a document and
resources identical to the opened project (12 of 12 runs).

## Incremental rebuild: finding

The compiler (`Compiler::compile_scoped` in `crates/multicad-model/src/compiler.rs`)
is a pure function of the document. It has no cross-call cache: the only cache,
`Profiles::cache`, lives for one compile. `EvaluationScope::Sources` restricts
an *inspection* compile to selected roots but is never a substitute for a full
Apply or export. The model runtime (`crates/multicad-model/src/runtime.rs`)
supersedes stale requests but always recompiles the whole document.

Measured consequence for one parameter edit:

| Scenario | Parameter | Shapes changed / evaluated | Interruption checks before → after | Rebuild ÷ warm full compile |
| --- | --- | ---: | ---: | ---: |
| Makerspace | `room_width` 9000 → 9600 | 2 / 10 | 243 → 243 | 1.01–1.02 |
| Cabinetry and table | `table_width` 1600 → 1800 | 13 / 26 | 173 → 176 | 1.01–1.13 |
| Tool case | `rail_pitch` 96 → 80 | 2 / 29 | 138 → 138 | 1.00–1.11 |
| Control desk | `desk_width` 240 → 280 | 11 / 18 | 91 → 91 | 1.00–1.01 |

**A parameter edit performs a full rebuild.** The tool-case edit changes 2 of
29 evaluated shapes but costs the same as a full compile. The acceptance
criterion "edits rebuild only affected graph regions" is therefore **not met**.
A correct incremental engine needs per-feature result caching keyed by the
evaluated inputs of each feature (parameters, upstream shape identities,
policy), native solid lifetime on the model worker thread, and invalidation of
derived topology/reference snapshots. That is not a small, safe change, so it
was not attempted here; `incremental_status_parameter_edit_reevaluates_the_whole_graph`
in `crates/multicad-app/tests/release_hardening.rs` pins the current behavior so
an incremental engine must update it together with this report.

**Update (CAD-150, same day):** the model runtime now compiles through a
cross-compile `EvaluationCache`. A cached edit rebuild re-runs only the
affected features plus providers that are never cached. The pin was replaced
by `incremental_status_parameter_edit_reevaluates_only_affected_features`.
Measurements, design and limits are in
[`incremental-rebuild-2026-10-30.md`](incremental-rebuild-2026-10-30.md). The
plain compile path measured above is unchanged.

## Cancellation: evidence and finding

`crates/multicad-app/tests/release_hardening.rs` interrupts the makerspace and
tool-case compiles at 17–18 evaluation boundaries spread from the first check
to the last. Every interrupted compile returns a supersession error, stops at
the boundary it was raised on (at most one further check classifies a nested
evaluator's failure), leaves the document unchanged, and the next compile
reproduces the uninterrupted shapes exactly. The same file checks that the
model runtime publishes only the latest of two queued edits, that `cancel`
prevents a publication and leaves the worker usable, and that `poll` never
blocked for 50 ms or more while a native rebuild ran.

**Fixed in this item:** an interrupt raised while an embedded symbol
definition was being validated surfaced as
`Pinned symbol 'Equipment footprint' has invalid native geometry: Sketch evaluation cancelled`,
blaming the source for an abandoned rebuild. Errors observed while a request is
interrupted are now reported as supersession (`multicad_model::is_superseded`),
and the symbol check passes interruptions through.

Measured cancellation latency from request to compiler return (9 requests per
scenario, all interrupted):

| Scenario | Median | Max | Longest span between two checks in a full compile |
| --- | ---: | ---: | ---: |
| Makerspace | 2.8 ms | 13 ms | 11 ms |
| Cabinetry and table | 14.5 ms | 27 ms | 39 ms |
| Tool case | 18 ms | 228 ms | 209 ms |
| Control desk | 8.7 ms | 245 ms | 483 ms |

**Finding:** individual native operations are not interruptible. The control
desk has a single uninterrupted span of about 480 ms (an OCCT call between two
Rust checkpoints), and the tool case about 210 ms. A superseded rebuild can
keep the model worker busy that long. The UI thread does not wait for it (the
runtime polls a slot), but the next rebuild starts late by up to that span.
Budget proposal below treats this as the cancellation bound.

## Recovery and compatibility regression

`crates/multicad-app/src/files/scenario_tests.rs` (unit tests of the desktop
binary, so they exercise the real `ProjectStore`) builds every scenario and:

- saves through `ProjectStore`, reopens with no diagnostics and an identical
  document, and re-saves the opened project byte-identically;
- writes a manual recovery snapshot, lists it, confirms plain Open refuses it,
  restores it as an unsaved copy with the identical document and resources,
  and confirms the original file is untouched; repeats with an automatic
  session slot;
- opens a format-1 container (source inside the manifest) with a
  `Migrated { from: 1, to: 2 }` diagnostic and the identical document;
- for each capability the project requires, an older reader without that
  capability rejects it with `Project requires unsupported core capabilities:
  <name>`; a project requiring a future capability and a format-3 header are
  rejected with exact messages;
- truncation at every header byte, 64 evenly spaced offsets and around the
  manifest/source boundary fails cleanly; bit flips in the signature, manifest
  and source chunk fail (signature and checksum messages); one appended byte
  reports `Project has unexpected trailing data`.

## CI

`.github/workflows/native-regression.yml` (reusable, read-only
`contents: read`, no secrets) runs on `ubuntu-24.04`, `macos-15` and
`windows-2025`. It builds every workspace target, then runs named steps so a
failure names its area: project-file round trip/recovery/damage, container
codec, cancellation/runtime, model runtime and geometry-worker cancellation,
kernel geometry validity, exporter validation, the four acceptance scenarios,
UI and MCP, application CLI/MCP/shared workflow, then the complete workspace.
A one-run dev-profile budget smoke follows; its report and scenario
deliverables are uploaded for seven days. `release.yml` calls it and the
release job now also waits for it. A release-contract test
(`tools/release-contract.test.mjs`) pins the read-only permissions and the
gate. Publication activation, credentials, tags and tracked versions are
unchanged, as `docs/RELEASING.md` requires.

Validation performed: YAML parsed with `js-yaml` (all three workflows), every
`cargo test` target selector compiled locally with `--no-run`, and `npm test`.
**No GitHub Actions run exists.** The Windows and Linux steps, the CI
smoke timing and the uploaded artifacts are unverified.

## Proposed budgets (to agree on the target low-power Mac)

Measured values are from the M5 Max above. Proposed budgets allow roughly a
3× slower target plus headroom and apply to models of the sizes above.

| Metric | Worst measured scenario | Proposed release budget |
| --- | --- | --- |
| Cold open, read + decode (≤ 120 kB project) | 9.6 ms | ≤ 100 ms |
| Cold open to compiled model | 947 ms (control desk) | ≤ 3 s |
| Warm full compile | 969 ms (control desk) | ≤ 3 s |
| Rebuild after one parameter edit | 983 ms (tool case, full rebuild) | ≤ 3 s now; target ≤ 0.5 s once incremental |
| Cancellation, request to compiler return | 245 ms | ≤ 750 ms; longest uninterrupted native span ≤ 500 ms |
| Export set preview + run | 2.3 s (control desk drawings) | ≤ 6 s per set |
| Save / reopen | 8.8 ms | ≤ 100 ms |
| Peak RSS, open + compile + edit + all exports | 77 MiB | ≤ 300 MiB |
| Orbit / selection frame time | not measured | ≤ 16.7 ms at 60 Hz (pending operator) |
| Idle CPU with a project open | not measured | ≤ 1 % of one core (pending operator) |

## Not measured here: pending operator procedure (macOS, interactive)

The native window must not be launched by an agent session. An operator on the
target low-power Mac should:

1. Build a release app: `npm run release:package -- 0.1.0-rc.1` (or use the
   local release-candidate ZIP described above), unzip it, and note the hardware model,
   macOS version and power source (`sysctl hw.model`, `sw_vers`, `pmset -g batt`).
2. Generate the projects: `node tools/release-budgets.mjs --profile release
   --repeat 3 --label target-mac` (also records the headless numbers on that
   machine).
3. **Cold/warm open in the app:** quit all instances, open
   `artifacts/benchmarks/target-mac/control_desk/control_desk.multicad` with a
   stopwatch from the Open click to a fully shaded model; repeat once warm.
4. **Orbit and selection:** for each project, orbit continuously for 10 s and
   click-select 20 faces while recording with Instruments (Time Profiler plus
   Core Animation FPS / Metal System Trace). Report median and 99th-percentile
   frame time and selection highlight latency.
5. **Rebuild responsiveness:** edit `desk_width` in Parameters and Apply;
   orbit during the rebuild and confirm the view keeps responding; report the
   time to the rebuilt model.
6. **Idle CPU:** leave the control-desk project open and untouched for 60 s;
   record `top -l 13 -s 5 -pid <pid> -stats cpu` and report the mean of the
   last 12 samples.
7. **Memory:** Activity Monitor real memory after opening all four projects in
   turn and running one export from each.

## Local macOS release candidate

Built with the existing packaging path, offline, from clean committed source:

```sh
CARGO_NET_OFFLINE=true npm run release:package -- 0.1.0-rc.1
```

The packager ran the complete workspace tests (1 498 passed, 0 failed,
3 ignored across 190 test binaries), cleaned and rebuilt the release products,
checked `--version`/`--diagnostics` against the version, source commit, target
and OCCT source hash, audited that the executable links only system libraries,
ran the packaged `--self-test` (a real constrained-profile solid, cut and STEP
round trip) and archived the result under `artifacts/release/` (git-ignored):

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `multicad-0.1.0-rc.1-macos-arm64.zip` | 26 535 577 | `b7db0797566a70f3f1edfb298e5c81e3617544e14364ea0e288f8e9a5a8f55ed` |
| `multicad-0.1.0-rc.1-macos-arm64-sources.zip` | 260 900 499 | `92241bb6e3a23a7bef2106ea783c50cd8a883dee485613cd3b915435ad904f42` |

Identity record (`.zip.json`): source commit
`d650c474dbbc8f2c3b545a05df13b99e9161041c`, target `macos-arm64`, OCCT source
hash `febdb33f3c2570f3ab04a19062594133f1f9c2fde624348aad8595fb85c95cd5`,
signing `ad-hoc, not notarized`. Both `.sha256` files verify with
`shasum -a 256 -c`.

Notices inside `MultiCAD.app/Contents/Resources/`: the product `LICENSE`,
`native-dependencies.json` (OCCT pin), the OCCT LGPL-2.1 license and exception,
`third-party.json` (471 Cargo packages with license expressions and sources)
and `third-party/` with each package's license/notice files. The
corresponding-source ZIP holds the exact commit, `Cargo.lock`, vendored Cargo
sources, the pinned OCCT archive and extracted source, and
`SOURCE-REPRODUCTION.md` with offline rebuild/relink instructions.

Limits: this is a single-platform local artifact, not a release. It is
ad-hoc signed only; nothing was publisher-signed, notarized, published,
uploaded or installed. Bit-for-bit reproducibility of a second build was not
checked. `ditto --sequesterRsrc` adds `__MACOSX/` AppleDouble entries to the
runtime ZIP (existing packager behavior). `package-release.mjs verify` needs
all three platform archives, so the aggregate set cannot be verified until the
Linux and Windows runners produce theirs. The `0.1.0-rc.1` label is local
only; no tag exists.

## Test fix found while packaging

Two unitless-DXF tests (`crates/multicad-core/tests/sketch_import.rs` and
`crates/multicad-ui/src/artwork_import.rs`) replaced the CRLF form of
`$INSUNITS`, but the fixture is committed with LF (`* text=auto eol=lf`). They
passed only in a working copy holding CRLF bytes and failed in fresh checkouts,
worktrees, CI runners and the corresponding-source archive, which would also
have stopped every CI packaging job. Both now normalize line endings first.

## Not verified

- Interactive macOS review: orbit/selection frame times, rebuild
  responsiveness in the window, idle CPU and in-app open times.
- Measurements on the low-power target Mac; the numbers above are from an
  M5 Max with background load.
- Any GitHub Actions run; Windows and Linux build, test and package results.
- Incremental rebuild (not implemented; full rebuild measured).
- Interruption inside long native OCCT operations (not supported).
- Publisher signing, notarization, publication; manufacturing correctness of
  the exported files beyond what the scenario tests read back.
- Human acceptance.
