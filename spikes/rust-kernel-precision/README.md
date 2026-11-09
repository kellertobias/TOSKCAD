# Rust kernel precision experiment

This standalone experiment parses exact lengths, classifies coordinates against a configurable grid, and intersects a vertical line with an analytic circle. It runs with the repository's pinned Rust toolchain and no native geometry dependencies. [OCCT baseline](../../docs/architecture/kernel-replacement-baseline.md)

`parse_length_mm` accepts signed decimals and one division with a unit. Examples include `0.1 mm`, `1/3 in` and `10 mm / 3`. An inch is exactly `127/5 mm`. Fractions are reduced to a signed numerator and a positive denominator. Both components have a magnitude limit of `10^18`. Products and sums use `i128`, which is wide enough for intermediate arithmetic on this domain. Inputs and reduced results outside the supported range return an error. A zero denominator has a separate error.

The grid uses a positive rational step. On-grid coordinates return signed 64-bit ticks. Off-grid coordinates retain their exact rational length. At `0.01 mm`, `1/4 mm` is 25 ticks. At `0.001 mm`, the same length is 250 ticks. `1/3 mm` stays off-grid at both settings.

For a circle with center `(cx, cy)` and radius `r`, a line at `x` gives the exact expression `cy ± sqrt(r² − (x − cx)²)`. The sign of the rational radicand distinguishes no intersection, tangency, and two intersections. For the unit circle at `x = 1/2 mm`, the result retains `±sqrt(3/4) mm` in algebraic form. The API returns an explicit range error when any intermediate reduced rational exceeds the bound.

Run the isolated checks from the repository root.

```sh
CARGO_TARGET_DIR="$PWD/artifacts/kernel-precision-build" cargo test --locked --offline \
  --manifest-path spikes/rust-kernel-precision/Cargo.toml
```

The next comparison needs a line-and-arc solid evaluated through the normal compiler, persistent references through an edit, and independent STEP readback. Those checks will establish how the exact lengths and error bounds travel through a complete CAD operation.
