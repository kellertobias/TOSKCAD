# Local MCP server verification, 2026-10-29

CAD-95 adds local MCP control over the shared transaction API. The
[architecture note](../architecture/mcp-server.md) describes the transports,
scope, tool catalog, units and error codes.

## Commits

| Commit | Step |
| --- | --- |
| `80f3daa` | Kernel: opt-in routing of OCCT messages and C++ stdout to stderr |
| `c7e7d81` | `multicad-mcp` crate, tool catalog, headless stdio server, `multicad --mcp` |
| `3c79be6` | Core `SessionOwner` boundary; the window runs control jobs on its bound project |
| `77f21cf` | Same-user local socket, `multicad --mcp --app` bridge, window control switch |

## Independent protocol client

[`crates/multicad-app/tests/mcp.rs`](../../crates/multicad-app/tests/mcp.rs)
starts the real `multicad` executable. It speaks newline-delimited JSON-RPC
over the process's stdio and uses no MCP SDK. Every result's text content must
equal its `structuredContent`.

- **Discovery.** `initialize` returns the `multicad` server, tool capability
  and instructions. `tools/list` lists all 18 tools, each with an object input
  schema. Exactly the tools that require `expected_revision` are not marked
  read-only. Unknown tools and malformed arguments fail with structured codes.
- **Parameterized part.** The client creates parameters `width = 40 mm` and
  `depth = =$width/2`. It then adds a sketch on XY with a rectangle driven by
  both parameters and a fixed Ø6 circle.
  - `inspect_sketch` reports `FullyConstrained`, 0 degrees of freedom and two
    regions. The plate region's area is 800 − 9π mm².
  - Extruding it 10 mm gives a native volume of (800 − 9π)·10 mm³ and bounds of
    40 × 20 × 10 mm.
- **Edits.** Setting `width = 50` regenerates the part at (1250 − 9π)·10 mm³.
  Changing the circle's diameter dimension to 8 mm gives (1250 − 16π)·10 mm³.
- **Stale request.** A request based on revision 6 at revision 8 fails with
  `stale_revision` and `current_revision: 8`. The revision does not change.
- **Cut.** A second sketch's Ø4 circle cuts through the part, giving
  (1250 − 20π)·10 mm³.
- **Undo and Redo.** Undo restores (1250 − 16π)·10 mm³ and Redo restores the
  cut. `history` attributes both to `mcp:protocol-test` on the `mcp` channel.
- **Export.** `../plate.step` is refused as `out_of_scope`. `plate.step` is
  written into the export folder only. Its read-back volume equals the model
  volume, and no partial file remains.
- **Save and reopen.** `save_project` writes the `.multicad` file and clears
  the dirty flag. A new server process reopens it with the same document ID,
  the same part volume and `depth = 25 mm` (derived).
- **Scope.** `--read-only` refuses mutations with `read_only`. Without a
  project, saving fails. Without an export folder, `export_step` returns
  `out_of_scope`.
- **Import.** `import_sketch_artwork` takes inline SVG text into a new XZ
  sketch, creating 5 curves. It reports the unsupported `text` element and
  records the bare file name in provenance. The regions measure 25π and
  600 − 25π mm². A unitless DXF fails with `import_failed`, and the revision
  does not change.
- **Window bridge.** Without a listener, `multicad --mcp --app` exits non-zero
  with nothing on stdout. Its stderr names the window switch. With a listener,
  the bridge relays a full session; a 3 in parameter reads back as 3 in.

## Window and headless equivalence

[`app_control.rs`](../../crates/multicad-app/src/app_control.rs) tests the real
desktop `Application` with its control switch on. A JSON-RPC client connects
over the local socket while the test runs the window's UI-thread job queue.

1. **Socket permissions.** The socket directory has mode 0700 and the socket
   mode 0600.
2. **Scope.** `document_info` reports mode `application` and transport
   `local-socket`.
3. **Workflow.** The client runs a workflow on the XZ plane: parameters, a
   rectangle, an extrusion, a reversed cut, a parameter edit, Undo and Redo.
4. **Equivalence.** The same workflow runs through a headless host. Both give
   identical results: volumes, bounds, solid counts, revisions, entity counts
   and parameter values, and the same entity kinds and names.
5. **Attribution.** The window's own Session is at revision 11. Its last
   activity is attributed to `mcp:window-test` on the `mcp` channel.

More window tests:

- An unapplied parameter-row draft makes `set_parameter` fail with `busy`, and
  inspection stays available.
- A second window cannot take over the socket.
- Calls still queued when the switch is turned off are refused with "MCP
  connections were turned off".
- UI tests check the binding. A connection stays on the project active at its
  first call, even after another tab becomes active, and fails once that
  project closes.

Tool-layer tests check cancellation:

- An extrusion cancelled during native work returns `cancelled`, and the
  document is unchanged.
- A cancelled Undo commits nothing.
- Eight malformed or failing requests return their codes and leave the
  document unchanged.

## stdout integrity

The first protocol run found a real defect: OCCT's STEP transfer printed
statistics to stdout, which corrupted the JSON-RPC stream at `export_step`.
`multicad_kernel::route_native_output_to_stderr()` now replaces OCCT's default
stdout printer with a stderr printer at warning level. It also redirects C++
`std::cout`. The headless server calls it before serving, and the desktop is
unchanged.

## Verification commands

All on the committed tree at `77f21cf`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,340 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-L1Cpna/MultiCAD.app`, executable SHA-256
  `71a3d8c10e3b43fb0415df6feba7a145a2640fdbfc2a89848929eb6d2a161dc1`.
- Over stdio, the bundled executable answers `initialize` as `multicad` and
  lists 18 tools. `--diagnostics` reports
  `MCP server: local stdio (multicad --mcp), rmcp 3.8.0`.
- Logs are in `/private/tmp/multicad-cad95/`.

## Limitations

- **No interactive macOS check.** The desktop toggle has not been exercised by
  hand: switching it on, connecting a real MCP client app, and watching the
  window update. The window path is proven by the in-process `Application` test
  and the bridge protocol test. Windows and Linux runtimes are unverified, and
  window control is unavailable on Windows.
- **Tool coverage.** Tools cover parameters, rectangle and circle sketches,
  dimensions, inspection, extrude and Booleans, evaluation, Undo/Redo, history,
  save, STEP export and DXF/SVG import. Revolve, fillet and chamfer, patterns,
  assemblies, drawings, symbols, materials and other exports are not exposed
  yet.
- **No review or push updates.** Edits commit directly; CAD-96 owns pending
  review. There are no MCP resources, prompts or progress notifications.
- **Window mode files.** Window mode does not save or export files.
