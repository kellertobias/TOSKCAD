# Geometry contract and OCCT baseline

The baseline for the Rust kernel study is MultiCAD commit `0d63338a1e88a53e322ced3418dcccc5dbdfe9f0` with statically linked OCCT 7.9.3. This document records the current contract and the first comparison fixture. The production solver, geometry backend, canonical document, and saved file format use the existing implementations. [Kernel replacement plan](https://plans.tokenet.de/CAD/290)

The [Rust kernel qualification matrix](rust-kernel-qualification.md) records the pinned candidate fixtures and open cutover checks.

## Current geometry contract

The model compiler consumes the Rust `Solid` façade. `Solid` owns a thread-confined OCCT handle. The shared Rust document records feature inputs and semantic references. Native operation history supplies candidates for those references after a rebuild. The renderer uses disposable meshes, while manufacturing export evaluates source geometry through its own checks. [Kernel façade](../../crates/multicad-kernel/src/geometry.rs) · [Compiler](../../crates/multicad-model/src/compiler.rs) · [Geometry policy](0006-geometry-policy.md)

| Surface | Current contract and representative evidence |
| --- | --- |
| Curves and trimmed profiles | The adapter constructs lines, circular arcs and circles and reads supported splines through the advanced curve interface. Profiles preserve their frame, bounded loops and holes. Curved sections, disconnected regions and unsupported boundaries have distinct outcomes. [Profiles](../../crates/multicad-kernel/tests/geometry.rs) · [Curves](../../crates/multicad-kernel/tests/curves.rs) · [Planar contours](../../crates/multicad-kernel/tests/planar_contours.rs) |
| Solids, topology and booleans | A published body requires positive finite volume and validated native topology. Zero-solid intersection is represented separately from a native or input error. Boolean fixtures check contained, disconnected and overlapping solids and preserve the input bodies. [Solid checks](../../crates/multicad-kernel/tests/geometry.rs) · [Components](../../crates/multicad-kernel/tests/solid_components.rs) · [Interference](../../crates/multicad-kernel/tests/interference.rs) |
| Construction and editing | The supported feature families include primitive and profile extrusion, revolve, sweep, loft, taper, draft, shell, split, affine and rigid placement, chamfer and fillet. Compound features also use dedicated builders for threads, gears and sheet metal. Use the feature recipes and current constraints when deciding scope. [Profiles and lofts](../../crates/multicad-kernel/tests/advanced_profiles.rs) · [Edge treatment](../../crates/multicad-kernel/tests/edge_treatment.rs) · [Threads](../../crates/multicad-kernel/tests/threads.rs) · [Gears](../../crates/multicad-kernel/tests/gears.rs) · [Sheet metal](../../crates/multicad-model/tests/sheet_metal.rs) |
| Persistent references | A topology snapshot gives elements analytic descriptions, stable source roles and candidates from operation ancestry. A parameter edit or placement can change the native ordinal. Consumers must resolve against the current source and expose ambiguity, stale input, or deletion for repair. [Provenance](../../crates/multicad-kernel/tests/provenance.rs) · [Analytic geometry](../../crates/multicad-kernel/tests/analytic_geometry.rs) · [Sketch references](../../crates/multicad-kernel/tests/sketch_references.rs) |
| Queries and drawings | Distances and witnesses come from finite selected B-rep geometry. Planar contours preserve analytic arcs. Drawing output uses native section, hidden-line and projection operations. [Distance](../../crates/multicad-kernel/tests/distance.rs) · [Face measurements](../../crates/multicad-kernel/tests/face_measurements.rs) · [Drawings](../../crates/multicad-kernel/tests/drawing.rs) |
| Tessellation | Display meshing is tolerance-based. Manufacturing meshing has separate checks for geometry, dimensions, degenerate triangles, surface orientation and watertightness. Changing display deflection leaves the exact topology intact. [Display](../../crates/multicad-kernel/tests/display.rs) · [Manufacturing mesh](../../crates/multicad-kernel/tests/manufacturing_mesh.rs) |
| Exchange and files | `import.step_source.1` embeds OCCT B-rep text for the supported STEP bodies. Rebuild uses that text even when the original STEP file is absent. Pinned projects can contain the same import records. Thread prototype B-reps are derived application assets and use separate native fingerprints. [Imports](imports.md) · [STEP round trip](../../crates/multicad-kernel/tests/step_exchange.rs) · [Prototype receipts](../../crates/multicad-kernel/src/thread_prototypes.rs) |
| Failures and publication | Invalid inputs, unsupported geometry, native failure, invalid results and worker failures are explicit. Preview and apply retain the last valid source until a matching accepted result is available. Stale publication and cancellation are tested separately from the time required for a native call to stop. [Geometry errors](../../crates/multicad-kernel/src/geometry.rs) · [Worker tests](../../crates/multicad-kernel/tests/worker.rs) · [Publication](geometry-jobs.md) |

The kernel build uses CXX and the shared OCCT build recipe. Packaging, notices, and diagnostics also identify or ship that dependency. Replacing the kernel requires updating each of these consumers. STEP B-reps stored in projects need a compatibility policy independent of disposable geometry and drawing caches. [Build](../BUILDING.md) · [Native build](../../crates/multicad-native-build/) · [Packaging](../../tools/native-package.mjs)

## First comparison fixture

The [release-mode baseline executable](../../crates/multicad-kernel/examples/kernel_slice_baseline.rs) uses a rectangular profile with a semicircular end and an authored circular hole. It extrudes the profile and cuts a second circular bore through the panel. Two thickness and radius combinations and two cutter seam placements produce four cases. The report records extrusion, cutter creation, subtraction and verification separately.

The analytic check computes the original area as `40 × 20 + π × (10² / 2 − 3²)` square millimeters. The required material change is `π × radius² × thickness` cubic millimeters. Top cap contours must contain one semicircular outer edge and two circular holes with the authored centers and radii. Area and volume checks use an absolute tolerance of `1e-6` in their respective units. Arc dimensions and sweep use `1e-7` mm and radians. These checks catch an unchanged solid even when the subtraction reports success. Rotating the cylinder changes its seam and leaves the intended bore geometry identical.

Two 10 mm cubes check a `0.01 mm` gap, face contact and `0.01 mm` overlap. The first two yield no volume. The last yields one cubic millimeter. Zero-radius construction must return an input error. The `0.01 mm` spacing is a test value for the candidate comparison. The OCCT baseline uses the existing `1e-7 mm` linear tolerance, zero added Boolean fuzz and floating-point inputs. It does not establish an integer lattice, exact authored arithmetic or general curved-intersection support.

Build and record 20 fresh process samples from the repository root. Each invocation emits one JSON line. Preserve the executable and record the source commit, machine, process and timing scope alongside the output.

```sh
cargo build --release --locked -j 2 -p multicad-kernel --example kernel_slice_baseline
mkdir -p artifacts/benchmarks/kernel-slice-occt
set -e
for trial in $(seq 1 20); do
  target/cargo/release/examples/kernel_slice_baseline
done > artifacts/benchmarks/kernel-slice-occt/raw.jsonl
```

The per-operation intervals are kernel calls in one process. The first extrusion is the first timed geometry operation in each process. Later cases may reuse internal initialization. The harness does not measure process startup, model compilation, input-to-frame latency, UI cancellation or STEP interoperability.

### Reference measurement

On an Apple M5 Max with 18 CPU cores and macOS 26.6.2, twenty fresh optimized processes passed all 80 panel and 60 contact cases. The maximum observed cut-volume error against the analytic result was `1.1e-11 mm³`. Peak process RSS ranged from 8.03 to 8.17 MiB. The OCCT source identity was `febdb33f3c2570f3ab04a19062594133f1f9c2fde624348aad8595fb85c95cd5`. The executable used Rust 1.99.1 and its SHA-256 was `51864ad8fe9c47fa6df161e9877684d8770830ddd2ec051178f09b5ed6c13391`.

| First panel operation | Median | Observed range |
| --- | ---: | ---: |
| Extrude profile | 1.018 ms | 0.989 to 1.150 ms |
| Construct cutter | 0.185 ms | 0.182 to 0.189 ms |
| Subtract cutter | 2.559 ms | 2.496 to 2.672 ms |
| Verify analytic expectations | 2.658 ms | 2.634 to 2.714 ms |

The raw JSONL observations, process timing and machine record are under `artifacts/benchmarks/cad338-occt-baseline/` in the local workspace. The comparison still needs paired trials against a candidate.

## Workload evidence and comparison limits

The existing release-budget harness covers parameter edits on the tool case and control desk and a larger plate field. Its historical cold and incremental measurements use different builds and system load. The prior herringbone trace separates construction, nine sampled interference phases and the current pose. The metric-thread traces distinguish the cold physical build and exact geometry reuse. Retain these stage boundaries in the matched comparison. [Rebuild evidence](../verification/incremental-rebuild-2026-10-30.md) · [Herringbone timing](../verification/gear-pair-performance-2026-11-06.md) · [Thread timing](../verification/metric-thread-capacity-2026-11-05.md)

The next study needs a stable downstream reference through the normal compiler, exact authored dimensions through UI and MCP, and the curved interior-loop, tangency and disjoint fixtures. The circular cutter seam control here cannot substitute for the proposed helical regression. General surface intersection, independent STEP readback, Rust-native file reopening, interactive responsiveness, matched performance trials and package cutover require separate evidence. [Standalone precision experiment](../../crates/multicad-rust-kernel/README.md)
