# Project references (CAD-185)

A project can place a pinned reference to another native `.tcd` project, for
example the shipped welded workbench in the apartment's workshop. A reference
is an ordinary pinned symbol ([ADR-0024](0024-pinned-symbol-variants.md))
whose content is the whole physical model of the referenced project, plus a
record of where that content came from.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/symbols/project.rs` | Source record, authoring, place/update transactions, path rules, state classification |
| `crates/multicad-core/src/symbols/remap.rs` | Provider allowlist, now including weldment and cabinetry geometry |
| `crates/multicad-model/src/project_references.rs` | Opening the referenced file, native pinning, search and state checks, installed example folders |
| `crates/multicad-model/tests/project_references.rs` | Placement, save/reopen, copy elsewhere, missing, changed and foreign sources |

## What is pinned

`symbols::project_definition` turns the referenced project into one static
`symbols.definition`:

- **Content.** Every physical occurrence of the source: its Parts that are not
  definition-only members of a reusable Assembly and that no unsuppressed
  Boolean consumes (the native compiler supplies that set), and its top-level
  Instances. A synthesized root Assembly, named after the source document,
  holds one identity-placed Instance per such Part plus every top-level
  Instance. The template is the dependency closure of that root, as for any
  authored symbol: features, parameters, stock, materials and Part
  properties.
- **Not pinned.** Drawings, export sets, file information, the parameter
  table and weld, connection and sequence records stay in the source file.
  They describe how the source is documented and fabricated, and weld marks
  must stay unique in a project that places the bench twice.
- **Identity.** The symbol identity and the synthesized root, ownership and
  occurrence identities derive from the source identities (SHA-256, UUID
  version 8). The same source always yields the same definition, and a later
  version of it keeps every placement's private identity map.
- **Revision.** `source <first 12 hex digits of the template hash>`. Different
  content always gets a different revision.
- **Evaluation.** `project_references::reference_content` compiles the source
  natively before pinning; a source that does not evaluate cannot be
  referenced.

### Provider allowlist

Symbol remapping only rewrites identities in schemas it has audited. CAD-185
adds `weldment.member`, `cabinetry.panel`, `cabinetry.machining`,
`cabinetry.machining_cut`, `cabinetry.hardware`, `cabinetry.joint`,
`cabinetry.joint_cut`, `cabinetry.loose_tenon` and the `materials.*` library
records. Each is first decoded by its typed schema, which rejects unknown
fields, so only complete UUID strings and slash paths are rewritten. Other
providers, including walls, sheet metal, imports and advanced features, are
still refused with the provider name; nothing opaque is copied.

## The source record

A `symbols.project_source` extension (schema 1, capability
`symbols.project_source.1`) sits next to the embedded definition:

| Field | Use |
| --- | --- |
| `definition` | The embedded definition (canonical input link `definition`) |
| `document` | Canonical identity of the referenced project. Must equal the definition template's identity |
| `path` | Relative path of the referenced file from the referencing file's folder, with `/` separators |
| `source` | SHA-256 of the referenced canonical source when pinned (informational) |

A definition has at most one source record. Paths are refused when they are
absolute, start with `/`, contain a drive colon, a backslash, an empty or `.`
segment, or do not end in `.tcd` or `.multicad`. `..` segments are allowed.
An unsaved project, or a source on another drive, records the bare file name.
No absolute developer path is ever written.

## Placing and updating

- **Place.** `prepare_project_reference` (or
  `project_references::prepare_reference_file` from a path) embeds the
  definition and source record if needed, materializes one private copy and
  its outer Instance, all in one undoable transaction. A second placement of
  the same content reuses the definition and record. Placing a different
  revision of an already referenced project is refused: update the existing
  reference first.
- **Move.** The outer Instance moves with the existing placement tools.
- **Update or relocate.** `prepare_project_update` (or
  `prepare_reference_refresh` from a path) takes the source file again. Equal
  content only rewrites the source record (a relocation). Changed content goes
  through the ordinary symbol update: every placement keeps its outer
  Instance, pose and the private identities of unchanged entities, and Undo
  restores the previous pin and geometry together. A file holding another
  project is refused.

## Finding the source again

Opening and evaluating a project never reads a referenced file: the pin is
the content. `project_references::check_references` reports each reference's
state without changing the document. It looks, in order, at:

1. the stored relative path from the referencing file's folder;
2. the same file name next to the referencing file;
3. the file name in each bundled directory.

The first candidate holding the same project decides. Otherwise the first
existing candidate is reported as another project or unreadable.

| State | Meaning |
| --- | --- |
| Current | Found, and its pinned content equals the reference |
| Updated | Found, same project, different content. The pinned revision stays in use until an explicit update |
| Different | A file at a candidate path holds another project. It is never adopted |
| Missing | No candidate exists. The pinned revision stays in use |
| Unreadable | A candidate could not be read or does not evaluate |

`ProjectSourceCheck::summary` gives one line naming the reference, its
revision, the state and the file.

### Bundled examples

`bundled_example_dirs` returns the existing folders among the
`MULTICAD_EXAMPLES_DIR` override, `MultiCAD.app/Contents/Resources/examples`
(macOS) and `resources/examples` next to the executable (Windows and Linux
archive layout), the same layout the Examples gallery
(`multicad_ui::examples::candidates`, CAD-186) packages. A user's copy of the
apartment saved in another folder therefore still resolves the workbench from
the installation.

The gallery copies an example together with the files it references:
`referenced_files` returns a project's stored reference paths and is
installed as the gallery's `DependencyResolver` in
`bundled_examples::library()` and the packaged self-test. A copy in
`<Documents>/ToskCAD Examples/` thus finds `welded-workbench.tcd` beside it.

## Limitations

- Save As does not rewrite stored paths; a copy elsewhere falls back to the
  file name next to it and the bundled folders.
- References are static: the source's parameters cannot be overridden per
  placement. Edit the source and update the reference.
- There is no window command or MCP tool for placing, checking or updating
  references yet. The window shows a placed reference as a symbol instance
  with its name and revision.
- Every placement embeds its own private copy of the source graph, as for any
  symbol.
- Evidence is from native tests on macOS. Interactive use in the window and
  Windows/Linux runtime are not verified.
