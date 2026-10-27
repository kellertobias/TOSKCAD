# Native project files

The native application opens and saves `.multicad` containers. Editable source,
critical provider payloads and pinned symbol definitions survive together. A
loaded display cache requires explicit provider validation before reuse.

The container codec is owned by
[`project_file`](../../crates/multicad-core/src/project_file/mod.rs). This reference
covers its envelope and resource records. The independently versioned canonical
source schema belongs to [`document`](../../crates/multicad-core/src/document/mod.rs)
and the [document contract](0004-canonical-document.md).

## File envelope

Source: [`codec.rs`](../../crates/multicad-core/src/project_file/codec.rs).

| Offset | Length | Encoding | Meaning |
| --- | --- | --- | --- |
| 0 | 8 bytes | `MCADPRJ` followed by zero | Signature |
| 8 | 4 bytes | Unsigned little-endian integer | Format version, currently 2 |
| 12 | 8 bytes | Unsigned little-endian integer | UTF-8 manifest length |
| 20 | 32 bytes | Raw SHA-256 digest | Hash of the exact manifest bytes |
| 52 | Manifest length | UTF-8 JSON | Manifest |
| After manifest | Declared source length | UTF-8 JSON | Canonical document in format 2 |
| After source | Declared chunk lengths | Unencoded bytes | Critical payloads, then optional caches, in manifest array order |

There is no compression, pathname extraction or trailing data. The writer orders
resource records by UUID. Readers follow array order. Format 1 embeds a `document`
in the manifest and omits the separate source chunk. It accepts optional
`required_capabilities`, defaulting to an empty set, and otherwise uses the same
payload, cache, recovery and extension fields. Opening format 1 records a migration
diagnostic; the next explicit Save writes format 2. This compatibility grammar
does not imply a previously released native application.

Default limits are 256 MiB per file, 4 MiB per manifest, 16 MiB for canonical
source, 64 MiB per resource chunk, and 4096 resource records including caches.
Checked arithmetic rejects overflow and unsupported platform sizes before reading
chunks. Callers may supply different limits to the core codec.

## Record conventions

UUIDs are JSON strings. SHA-256 values in JSON are arrays of exactly 32 unsigned
byte integers, each 0-255. Human-readable reports use hexadecimal strings instead.
Unsigned 64-bit fields are JSON integer tokens; consumers must preserve their full
integer precision. All byte limits count UTF-8 bytes.

Fields described as extensions are flattened into their containing object. They
are preserved, including JSON null values, and cannot shadow reserved field names.
Optional nullable fields accept omission or null as absence. The writer emits
null for `owner`, `recovery` and `original_path`, and omits absent `symbol_pin` and
`slot`. Other required fields cannot be null.

### Manifest

Source: `Manifest` in [`codec.rs`](../../crates/multicad-core/src/project_file/codec.rs).

| Field | Wire type | Required | Default / use |
| --- | --- | --- | --- |
| `identity` | Source identity object | Yes | Must match decoded document identity |
| `source` | Chunk object | Yes in format 2 | Exact source length and hash |
| `required_capabilities` | Array of strings | Yes in format 2 | At most 128 nonempty names, each at most 256 bytes; unsupported requirements reject opening |
| `payloads` | Array of payload records | No | Empty; critical bytes required to preserve source |
| `caches` | Array of cache records | No | Empty; advisory derived results |
| `recovery` | Recovery object or null | No | Absent; marks a separate recoverable snapshot |
| Other fields | Arbitrary JSON | No | Preserved manifest extensions |

Source identity has required fields `id` (UUID string) and `name` (string).
Both must equal the canonical document values after decoding. `core.document.1`
is required for current source. An offset-plane anchor additionally derives
`core.plane.offset.1`, even if a malformed manifest omits it.

### Chunk, payload record and cache record

Source: `Chunk`, `PayloadRecord` and `CacheRecord` in
[`codec.rs`](../../crates/multicad-core/src/project_file/codec.rs).

| Record | Field | Wire type | Required | Use |
| --- | --- | --- | --- | --- |
| Chunk | `length` | Unsigned 64-bit integer | Yes | Number of following bytes, subject to limits |
| Chunk | `sha256` | 32-byte JSON array | Yes | Hash of those exact bytes |
| Payload record | `descriptor` | Payload descriptor object | Yes | Identity and provider metadata |
| Payload record | `chunk` | Chunk object | Yes | Critical payload bytes |
| Cache record | `descriptor` | Payload descriptor object | Yes | Identity and provider metadata |
| Cache record | `key` | Cache key object | Yes | Validation requirements |
| Cache record | `chunk` | Chunk object | Yes | Advisory cache bytes |

Payload and cache IDs must be unique across both arrays. A critical checksum or
descriptor failure rejects the project. An optional cache with a failed checksum, or with decoded descriptor/key values
that fail validation, is discarded with a diagnostic. Malformed manifest syntax
or missing required fields rejects the container. A truncated file, duplicate identity, unsupported format or
capability, identity mismatch, invalid source, or unexpected trailing byte rejects
opening. Existing UI sessions remain unchanged on those failures.

### Payload descriptor and symbol pin

Source: `PayloadDescriptor`, `SymbolPin` and `Payload::validate` in
[`model.rs`](../../crates/multicad-core/src/project_file/model.rs).

| Field | Wire type | Required | Default / use |
| --- | --- | --- | --- |
| `id` | UUID string | Yes | Non-nil resource identity |
| `kind` | String | Yes | Nonempty provider role, at most 256 bytes |
| `schema_version` | Unsigned 32-bit integer | Yes | Positive provider schema version |
| `media_type` | String | Yes | Nonempty content type, at most 256 bytes |
| `owner` | UUID string or null | No | Absent; if present, non-nil source owner identity |
| `symbol_pin` | Symbol pin object or null | No | Absent; checked embedded symbol identity |
| Other fields | Arbitrary JSON | No | Preserved descriptor extensions |

A symbol pin contains required `symbol` (non-nil UUID), `revision` (nonempty
string, at most 1024 bytes), and `content` (32-byte hash array). Its descriptor
kind must be `symbol.definition`; its content hash must match the payload bytes.
`Payload::pinned_symbol` writes an embedded checked project with media type
`application/vnd.multicad.project`. Nested projects are decoded only when a
provider explicitly requests them, avoiding recursive eager expansion on Open.

### Cache key

Source: `CacheKey`, `DerivedCache::validated` and `ProjectFile::cache_key` in
[`model.rs`](../../crates/multicad-core/src/project_file/model.rs).

| Field | Wire type | Required | Default / use |
| --- | --- | --- | --- |
| `source` | 32-byte JSON array | Yes | Canonical source hash |
| `producer` | String | Yes | Nonempty producer identity, at most 256 bytes |
| `producer_version` | String | Yes | Nonempty implementation version, at most 1024 bytes |
| `schema_version` | Unsigned 32-bit integer | Yes | Positive cache schema version |
| `dependencies` | Object mapping names to 32-byte arrays | Yes | At most 1024 entries; names at most 256 bytes |
| Other fields | Arbitrary JSON | No | Preserved key extensions; compared exactly |

Opening reports that a cache needs provider validation, or that its source has
changed. Raw cache bytes are private. A provider computes the expected current key
and calls `validated`; source, producer, schema or dependency mismatches return a
rebuild reason. The project helper includes all critical payload hashes, required
capabilities and core interpretation semantics. Providers must add kernel policy,
tessellation settings and other external dependencies they actually use. Storage
does not implement unavailable geometry, drawing or symbol providers.

### Recovery

Source: `RecoveryInfo` in [`model.rs`](../../crates/multicad-core/src/project_file/model.rs).

| Field | Wire type | Required | Default / use |
| --- | --- | --- | --- |
| `id` | UUID string | Yes | Non-nil snapshot identity and filename key |
| `slot` | UUID string or null | No | Absent for manual snapshots; if present must equal `id`, marking an automatic session/tab slot |
| `created_unix_ms` | Unsigned 64-bit integer | Yes | Snapshot timestamp in milliseconds since Unix epoch |
| `source_revision` | Unsigned 64-bit integer | Yes | Session revision captured by the job |
| `original_path` | String or null | No | Absent; informational source path, never an automatic replacement destination |

## Native file operations

[`ProjectStore`](../../crates/multicad-app/src/files/native.rs) owns file I/O.
[`FileWorker`](../../crates/multicad-ui/src/storage.rs) runs bounded background
jobs. Their receipts carry request identities; the UI binds a save to a local tab
and acknowledges the exact immutable source snapshot written. Subsequent edits or
Undo remain dirty relative to that disk snapshot. Resource bytes use shared
immutable ownership across these snapshots.

Save creates a temporary file in the destination directory, writes and flushes it,
synchronizes the file, then publishes through replacement. A persistent advisory
lock serializes cooperating MultiCAD writers. Expected destination stamps are
checked before serialization and immediately before publication. The returned
published file handle supplies the save receipt. Save As requires confirmation
before replacing an existing native project; a stale destination is rejected.
Arbitrary noncooperating programs cannot receive portable atomic compare-and-swap
guarantees. Unix directory-sync failures are reported as warnings after a successful
publication. Windows directory durability and platform runtime behavior require
separate validation.

Open accepts native projects. Import source snapshot explicitly accepts the legacy
canonical JSON format and creates an unsaved native document. JSON acceptance
fixtures use `SnapshotStore` separately; production Save never falls back to it.

Automatic recovery debounces canonical changes for 20 seconds. Draft parameter or
sketch previews are excluded. A random local-tab slot is updated atomically,
preserving other sessions and manual checkpoints. Slot reuse verifies document,
recovery and slot identities. Manual checkpoints allocate separate files. No
recovery files are deleted automatically.

Recovery files live in `MultiCAD/recovery` under macOS Application Support, Windows
Local AppData, or Linux XDG state home (falling back to `~/.local/state`). Discovery
scans checked manifests and retains the newest 512 entries by recorded timestamp;
other snapshots remain on disk. Missing platform state environment variables
fall back to the system temporary directory. Summary discovery does not validate payload bytes.
Restore performs a complete checked read, rejects a slot changed since its displayed
metadata, and opens an unsaved copy without a destination. Save As chooses its file.
Ordinary Open rejects recovery snapshots to prevent accidental replacement.

Undo and command history, widget state, camera state and native kernel handles are
not persisted as editable source. Display meshes never become manufacturing data.

## Verification sources

- [Container, checksum, capability, migration and cache tests](../../crates/multicad-core/tests/project_file.rs)
- [Filesystem failure, process interruption, external-writer and recovery tests](../../crates/multicad-app/src/files/tests.rs)
- [Async tab, resource, recovery and draft-exclusion tests](../../crates/multicad-ui/src/application/files.rs)
- [Engineering decision](0008-project-container.md)
