# ADR-0039: Shared arithmetic for document and geometry lengths

| | |
| --- | --- |
| Status | Accepted implementation design; application cutover and human acceptance remain separate |
| Date | 2026-11-11 |
| Deciders | Codex, within the implementation authority for CAD-290 |

## Context

A thread tool and its stock must agree on equal authored lengths before the kernel classifies a residual floor.
The regression tests reproduce a mismatch for `1.956 mm`, including its equivalent formula and parameter binding.
Rounding a rational numerator and denominator separately caused another mismatch for a Unified thread length.
[Dimension contracts](../../crates/multicad-core/tests/dimension_literals.rs) and [thread workflows](../../crates/multicad-model/tests/rust_unified_threads.rs) retain these cases.

The document core is independent of the geometry kernel.
Exact formula evaluation previously belonged to the model provider, while other callers evaluated dimensions through floating SI quantities.
The same canonical dimension therefore needs one evaluator available to document tools and geometry providers.

## Decision

Use the independent `multicad-precision` crate for bounded rational arithmetic and coordinate quantum operations.
The geometry kernel reexports its existing public numeric types from this crate.
The document core depends on precision without depending on geometry.

Keep exact length evaluation in `multicad-core::parameters::exact_length_mm` and reuse it from the model provider.
`Dimension::value_in` preserves a length literal unchanged when its requested unit matches its authored unit.
Length expressions and conversions use the shared exact evaluator when representable, then round the result once for floating consumers.

Formula comparisons retain the existing Project evaluator's reference arithmetic so geometry follows the same conditional branch.
Expressions outside the bounded evaluator's supported arithmetic retain the existing finite floating evaluation path.
Angles retain their existing SI conversion path for both literals and expressions.
Evaluation leaves the retained document source unchanged.

## Consequences

**Positive:** Equal supported length expressions reach geometry providers through the same arithmetic and rounding rule.
The existing geometry API remains available through reexports, and both consumers share one rational implementation.

**Negative:** The workspace has an additional crate, and kernel coverage must include `multicad-precision` alongside `multicad-rust-kernel`.
Exact expression queries repeat parsing and dependency evaluation; a future cache needs measured cost and explicit invalidation rules.

**Neutral:** Literal fields retain their existing `f64` serialization, which the exact evaluator reads through its shortest decimal spelling.
Formula fields retain their authored text and bindings.
This decision does not complete geometry, desktop, exchange or package qualification.

## Alternatives considered

**Let the document core depend on the geometry kernel.** This would reuse its rational implementation with fewer package changes.
It would also couple document evaluation to the geometry dependency graph, so the independent precision crate owns the shared arithmetic.

**Keep separate document and model evaluators.** This would preserve the dependency layout with a smaller initial change.
It would retain the arithmetic disagreement that caused the through-thread regressions.

## References

- [Precision implementation](../../crates/multicad-precision/src/lib.rs)
- [Canonical exact evaluator](../../crates/multicad-core/src/parameters/exact_length.rs)
- [Dimension query](../../crates/multicad-core/src/constraint_sketch.rs)
- [Model compatibility import](../../crates/multicad-model/src/rust_backend/exact_length.rs)
- [CAD-290](https://plans.tokenet.de/CAD/290)
