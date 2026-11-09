# Rust kernel precision and rounded panel experiment

This Rust kernel crate builds a rounded panel with an authored circular hole and a second bore cut through the solid. It also tests exact lengths and analytic circle intersections. It runs with the repository's pinned Rust toolchain and no native geometry dependencies. [OCCT baseline](../../docs/architecture/kernel-replacement-baseline.md)

`parse_length_mm` accepts signed decimals and one division with a unit. Examples include `0.1 mm`, `1/3 in` and `10 mm / 3`. An inch is exactly `127/5 mm`. Fractions are reduced to a signed numerator and a positive denominator. Both components have a magnitude limit of `10^18`. Products and sums use `i128`, which is wide enough for intermediate arithmetic on this domain. Inputs and reduced results outside the supported range return an error. A zero denominator has a separate error.

The grid uses a positive rational step. On-grid coordinates return signed 64-bit ticks. Off-grid coordinates retain their exact rational length. At `0.01 mm`, `1/4 mm` is 25 ticks. At `0.001 mm`, the same length is 250 ticks. `1/3 mm` stays off-grid at both settings.

For a circle with center `(cx, cy)` and radius `r`, a line at `x` gives the exact expression `cy ± sqrt(r² − (x − cx)²)`. The sign of the rational radicand distinguishes no intersection, tangency, and two intersections. For the unit circle at `x = 1/2 mm`, the result retains `±sqrt(3/4) mm` in algebraic form. The API returns an explicit range error when any intermediate reduced rational exceeds the bound.

## Rounded panel

The profile uses three axis-aligned segments and one semicircle. The solid extrudes along positive Z from zero. Both circular holes must lie strictly inside the profile and be separated from other holes. A subtracting cylinder must extend beyond both caps. The experiment reports tangency, overlap, and unrepresentable arithmetic as errors and leaves the input solid unchanged.

Top area and volume remain exact as rational coefficients of `constant + π × factor`. A point probe distinguishes material, boundary, and empty space. A face key uses an authored 128-bit ID and a surface role; the tests rebuild the panel with a fractional thickness and a different bore diameter, then resolve the same keys. The [panel tests](tests/prism.rs) compare the exact measures against the recorded OCCT results for both panel sizes and cutter seam orientations.

The compiler can select the Rust backend for a whole evaluation of a rounded XY sketch extrusion, an axial cylinder, and a through-cut. A saved panel can rebuild after a fractional height edit and carry its attached sketch plane to the edited cap. The [compiler tests](../multicad-model/tests/rust_backend.rs) compare both cutter seam orientations with OpenCascade and check that a tangent bore fails explicitly.

The Rust backend publishes a display mesh and picking keys for the panel and cylinder. It uses [Spade](https://docs.rs/spade/2.15.1/spade/struct.ConstrainedDelaunayTriangulation.html) to triangulate sampled cap boundaries and checks the material side of every boundary edge. Planar contour queries read the analytic topology, so a bore remains a circle in the contour regardless of display mesh density. App backend selection, general trimmed surfaces, helical seam tests, independent STEP readback, and matched workload timings still need verification.

## Verification

Run the isolated tests and coverage checks from the repository root. The coverage command requires `cargo-llvm-cov` and the pinned toolchain's `llvm-tools` component.

```sh
cargo test --locked -p multicad-rust-kernel
cargo llvm-cov --locked -p multicad-rust-kernel \
  --fail-under-lines 100 --fail-under-functions 100
```
