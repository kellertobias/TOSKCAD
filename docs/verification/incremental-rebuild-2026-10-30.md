# Incremental model rebuilds, 2026-10-30

These checks are agent verification for CAD-150, not human acceptance. All
numbers are **headless** measurements on one development Mac. No window was
launched, so the timings say nothing about frame rate or how responsive
editing feels in the app. They cover only the compiles measured here.

## What changed

Before this change, every compile re-ran every feature: its native provider,
topology export and display tessellation. The model runtime now keeps an
`EvaluationCache` (`crates/multicad-model/src/eval_cache.rs`) on its worker
thread and compiles with `Compiler::compile_cached`. Each compile still walks
the whole canonical document in dependency order. When a feature's **result
key** matches an earlier compile, the compile reuses that compile's exact
solid and its display/topology snapshot:

- **Key.** SHA-256 over these inputs:
  - the cache format;
  - the geometry policy and display mesh options;
  - the feature identity;
  - the feature's *input closure*: each entity in it with its full canonical
    source and, for parameters, the evaluated value.

  The closure follows two kinds of edge, transitively. One is
  `Document::evaluation_dependencies`, the graph that already orders the
  compile. The other is every entity identity mentioned in the source,
  including expression bindings and IDs embedded inside strings. A Part that
  is only mentioned contributes its own source plus the identity, type and
  suppression of its listed features. Geometry can only flow along evaluation
  dependencies, so nothing more is needed.
- **Revision.** The revision is not part of the key. A reused snapshot is
  re-stamped with the current revision.
- **Which features are cached.** Only audited provider types:
  - Extrude, Primitive, Revolve, Boolean, Split, Transform, Edge treatment,
    Advanced and Align;
  - cabinetry Panel, Joint cut and Machining cut;
  - Sheet and Sheet hardware;
  - FDM aid;
  - the STEP body, Board and Component imports;
  - weldment Member, whose scanned connections and connected partner members
    are declared as hidden inputs.

  Some providers find related entities by scanning the document or resolve
  assembly poses: architecture walls, slabs and stairs, Transfer, and Scale.
  These are always evaluated, and so is any feature that depends on one of
  them, on an Instance or Assembly, or on a sketch targeted by an occurrence
  reference. Unknown or new provider types are not cached until someone
  audits them. The architecture types are also changing in parallel work
  (CAD-148).
- **Sharing.** A hit shares the immutable B-rep (`Solid::share`, a TopoDS
  handle copy) and the `Arc` snapshot. It also restores side results: panel
  frames, sheet layouts, member paths and consumed tool versions. Kernel
  operations never modify their inputs. The drawing section intersection was
  the only Boolean without `SetNonDestructive(true)`; it now has it.
- **No stale results.** Entries are stored only after a successful
  evaluation, so a superseded or failed compile can leave only valid entries.
- **Memory bound.** After each compile, entries used by that compile are kept;
  their geometry is shared with the returned model. Older entries are evicted
  least-recently-used first, down to 256 MiB of *estimated* size and
  4 × `MAX_MODEL_FEATURES` entries. The estimate covers mesh, edge and vertex
  arrays plus 1.25 KiB per topology element for the topology record and
  B-rep; it is not a measured native allocation.
- **Threads.** The cache holds kernel solids, so it is `!Send`/`!Sync` and
  lives on the thread that compiles. Plain `compile`/`compile_interruptible`
  are unchanged. Cancellation still uses the existing per-entity checkpoints.

Not done, with reasons:

- **Parallel evaluation of independent branches.** It needs `Solid` across
  threads, and the kernel deliberately forbids that.
- **Tessellation as a separate step.** Reused features already skip it.

## Machine, toolchain and source

| Item | Value |
| --- | --- |
| Hardware | Apple M5 Max, `Mac17,6`, 18 cores (6 performance, 12 efficiency), 64 GiB RAM, AC power |
| OS | macOS 26.6.2 (Darwin 25.6.0) |
| Toolchain | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, pinned, `--locked --offline`, `release` profile |
| Before | commit `f358f8c` (local main before CAD-150), measured from a detached worktree |
| After | commit `f35d754` (this branch), clean tracked tree |
| Background load | Other agent sessions shared the machine. Load average was about 10–12 during the after run and 4–8 during the before run. Medians of 3 fresh processes are reported, and the spread is listed. |

## Reproduce

```sh
node tools/release-budgets.mjs --profile release --repeat 3 --label cad150-after
# before: the same command in a worktree at f358f8c (four scenarios only)
```

The harness has two new parts:

- An `incremental` section for each scenario. It reports cache fill, cached
  warm compile, the cached rebuild after the scenario's parameter edit (with
  evaluated and reused feature counts), an exact equality check of the full
  snapshot against the plain compile, and the cached rebuild after undo.
- A generated fifth scenario, `large_plate_field`
  (`crates/multicad-app/examples/large_scenario.rs`): 10 × 20 drilled plates,
  each a parameter-driven Box, a Cylinder and a Cut Boolean. That is 200
  parts, 600 features and 36 800 triangles. The edit sets `row_0_width`, which
  drives the 20 plates in row 0.

## Results

The **plain** columns use the unchanged uncached compile path. The **cached**
columns use the path the model runtime now takes. All times are in
milliseconds.

| Scenario | Features | Before: warm compile | Before: edit rebuild | After: plain edit rebuild | After: cached warm compile | After: cached edit rebuild | Edit: evaluated / reused (uncacheable) | Cached rebuild after undo |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | ---: |
| Makerspace | 7 | 102 | 102 | 103 | 95 | 97 | 4 / 3 (4 walls/slabs/stair) | 97 |
| Cabinetry and table | 26 | 337 | 337 | 334 | 13 | 292 | 18 / 8 (0) | 13 |
| Tool case | 28 | 672 | 675 | 713 | 628 | 643 | 12 / 16 (8: 4 transfer, 4 dependent aids) | 576 |
| Control desk | 17 | 937 | 945 | 928 | 299 | 897 | 10 / 7 (1 transfer) | 300 |
| Large plate field | 600 | 2 110 † | 2 116 † | 2 116 | 34 | 195 | 40 / 560 (0) | 40 |

† The large scenario does not exist at `f358f8c`. Its "before" values are the
plain compile measured in the after build. That compile path is the
pre-CAD-150 code, unchanged.

Spread of the cached edit rebuilds over 3 processes, in ms:

| Scenario | Runs |
| --- | --- |
| Makerspace | 96.5, 98.1, 95.7 |
| Cabinetry and table | 291.5, 292.0, 293.9 |
| Tool case | 688.8, 610.1, 642.6 |
| Control desk | 982.9, 894.4, 897.5 |
| Large plate field | 194.7, 201.6, 195.0 |

Under the higher load the tool-case plain timings were noisy (warm 663–829 ms
after vs 658–699 ms before).

Other observations:

- **Identical results.** All 15 cached edit rebuilds produced a snapshot
  exactly equal to the plain compile (`edit_identical_to_plain_compile`).
- **Cache fill cost.** The first cached compile cost the same as a plain
  compile within noise: 102 / 342 / 708 / 930 / 2 126 ms. Key hashing for 600
  features is part of the 34 ms cached warm compile of the large field.
- **Cache size after the run.**

  | Scenario | Entries | Estimated size |
  | --- | ---: | ---: |
  | Makerspace | 3 | 0.1 MiB |
  | Cabinetry and table | 44 | 2.6 MiB |
  | Tool case | 24 | 1.1 MiB |
  | Control desk | 25 | 2.8 MiB |
  | Large plate field | 640 | 21.7 MiB |

- **Peak RSS of a measure process rose.**

  | Scenario | Before | After |
  | --- | ---: | ---: |
  | Makerspace | 37 MiB | 38 MiB |
  | Cabinetry and table | 45 MiB | 56 MiB |
  | Tool case | 67 MiB | 88 MiB |
  | Control desk | 64 MiB | 70 MiB |
  | Large plate field | n/a | 317 MiB |

  The process now holds the cache alongside the plain models it also builds.
  The RSS of the runtime worker in the app was not measured.

What the numbers show:

- **Reuse works where providers are cached.** A one-parameter edit re-ran 40 of
  600 features on the large field, and the cached rebuild took 195 ms instead
  of 2 116 ms. Cabinetry re-ran 18 of 26 features. An undo/toggle back to an
  earlier state reused everything cacheable: 13 ms for cabinetry, 40 ms for
  the large field.
- **Uncached providers dominate the rest.** The makerspace walls/slabs/stair,
  the tool-case and control-desk Transfer features, and their dependents still
  run on every rebuild. That is why those scenarios improve little: tool case
  628 ms and control desk 299 ms even with nothing changed. Making Transfer and
  the architecture providers cacheable needs their pose-resolution and
  document-scanning inputs declared. That is follow-up work.
- **Cost scales with the edited region.** On the large field, the cost of an
  edit follows the number of affected features, not the model size: 40
  features take about 195 ms, while the fixed per-compile overhead is about
  34 ms for 600 features.

## Tests (real runs)

- `crates/multicad-app/tests/incremental_rebuild.rs` compares the complete
  `ModelSnapshot` (shapes, meshes, topology names, volumes, references and
  side results) of cached compiles exactly against cold compiles:
  - four acceptance scenarios: first compile, unchanged recompile (only
    uncacheable providers re-run) and parameter edit;
  - the large scenario, with pinned re-evaluation counts: a row edit re-runs
    12 of 72 features, a hole-radius edit re-runs 48;
  - seeded randomized edit/undo sequences on the large scenario,
    cabinetry/table and the tool case, including values providers reject (the
    errors must match too);
  - cached compiles interrupted at spread boundaries: supersession is
    reported, and the next cached compile equals a cold compile;
  - eviction: with a zero retained budget only the latest compile's entries
    survive; with the default budget, toggling back reuses everything.
- `crates/multicad-app/tests/release_hardening.rs`. The CAD-97 pin
  `incremental_status_*` now asserts that the cached makerspace edit re-runs
  only the 4 never-cached architecture providers, reuses 3 features and equals
  the cold result; a plain compile still performs every step. A new runtime
  test checks that the snapshots the runtime publishes after successive edits
  exactly equal cold compiles.
- `crates/multicad-kernel/tests/share.rs` checks that a shared solid matches
  its source and outlives it, and that operations on either owner leave the
  other unchanged.
- `eval_cache` unit tests cover finding identities in values, strings and
  keys, and the cacheable-type allowlist.

## Not verified

- No interactive check of rebuild responsiveness in the native window. No
  low-power target Mac. No Windows/Linux runs; CI has not executed these
  tests.
- The memory bound uses estimated sizes. Native OCCT memory per entry was not
  measured.
- The purity audit is based on reading source for the allowlisted providers.
  A future change that makes one of them read unlinked entities would need to
  update `cacheable_type`/`hidden_inputs`. The randomized tests cover only the
  exercised scenarios.
- The modeling and drawing runtimes in `multicad-app` use the model
  `Runtime` and so get the cache. Direct compiles still use the plain
  uncached path: MCP `tools.rs`, mechanical imports and the export pipeline.
