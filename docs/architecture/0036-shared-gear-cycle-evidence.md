# ADR-0036: Application-owned gear cycle evidence

Status: accepted implementation design, 2026-11-06. Verification is pending
under [CAD-232](https://plans.tokenet.de/CAD/232).

Preview, display, drawings, manufacturing and MCP have independent native
owners. Repeating the nine sampled phases in each owner is expensive even
after another owner has completed the same native validation.

The desktop composition owns one bounded `PairCycleProofStore` and explicitly
injects clones into its worker factories and MCP hosts. Standalone headless
hosts own independent stores. No store is process-global or saved in a project.
The compiler and exporter remain copyable. A scoped detached handle transfers
into a worker at construction; native solids remain on their existing owners.

Entries contain only the complete versioned key and immutable phase angles.
The key retains all native constructor inputs, pair/member/output identities,
member and base frames, geometry policy, engagement mode and family context,
and adds the canonical document identity. Only the periodic drive is excluded.
Every compile still validates canonical ownership and the actual current pose
using native solids; a hit skips only the completed sampled cycle.

New evidence is staged until the outermost full compilation and its final
interruption checkpoint succeed. Nested template compiles merge staged entries
into their parent; failure or unwinding discards them. Source-scoped compilation
cannot mint cycle evidence. A completed native compile is the publication
boundary, separate from later artifact writing, UI Apply or human acceptance.

Retained and staged payloads are bounded to 64 entries and 1 MiB of key/angle
bytes per compilation scope. Committed entries use least-recently-used eviction.
The short mutex protects detached lookup, accounting and publication only;
native checks run outside it. Concurrent misses may duplicate work. Poisoning
or an oversized entry loses reuse rather than accepting unproved geometry.
Explicit shared clear advances an epoch, preventing older in-flight work from
repopulating the store. An injected store takes precedence over worker-local
receipts, so clearing or switching it cannot fall back to older local proofs.
Clearing a native feature cache leaves the application store intact.

This removes repeated cycle work, not first construction, required current-pose
Booleans, occurrence provenance or tessellation. Controlled lane timings and
actual macOS progress/cancellation checks must establish the observed result.
No near-instant generation, continuous engagement or physical fit is promised.

`Model::timings()` and MCP `evaluate_model.native_timings` report successful
compiler wall times. Pair-validation time contains the per-pair sampled-cycle
and current-pose times. The remaining buckets are partial, non-exhaustive work
categories, not a sum of total time. Optional `MULTICAD_MODEL_TIMINGS_JSONL`
appends these diagnostics, document/revision, owner name, display setting and
pair receipts to a local file. Total is captured before diagnostic writing.
Failed/cancelled jobs require external timing; these records do not measure
input-to-visible-frame latency or later export serialization.
