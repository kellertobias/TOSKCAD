# ADR-0008: Preserve project source and payloads in a checked native container

| | |
|---|---|
| Status | Accepted engineering decision; file workflow acceptance remains separate |
| Date | 2026-10-27 |
| Decider | Codex, using the delegated architecture authority |

## Context

A canonical document describes parameters and dependencies, while imported kernel
data and pinned symbol definitions can require additional bytes. Editing source
must preserve those bytes even when their providers are unavailable. Rebuildable
meshes and drawings also need versions and dependency identities before reuse.

The native application must distinguish opening a project from importing the
experimental document JSON format. Core plane semantics can evolve without adding
an entity variant, so the file also needs explicit capability requirements.

## Decision

Use a streaming `.multicad` container with a fixed signature and format version.
(Revised 2026-10-31: new projects use the `.tcd` extension for the same container;
`.multicad` stays openable. See [project files](project-files.md#file-names-and-associations).)
A bounded JSON manifest identifies canonical source, critical payloads and advisory
caches. SHA-256 checks protect the manifest and each content entry. Entries are
ordered byte sequences rather than extracted paths.

Keep the canonical `Document` separate from `ProjectResources`. Resource bytes use
immutable shared ownership so UI edits can preserve them without copying their
contents. Unknown provider definitions and payload metadata survive save/load.
Pinned symbols embed a checked project payload with an explicit symbol identity,
revision and content hash. Symbol authoring and library updates remain separate.

Treat every loaded cache as requiring provider validation. Providers compute an
expected key from current source, producer version, schema and dependencies. The
default key includes all critical payloads and core capabilities. A corrupt
optional cache produces a rebuild diagnostic; critical source or payload corruption
rejects the project.

Declare required core capabilities in the manifest and derive requirements from
known source semantics. An `offset_plane` anchor requires `core.plane.offset.1`.
A reader without that capability rejects the file before publishing source.
Cache dependencies also identify the core interpretation policy.

Write the current format as version 2. Support the version 1 inline-source grammar
through an explicit checked migration that preserves entity and payload identities.
The version 1 fixture defines compatibility; it does not imply a prior released
native application. Legacy raw document JSON uses an explicit import action.

Keep file-system operations in the application layer. Save uses a temporary file
in the destination directory, flushes it, and publishes it through atomic
replacement. Recovery snapshots use separate files and require an explicit restore
action. A restored snapshot opens as an unsaved project, preserving the original
file until the user chooses a destination.

## Consequences

Source edits retain unavailable-provider data, and display cache changes cannot
replace manufacturing source. Capability checks prevent silent interpretation of
new plane semantics by an older reader. Bounded streaming avoids assembling a
second copy of every payload for a save.

The container is uncompressed and requires a MultiCAD-aware reader. Large embedded
payloads can therefore increase disk usage. Checksums detect corruption rather
than authenticate an author. Providers still own native payload validation and
must include their external evaluation dependencies in cache keys.

## Alternatives considered

**ZIP archive:** familiar inspection tools and compression would improve portability.
It adds archive parsing, decompression budgets and entry-path rules. The initial
container uses ordered records and bounded streaming to keep those concerns small.

**Document JSON alone:** human-readable source would simplify inspection and diffs.
It cannot preserve required binary assets without embedding another encoding or
depending on sidecar files. It remains an explicit import format.

**Persist evaluated native state as source:** opening could avoid recomputation.
It would couple saved intent to kernel versions and cached topology. Exact source
and critical payloads remain authoritative; evaluated caches remain advisory.

## References

- [Project model and cache keys](../../crates/multicad-core/src/project_file/model.rs)
- [Bounded container codec](../../crates/multicad-core/src/project_file/codec.rs)
- [Round-trip, migration and failure tests](../../crates/multicad-core/tests/project_file.rs)
- [Canonical document boundary](0004-canonical-document.md)
