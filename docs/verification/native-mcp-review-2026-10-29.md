# MCP review and control activity verification, 2026-10-29

CAD-96 adds pending review of external changes and an append-only control
audit. The [MCP server note](../architecture/mcp-server.md) describes the
policy, tools and window sections.

## Commit

| Commit | Step |
| --- | --- |
| `5cfdf2a` | Core `ControlLedger`, MCP review routing, `review_status` and `activity` tools, window review panel and native viewport preview |

## Evidence

- **Core ledger.**
  [`core/tests/control_ledger.rs`](../../crates/multicad-core/tests/control_ledger.rs)
  checks the following.
  - Queued renames leave revision 0 untouched.
  - Accepting one applies it as revision 1, attributed to `mcp:reviewer-test`
    with its label.
  - A competing change prepared at revision 0 becomes `stale` and is never
    rebased, and a closed review cannot be accepted again.
  - A rejected change never applies.
  - A window Undo is imported once, in order. A queued Redo applies as the
    client.
  - The audit holds all nine outcomes in order, with consecutive sequence
    numbers, and none was removed when history moved backwards.
  - Stale expiry works, the queue is limited to 64 changes, and the audit
    drops beyond 10,000 entries while counting the drops.
- **Tool layer with a reviewed owner.**
  [`mcp/tests/tools.rs`](../../crates/multicad-mcp/tests/tools.rs) checks the
  following.
  - `set_parameter` returns `pending_review` at revision 0 with the document
    unchanged.
  - `review_status` reports `pending`, not stale. After Accept it reports
    `applied` at revision 1, with the client as actor.
  - The competing request is stale and its parameter never appears.
  - A stale request fails and is audited.
  - A reviewed Undo applies as the client.
  - `activity` lists inspected, previewed, applied, stale, failed and undone
    in order. The applied entry carries the tool, arguments, base revision 0,
    revision 1, the actor and the affected IDs.
- **Real window over the local socket.**
  [`app_control.rs`](../../crates/multicad-app/src/app_control.rs) uses the
  native model backend.
  - The first `create_sketch` is reviewed and changes nothing until
    `accept_review`.
  - A trusted call applies directly.
  - A reviewed extrusion stays at revision 2. **Preview in viewport**
    evaluates the candidate natively: the preview scene's instance measures
    30 mm wide and 10 mm high, and the Model viewport's review scene is shown
    while the document stays at revision 2.
  - Accept commits revision 3 as `mcp:window-test` and closes the preview.
  - A change queued before a window Undo becomes stale, and a rejected change
    reports `rejected`.
  - The client's `activity` reads in order: previewed, applied, applied,
    previewed, applied, previewed, then a window Undo by `local-user`, then
    stale, previewed, rejected, inspected, inspected.
  - The existing window/headless equivalence test now trusts its client
    explicitly.
- **Window panel.** A headless render of Command and MCP history checks the
  following.
  - Connections (with **Apply without review**), Pending review and Activity
    appear in that order.
  - A queued change shows its review number, client, label, base revision,
    and Accept, Reject and Preview in viewport.
  - The activity lists the preview. Accept applies it.
  - The window contains no chat or prompt text.
- **Binding and drafts.** Existing tests still hold: a connection stays on
  its project, and window drafts block external commits with `busy`.

## Verification commands

All on the committed tree at `5cfdf2a`, Rust 1.99.1, locked and offline:

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline --no-fail-fast
npm test
CARGO_INCREMENTAL=0 ./build
```

- Formatting and strict Clippy pass.
- Tests: 1,357 passed, 0 failed, 2 existing ignored.
- Release contracts: 25/25.
- The native build produced
  `artifacts/development/build-IvCiUn/MultiCAD.app`, executable SHA-256
  `3ec6e6b1dfb5b8d2607353c51bd55f4cb9cc27d2bf93d5cd4fa098893c5b2408`.
- Logs are in `/private/tmp/multicad-cad96/`.

## Limitations

- **One transaction per change.** Each reviewed call is its own transaction.
  Several calls cannot be grouped into one reviewed transaction.
- **No Undo/Redo preview.** Undo and Redo have no viewport preview.
- **Session-only trust.** Trust lasts for the window session and is not
  saved.
- **Audit not persisted.** The audit lives in memory with the open project.
  It is not written to the project file.
- **No interactive macOS check.** Previews, Accept and Reject were driven
  through the application API and headless rendering, not by hand. Windows
  has no window control.
