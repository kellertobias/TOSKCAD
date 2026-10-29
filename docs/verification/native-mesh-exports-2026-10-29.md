# OBJ and 3MF export verification, 2026-10-29

CAD-91 adds OBJ and 3MF to the native export provider, alongside existing STEP.
The [export sets note](../architecture/export-sets.md#mesh-output) describes the
writers, hierarchy modes and publication checks.

## Commits

| Commit | Step |
| --- | --- |
| `98ba0ba` | Kernel: tessellation winding follows the face's orientation in the solid |
| `78fa3cb` | OBJ/3MF writers, separate and assembly hierarchies, units, evidence, publication readback, Export preview labels |

## Defect found and fixed

The first OBJ readback measured a 10 × 20 × 30 mm box as 2000 mm³. The kernel
tessellator took each face's orientation from its detached copy, which loses
the orientation composed in the solid, so reversed faces kept inward winding.
The box's signed mesh volume was −2000 mm³.

- **Fix.** Winding now follows the exact topology map.
- **Test.** A new kernel test requires the box mesh to enclose exactly
  6000 mm³, and a cylinder mesh to lie between 99% and 100% of πr²h.
- **Regression.** All existing display, viewport and drawing tests still pass.

Publication now also compares each mesh's volume with the exact solid's volume,
within a chord allowance.

## Independent readers

[`model/tests/mesh_exports.rs`](../../crates/multicad-model/tests/mesh_exports.rs)
reads the files with code that shares nothing with the writers:

- a local OBJ parser;
- a ZIP reader that walks the central directory and checks each CRC;
- quick-xml for the 3MF model;
- the system `unzip -tq` when it is installed. It ran and accepted every 3MF
  on macOS.

The fixture uses parametric primitives:

- a 10 × 20 × 30 mm Panel tagged `cnc`;
- an R5 × 10 mm Pin;
- a Bench assembly with a rotated and a moved copy of the Panel.

Results:

- **Separate parts.** OBJ in millimeters and 3MF in inches write eight files
  for four subjects.
  - Every file's world bounds match the exact solid within the 0.05 mm chord.
  - Box volumes are exactly 6000 mm³, and the pin lies between 99% and 100% of
    250π mm³.
  - The 3MF declares `unit="inch"` with one object and one build item, and its
    coordinates scaled back match.
  - The rotated copy's OBJ lies at x 30–50, y 5–15, z up to 32, as its STEP
    does.
  - Objects carry product names ("Panel …").
- **Assembly.** An assembly-scope 3MF in millimeters is one file with one
  shared object, the local definition at 0–10, 0–20, 0–30.
  - Its two build items place it at exactly (−20…−10, 40…60, 0…30) and
    (30…50, 5…15, 2…32), each 6000 mm³, with no double transform.
  - The assembly OBJ, in centimeters, has two world-space objects totalling
    12 cm³.
- **Tag scope and reproducibility.** A `cnc` tag scope selects the Panel and
  both copies, never the untagged Pin. Two runs into separate folders produce
  byte-identical 3MF files, and a repeated run reuses the verified bundle.
- **Failure cases.** GLB reports CAD-131, and OBJ with removable-aid exclusion
  reports Unsupported. Neither can run, and nothing is written. The earlier
  bundle and the project source are unchanged.

The existing STEP readback tests still pass.

## Verification commands

All on the committed tree at `78fa3cb`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,368 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-p1ybuh/MultiCAD.app`, executable SHA-256
  `9d7dd6b62fb2116bc01ca8df10449695027a525a6736b0502131395a9595bc32`.
- Logs are in `/private/tmp/multicad-cad91/`.

## Limitations

- **No third-party reader in tests.** The ZIP/3MF test readers are written in
  this repository, separately from the writers. A third-party ZIP crate needs
  a network fetch, so the system `unzip` is the third-party check when it is
  installed. Slicer interoperability, such as PrusaSlicer or Bambu Studio, was
  not tested.
- **STEP assemblies.** Assembly-preserving STEP and STEP product names remain
  Unsupported. Placed occurrences export as separate world-space STEP files.
- **No appearance data.** Mesh files carry no color, material or texture.
  There are no 3MF materials or OBJ MTL files yet.
- **Removable aids.** Exclusion needs the future typed aid provider. STL and
  GLB belong to CAD-131.
- **No interactive macOS check.** The Export workspace runs were driven
  through the API and tests only.
