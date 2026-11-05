# ADR-0033: Integrate closed volumes with an orthonormal Gauss-Kronrod basis

| | |
| --- | --- |
| Status | Accepted engineering decision; execution evidence remains separate |
| Date | 2026-11-05 |
| Decider | Codex, using delegated implementation authority |

This record owns the mass integral in the [native volume adapter](../../crates/multicad-kernel/src/bridge.cpp).
The [model evaluator](0012-native-model-evaluation.md) owns source evaluation.
Centroid and inertia calculations retain their own contracts.

## Context

Native volume integration stalled while adopting valid zero-bore bevel solids.
The [captured native profile](../../artifacts/bevel-review-20261105/checks/model-zero-bore-volume-sample.txt)
locates the work inside the Gauss-Kronrod integral. Exact planar meridian faces
alone did not eliminate the stalled whole-solid calculation.

Radial transition faces have nearly zero contributions about an axial reference.
Refinement of their floating-point residuals is the numerical hypothesis, because
the per-face diagnostic changed that reference and did not identify the failing
whole-integral contribution.

The pinned OCCT 7.9.3 public API offers point and plane forms of
`BRepGProp::VolumePropertiesGK`. The plane form integrates a directional flux
over the same oriented, closed native boundary. Imported STEP solids need the
same volume checks as solids constructed from recipes.

## Decision

Average three public plane-overload integrals for each closed shell. Preserve
the pinned wrapper's shell selection, orientations and repeated occurrences.
Use its reference point: the average of vertex occurrences in native traversal
order, with the same triangulation-node or origin fallback when vertices are absent.
That fallback supplies a reference point; the native surfaces still supply the integral.

Construct unit directions proportional to `(1,sqrt(2),sqrt(3))` and
`(-sqrt(2),1,0)`, with their cross product as the third direction.
For each direction n, use the same shell reference v and plane coefficient
`D=n dot v`. The pinned implementation evaluates `(n dot N)*(n dot x-D)`.
The coefficient convention differs from the geometric plane offset.

For an orthonormal basis, `sum(n_i*n_i^T)=I`. Averaging the three integrands
therefore gives `(x-v) dot N/3`, the original point integrand. This preserves
its algebra even when the trimmed boundary has a small residual directional flux.

Retain the requested per-face tolerance `1e-10`, closed-shell selection, spline
spans and the existing whole-volume relative-error limit `1e-7`. Preserve signed
face orientations and existing shared-face handling. Native validity and a finite
positive total mass remain required before caching a certificate. A failed
integral must not create a reusable certificate.

Average signed masses and absolute error estimates; inward cavity shells must
subtract from the total. The public wrapper returns relative error only when
its mass exceeds `Epsilon(1.)`; otherwise its estimate is absolute, including
for negative cavity contributions. Convert each estimate accordingly before
aggregation, then divide the aggregate absolute error by the final positive mass.

Keep the mass cache and its concurrency ownership unchanged. Regenerate cached
thread receipts when the adapter fingerprint changes, and compare the resulting
manufacturing geometry independently of receipt metadata.

## Consequences

Positive: constructed solids and imported STEP solids share the original point
integrand without depending on a source-family label or stored analytic volume
formula. The existing numerical acceptance limits continue to govern the result.

Negative: each uncached shell requires three native integrals. A basis direction
can still align with a face after rotation, so finite tests do not guarantee
convergence for every solid. Translated and scaled cases require regression
coverage, and the reported native error estimate remains an estimate.

Neutral: the choice does not alter the solid, its manufacturing geometry or
display tessellation. Mass tests do not establish centroid or inertia accuracy.

## Alternatives considered

**Retain the point integral.** This keeps the existing public overload and avoids
a new direction choice. The observed stalls on valid zero-bore bevel examples
prevent its use for those interactive cases.

**Use one offset directional integral.** It uses one public call and avoids the
point reference's radial contributions. The
[native comparisons](../../artifacts/bevel-review-20261105/volume-diagnostics/analysis-summary.json)
found offset-dependent masses outside the consistency limit on unchanged thread
BReps. Tightening the integration tolerance did not reconcile those values.

**Attach a bevel-specific analytic certificate.** A formula could avoid the native
integration cost for declared recipes. It would require a separate proof for
the actual approximated boundary and would leave imported STEP solids on the
stalled path.

**Use the fixed integration rule.** It reduces adaptive work and uses an existing
public API. It does not supply the adaptive error estimate required by the
current acceptance guard.

## References

- Pinned source: `artifacts/native-deps/OCCT-7_9_3/src/BRepGProp/BRepGProp.cxx`,
  `BRepGProp_UFunction.cxx`, `BRepGProp_TFunction.cxx` and `BRepGProp_VinertGK.cxx`.
- [Independent native and STEP volume cases](../../crates/multicad-kernel/tests/spherical_bevel.rs).
- [Existing volume-cache and failure controls](../../crates/multicad-kernel/src/geometry.rs).
- [Independent comparisons on the registered thread geometry](../../artifacts/bevel-review-20261105/volume-diagnostics/all-registry-basis-integrals.json).
