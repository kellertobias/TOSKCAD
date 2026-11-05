# Long metric threads and parallel physical validation

CAD-255 fixes a valid M8 × 1.25 thread at 50 mm length that previously failed before native Preview could become ready. Commit `dd50334` adds a narrowly typed bounded reconstruction and parallel physical-profile validation. Commit `06f831f` refreshes the trusted metric cache's source receipts. All work stays on `main`; no installation or publication is part of this verification.

## Diagnosis and preserved checks

The earlier UI review recorded an uncaptured, clipped error for this length. Fresh canonical and cache-bypassing kernel diagnostics reproduced `thread: Native helical ridge construction failed`. A faithful standalone OCCT reproduction localized it to continuous `PipeShell::Build`, reporting `PipeNotDone` before MakeSolid, fusion, trimming, physical probes, or volume integration. Forty authored turns plus the extensions are within the existing budget.

Increasing the sweep's segment allowance alone made a ridge build but reported 0.104666 mm surface error. That experiment was rejected. The implemented retry instead reconstructs the same authored helix with the existing bounded spans. Only continuous `PipeNotDone` enters this new retry; MakeSolid, B-rep validity, ancestry and unrelated errors retain their failure behavior.

The bounded path already passed the native guards but spent most of its time classifying physical crest/gap probes. Every quarter-pitch band and the extra original regression sample remain checked, with the same coordinates and tolerances. Rounded and asymmetric-root probes also retain their original formulas. A bounded OCCT pool supplies up to 16 participants; copies and classifier state are private to each participant. Geometry is copied sequentially before workers start. The implementation caps copied faces, leaves nominal CPU headroom, falls back to the original complete serial guard if setup cannot complete, and replays failures in their original band/start/probe order after joining. It changes no global pool configuration.

## Controlled native comparison

The machine reports macOS/arm64 and 18 available logical CPUs. The comparison runs the same frozen C++ implementation and M8 × 1.25 × 50 mm recipe in separate fresh standalone processes. Serial mode limits only that standalone process's OCCT pool to one participant; parallel mode uses its default pool. Both include the unchanged volume guard.

| Measurement | Serial | Parallel |
| --- | ---: | ---: |
| Participants | 1 | 16 |
| Total wall time | 31.203 s | 4.608 s |
| Physical checks | 29.219 s | 2.334 s |
| Parallel classifier setup | — | 0.304 s |
| Checked bands | 161 | 161 |
| Faces / ancestry seeds | 167 / 167 | 167 / 167 |

Both produce one valid solid, exactly the same measured volume, and byte-identical final B-reps and ancestry receipts. This is about 6.8× faster overall and 12.5× faster in physical checks for this controlled pair. There is one observation per mode, not a latency distribution. The separate new native regression reports construction stages around 0.716 s for the 12.5 mm control, 4.595 s for 50 mm and 4.329 s for 49.8 mm. These measurements do not establish OS-window input-to-paint latency.

Reproduction sources, raw profiles and receipts are under `artifacts/thread-performance-followup-20261105/`, particularly `independent-m8-length-diagnostic.json`, `independent-pipe-capacity-diagnostic.json`, and `independent-controlled-bounded-m8.json`.

## Trusted metric cache and real command timings

After freezing the fingerprinted kernel source, the native generator rebuilt the 19 shipped authored/pinned recipes as 15 unique prototypes in a fresh temporary directory. Its independent audit recomputes source identity `afa933f37fa62f5b1892755cced1700d1c84dfc26b56fcd346e64e6e243bb047`, checks digests, exact recipe/policy keys, native face/solid records and complete neutral-role ancestry, and checks each authored file and pinned template. All 15 B-reps, totaling 11,294,004 bytes, are byte-identical to the retained assets; only the registry's source fingerprint changes. The final executable is rebuilt after installing the audited registry.

Cold restoration tests cover every shipped recipe with zero fresh constructions and zero warm-cache hits. Factory retention/rebinding tests also pass. Generator throughput and restoration timings measured alongside builds are not used as latency acceptance evidence.

The final local bundle is `artifacts/thread-performance-followup-20261105/optimized/MultiCAD.app`, executable SHA-256 `46948738ff624ca49bc1f7842eeb9cc6929ea82ef2fa800121f999160184fc7d`. Its real public headless MCP command measurements, without competing native builds/tests, are:

| Operation | Measured time |
| --- | ---: |
| Default M3 modeled screw creation | 431.871 ms |
| Unchanged edit of that feature | 0.637 ms |
| Second identical screw, new identity | 1.982 ms |
| Change underhead length to 11 mm, retain 10 mm thread | 321.655 ms |

Real shipped Library pins were separately imported, placed twice, exported as native STEP, saved and independently reopened. The scopes remain separate:

| Pin | First import | Two placements after import | Customized edit after independent reopen |
| --- | ---: | ---: | ---: |
| M3 socket, L30 | 802.875 ms | 40.537 / 46.283 ms | 1342.201 ms |
| M2.5 socket, L12 | 516.375 ms | 30.721 / 35.624 ms | 906.967 ms |
| M10 external hex, L30 | 542.879 ms | 27.427 / 32.263 ms | 932.011 ms |

These are recorded command observations, not OS-window paint measurements or universal guarantees. All native benchmark processes exit normally. The first catalog-driver attempt used an incorrect M2.5 filename and stopped before that case; the corrected verified filename was then run successfully. Its diagnostic remains alongside the successful reports. Driver transcripts, exports and saved files are retained under `default-benchmark/` and `catalog-benchmark-1/` through `catalog-benchmark-3/`.

After the full workspace tests finished, a fresh final-executable MCP process measured the exact custom M8 × 1.25 × 50 mm case: **5014.657 ms cold**, **5.368 ms for a second identical thread with new canonical identities**, and **1.348 ms for an unchanged edit**. The source parameters remain unchanged, both Parts evaluate successfully, and the process exits 0. `long-thread-benchmark/` retains the complete transcript and report; `benchmark-long-thread.mjs` reproduces it. The full-command cold time includes work beyond the isolated C++ construction timing above. This supports fast reuse, while retaining the explicit several-second cold limitation.

## Actual macOS workflow

The rebuilt bundle's actual UI created the M8 × 1.25, 50 mm modeled thread, reached Native preview ready, fitted the complete preview, and cancelled at unchanged revision 0. Repeating the same preview reached Ready on the first captured state. Apply, toolbar Undo/Redo and Save As succeeded. After quitting and cold-launching, the saved file reopened; Edit feature retained diameter 8 mm, pitch 1.25 mm and length 50 mm. Cancelling that recalled edit preserved the source.

A separate new project selected Machine screw in Hardware, previewed the default modeled M3 screw, applied it, and saved the native result. This form identifies the default as custom geometry; sourced catalog designation and certified fit are separate matters. Both saved files and the 20 screenshot/accessibility pairs are retained. Captures 02 and 03 show pending work; 04/05 show the settled long-thread Ready state. Capture 17 is a transition snapshot; 18 is the settled default-screw Ready state. The automation bridge omits some scrolled controls, so corresponding visible screenshots provide those interaction details. These captures establish behavior, not a p95 latency result. All completed QA windows are closed.

Independent visual review inspected 15 of the 20 inventoried frames. Capture 04 retains the earlier camera crop; 05 frames the whole shaft after Fit. Some control values are established by accessibility text rather than the current scrolled raster. The screw head and helix are visible, but its recess faces away; these screenshots do not prove cavity dimensions or fit.

Independent public Rust readback, linked to the exact release libraries used for the bundle, recompiles both actual GUI-saved sources. M8 retains the modeled nominal-basic M8 × 1.25 × 50 mm definition, one solid with 167 faces, volume 2067.982858545 mm³, and bounds approximately [-4, -4, 0] to [4, 4, 50] mm. M3 retains its modeled nominal-basic 0.5 mm pitch, 10 mm thread/underhead length, socket-cap and hex-socket settings; its one solid has 96 faces, volume 126.088748718564 mm³, and bounds approximately [-2.84, -2.84, -3] to [2.84, 2.84, 10] mm. The respective meshes contain 14,738 and 7,888 triangles, with finite coordinates, valid indices and complete native-face coverage. Source files remain unchanged. The receipt and standalone reproduction are `independent-native-saved-thread-and-screw-readback.json` and `read_saved_native.rs`.

## Regression coverage and remaining limits

The new native tests cover fresh 12.5, 50 and fractional 49.8 mm lengths, exact bounds, one-solid validity, analytic volume within the established metrology budget, physical material/void probes, complete face ancestry and independent STEP readback. Invalid short lengths and turn-budget inputs still refuse. Ten existing root-rounded, handed, tapered axial-pipe, rounded-fit and receiving-cut tests pass. Five prototype and six factory tests pass. Final formatting, strict locked all-target workspace Clippy and the release build pass. The complete final `cargo test --workspace --locked` command exits **0: 2600 passed, zero failed, four intentional ignores across 296 groups**, using the pinned toolchain, committed lockfile, optimized test code and enabled debug assertions/overflow checks. Raw logs, exits and the summary are retained in `checks/`.

The custom 50 mm cold construction still takes several seconds. Repeated cached work meets the recorded 250 ms command budget in these samples; first import and some customized reopened edits take longer. CAD-252 retains broad cold-fastener latency work, and CAD-243 retains busy UI input-to-visible-feedback acceptance. This round does not establish a new fit certification, physical manufacture, Windows/Linux interaction, or human acceptance.
