# Local MCP server

MultiCAD exposes local application control through the
[Model Context Protocol](https://modelcontextprotocol.io/). The
[`multicad-mcp`](../../crates/multicad-mcp/src/lib.rs) crate uses rmcp 3.8.0
and calls the same Session, solver and native model functions as the desktop
UI. It does not click the UI, keep a second modeling engine, or embed an AI
model or chat.

## Connections

| Mode | Start | Document | Transport |
| --- | --- | --- | --- |
| Headless | `multicad --mcp [--project FILE.multicad] [--export-dir DIR] [--read-only]` | The `--project` file, opened or created on first save; otherwise an unsaved document | The client launches the process and talks over stdin/stdout |
| Window | `multicad --mcp --app [--socket PATH]` | The project open in the desktop window | The bridge connects to the window's same-user local socket |

A typical client configuration:

```json
{ "mcpServers": { "multicad": {
    "command": "/Applications/MultiCAD.app/Contents/MacOS/multicad",
    "args": ["--mcp", "--project", "/Users/me/parts/bracket.multicad",
             "--export-dir", "/Users/me/parts/out"] } } }
```

Use `"args": ["--mcp", "--app"]` to control the open window instead.

Neither mode opens a network port, and remote control is not offered.

- **Window listener.** It starts only after **Allow local MCP connections** is
  switched on in **Command and MCP history**. That window shows the endpoint,
  the connected clients and the project each client controls.
- **Socket location.** The socket lives in `multicad-mcp-<uid>` under
  `$XDG_RUNTIME_DIR` or the per-user temporary directory. The directory must be
  owned by the user with mode 0700, and the socket has mode 0600.
- **Turning it off.** Switching it off closes connections and refuses calls that
  are still queued.
- **Windows.** Window control is reported as unavailable on Windows. Headless
  mode works on every platform.

## Scope

`document_info` reports the scope, and no tool can widen it.

- **Project.** `save_project` writes only the headless `--project` file. It uses
  the desktop's atomic, conflict-checked project store. A file changed by
  another writer is not replaced. In window mode, saving stays with the window.
- **Exports.** `export_step` writes only into `--export-dir`, using a bare
  `.step` or `.stp` file name. It writes to a hidden partial file, reads the file
  back, then renames it. It fails without an export folder. In window mode,
  exports are unavailable.
- **Read-only.** `--read-only` refuses every mutating tool.
- **Window binding.** A window connection controls the project that was active
  at its first call. It does not follow later tab switches. When that project
  closes, its calls fail with `unavailable`.
- **Window drafts.** While the window has an unapplied draft, such as a
  parameter row, sketch edit, page draft or export draft, commits, Undo, Redo
  and saves fail with `busy`. Inspection stays available.

## Transactions

Every mutating tool requires `expected_revision`. A request based on another
revision fails with `stale_revision` and the live `current_revision`.

Each success commits exactly one transaction. Its actor is `mcp:<client name>`
on the `mcp` channel, and Undo and Redo apply to it like any UI edit.

Sketch edits are solved before they commit. A failed solve returns
`solve_failed` with the conflicting constraints. Extrusions are also evaluated
with the native kernel before they commit, and any evaluation error is returned.

Native work runs outside the document owner and can be interrupted. A client's
`notifications/cancelled` stops it at the next check. A cancelled call never
commits; internally it reports `cancelled`, and rmcp suppresses the response.

## Tools

The tool names and input schemas below are the compatibility contract. Each
schema is generated from the Rust argument types with schemars and returned by
`tools/list`. Unknown fields are rejected.

- **Annotations.** Read-only tools carry `readOnlyHint: true`. Every tool that
  needs `expected_revision` is mutating.
- **Results.** Each result carries `structuredContent`, with the same JSON as
  text.
- **Errors.** A failure sets `isError` and carries
  `{code, message, involved?, current_revision?}`.

| Tool | Mutates | Purpose |
| --- | --- | --- |
| `document_info` | no | Name, identity, revision, dirty state, undo/redo, entity counts, units and scope |
| `list_entities` | no | IDs, names, kinds and references, optionally filtered by kind |
| `get_entity` | no | The complete canonical source record |
| `list_parameters` | no | Expression, unit, evaluated value, role and exposure |
| `set_parameter` | yes | Create a parameter, or edit the one with that name |
| `create_sketch` | yes | Empty sketch on the `xy`, `xz` or `yz` origin plane |
| `sketch_add_rectangle` | yes | Fully constrained rectangle with a fixed corner and driving width and height |
| `sketch_add_circle` | yes | Circle with a fixed center and a driving diameter |
| `set_sketch_dimension` | yes | New literal or formula for an existing driving dimension |
| `inspect_sketch` | no | Solve state, degrees of freedom, conflicts, geometry, constraints, measurements, and regions with index and area |
| `extrude` | yes | Extrude regions by a distance as `new_body`, or `join`, `cut` or `intersect` a `target_part` |
| `evaluate_model` | no | Native evaluation of every final part, plus reference diagnostics |
| `undo`, `redo` | yes | The most recent transaction on any channel |
| `history` | no | Recent activity with actor, channel, label, kind and changed IDs |
| `save_project` | yes | Save to the scoped project file |
| `export_step` | no | STEP of one final part into the export folder, with readback volume and SHA-256 |
| `import_sketch_artwork` | yes | DXF or SVG text into an existing or new sketch, with provenance (see [sketch import](sketch-import.md)) |

`evaluate_model` reports each part's producing feature, solid count, volume
and bounds.

### Values and units

- **Lengths.** A length is `{"value": 40, "unit": "mm"}` or
  `{"expression": "=$width/2", "unit": "mm"}`, with exactly one of `value` or
  `expression`. The unit is always required: `mm`, `cm`, `m`, `in` or `ft`.
- **Parameters.** A parameter takes `"40"` or `"=$width*2"` and any declared
  parameter unit.
- **Coordinates.** Sketch coordinates (`origin_mm`, `center_mm`) are
  millimeters in the sketch plane.
- **Results.** Results report millimeters, square millimeters and cubic
  millimeters, plus parameter values in their declared unit.
- **IDs.** Persistent entity IDs are UUID strings.
- **Regions.** Region indices come from `inspect_sketch` and apply only at the
  revision it reported.

### Error codes

| Code | Meaning |
| --- | --- |
| `invalid_request` | Malformed arguments, wrong unit kind, missing target, or a region index that does not exist |
| `stale_revision` | `expected_revision` is not current; `current_revision` is set |
| `stale_preview`, `invalid_document`, `invalid_parameter`, `no_history`, `revision_limit` | Session errors |
| `invalid_sketch`, `solve_failed` | The sketch cannot be read, or the edit does not solve |
| `model_failed` | Native evaluation failed; `involved` names the entity |
| `not_found` | The ID does not exist or has the wrong kind |
| `read_only`, `out_of_scope`, `busy`, `unavailable` | Scope or owner refusals |
| `save_failed`, `export_failed`, `import_failed` | File operations |
| `cancelled` | Cancelled before commit; nothing changed |
| `unknown_tool` | The tool name is not in the catalog |

## Architecture

1. **Owner boundary.** [`control.rs`](../../crates/multicad-core/src/control.rs)
   in core defines `SessionOwner` and `ControlJob`, an owner-thread job.
2. **Tool layer.** The [tools](../../crates/multicad-mcp/src/tools.rs) submit
   short jobs to read or commit. Each job runs to completion on the owner
   before the next one starts. Solving, region extraction and native
   compilation work on snapshots.
3. **Headless owner.**
   [`Headless`](../../crates/multicad-mcp/src/headless.rs) owns a Session
   behind a mutex.
4. **Window owner.** The window's
   [`external_control`](../../crates/multicad-ui/src/application/external_control.rs)
   runs jobs on the UI thread against the bound project, up to 32 jobs per
   frame.
5. **Transports.** The [local socket](../../crates/multicad-mcp/src/local_socket.rs)
   and [app control](../../crates/multicad-app/src/app_control.rs) connect the
   window. The UI crate has no protocol dependency.
6. **Clean stdout.** Before serving stdio, the server calls
   `multicad_kernel::route_native_output_to_stderr()`. That sends OCCT messages
   and C++ `std::cout` to stderr, at warning level, so native diagnostics never
   corrupt the JSON-RPC stream.

## Not yet covered

- **More tools.** There are no tools yet for revolve, fillet and chamfer,
  patterns, assemblies and joints, drawings, symbols, materials, or exports
  other than STEP. They will use the same tool layer as their shared commands
  become available.
- **Review before commit.** MCP edits commit directly. CAD-96 owns pending
  review and richer attributable history.
- **No push updates.** There are no MCP resources, prompts or progress
  notifications. Clients poll `document_info` for the revision.
