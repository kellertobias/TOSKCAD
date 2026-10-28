# Manufacturing properties

The [`materials`](../../crates/multicad-core/src/materials/mod.rs) module stores reusable manufacturing definitions and resolves effective Part properties. The native Model Materials editor calls its shared transactional API. Native scene publication reads the same resolved appearance.

Stock specifications describe authored manufacturing assumptions. Assigning stock does not resize a solid, verify thickness, calculate bend allowances or certify a printing process. Those operations require geometry-aware domain tools.

## Canonical ownership

Existing `Material` entities retain density, base color and an optional embedded section hatch. `Part.material` remains the authoritative manufacturing material link. Existing `Part.tags` retain their identity independently of display names.

Reusable appearance, stock and section hatch definitions use versioned `Extension` entities. `MaterialStyle` associates default appearance and hatch definitions with a Material. `PartProperties` associates stock and optional style overrides with a Part. Association records link to their owners, so adding them does not create a Part dependency on its own downstream record.

[`source.rs`](../../crates/multicad-core/src/materials/source.rs) validates the known definitions and their exact link roles. It rejects duplicate association records for an owner. Missing and wrong-kind targets remain readable for repair; they are reported by `diagnostics(document)` and rejected when resolving an effective assignment.

The native container derives `materials.library.1` when the document contains these definitions or a supported inline hatch. Readers without this capability reject opening the file. The module leaves the existing Material and Part wire fields unchanged; project resources and display caches do not own manufacturing properties.

## Resolution and acknowledgement

A Part's explicit appearance or hatch override takes precedence over its Material defaults. Without a linked appearance, the Material base color supplies the visible color. An unsupported embedded legacy hatch produces an error when no supported hatch override replaces it.

`LibraryRef` records a definition ID and its acknowledged content hash. References remain live: editing a referenced appearance updates its effective color. A changed hash produces a review diagnostic until the assignment or Material defaults explicitly acknowledge current content. Renaming a library entry leaves this hash unchanged because the fingerprint covers its entity data.

Broken explicit links produce errors instead of substituting unrelated defaults. An unused broken Material default does not prevent a valid Part override from resolving. The document-level diagnostic still identifies the broken default.

`Resolver::new(document)` builds an immutable index for batch consumers. Scene publication creates one resolver for the snapshot and resolves each Part through it. The Inspector caches derived labels and diagnostics by document identity, revision and selected Part IDs. Stable UI painting does not serialize the entire document.

Metadata associations propagate cache invalidation to their owners and dependent drawing consumers. This propagation leaves canonical references, `affected()` and topological ordering unchanged. Session change reporting combines the old and new invalidation graphs, so removed or reassigned associations invalidate both sets of owners.

## Stock and section data

Stock definitions link to a manufacturing Material. Panel specifications store thickness, sheet dimensions and grain direction. Sheet specifications add an authored inside bend radius and K factor. Profile specifications store a designation and outer dimensions, with an optional wall thickness. Print specifications store a process, optional nozzle size and layer height.

Stock distances use millimeters. Thicknesses, stock lengths and outer dimensions must be positive. Bend radii can be zero, and K factors range from zero through one. A Part cannot assign stock belonging to a different manufacturing Material through the shared assignment command. Metadata alone does not establish that the stock matches the current solid.

`HatchStyle` stores a none, lines or cross pattern, counterclockwise angle in degrees, spacing and line width. Distances are in millimeters on the drawing page. Drawing consumers can resolve a Part's section style without choosing patterns according to its workspace. Associative section rendering belongs to the drawing-view provider.

## Transactions and tags

[`commands.rs`](../../crates/multicad-core/src/materials/commands.rs) provides library upserts, multi-Part property assignments and tag edits. Preparing a command leaves canonical source unchanged. The caller commits the original Session's prepared result to create one attributable Undo step. Revision and foreign-preview checks remain the Session's responsibility.

Property assignments preserve feature history, tags and containment. Tag commands explicitly add, remove or replace tags on selected Parts. The native Inspector applies properties and tags through separate commands, so each action has its own Undo step. A failed command changes no Part in the selection.

Tags use exact, case-sensitive matching. The shared API requires trimmed tokens without control characters, with a maximum of 128 UTF-8 bytes per token and 256 tags per edit/resulting Part. The Inspector edits one tag per line, preserving commas inside a tag.

`TagQuery` combines `all`, `any` and `none` sets. Every `all` tag must match, a nonempty `any` set requires one match, and every `none` tag must be absent. Empty sets impose no restriction. Results contain canonical Part IDs regardless of their names or display visibility; export providers must separately validate current physical outputs.

## Verification boundaries

The [core material tests](../../crates/multicad-core/tests/materials.rs) cover linked persistence, inheritance, override independence, changed definitions, stock families and tag queries. They also cover atomic failure, revision checks, foreign previews and Undo. The native [library](../../crates/multicad-ui/src/materials/library.rs) and [Part Inspector](../../crates/multicad-ui/src/materials/part.rs) tests click actual Apply buttons.

The [scene publication test](../../crates/multicad-app/src/modeling/tests.rs) saves and reopens an assigned native solid, checks its exact volume and verifies its resolved color and hatch. Removing its appearance leaves exact geometry compilable while scene publication reports the broken assignment. These tests do not establish manufacturing suitability or replace interactive acceptance on each operating system.
