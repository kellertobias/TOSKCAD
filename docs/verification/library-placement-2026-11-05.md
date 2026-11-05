# Library placement and tree assemblies

CAD-242 adds direct Place actions to Library and exposes Create Assembly in the left model tree. A new Library pin and its occurrence enter the canonical document together in one undoable transaction. Cancel leaves both absent. Existing pins remain usable without the original files, and changing a pin still requires an explicit update.

The implementation is committed on `main`: `2261037` provides the shared core/MCP transaction, `28e7f59` adds graphical placement and tree/command routing, and `090c2b3` keeps Library actions above detailed metadata. The final local macOS runtime has SHA256 `2c495c63f9cec6fe37227590b7e6efb9f4eae45eae3d67b6be1f0fe091ae6347`. This verification includes no installation, push or publication.

## Canonical behavior

Only the visible outer occurrence joins the chosen parent. Embedded definitions, private wrappers and generated members retain their own ownership. The parent is captured when the controls open and revalidated on preparation. Exact pins reuse the embedded definition identity; another revision is refused until explicitly compared and updated.

Placement controls retain their allocated identities through exposed-parameter and pose edits. Viewport handles move cached display geometry during a gesture. That presentation carries no native proof. Release requests a fresh native preview, and Apply remains a separate action. Session/resource guards reject stale candidates; a Library request token also prevents an asynchronous file reply from opening placement after Cancel and browser reopening.

The public `place_library_symbol` MCP tool uses the same transaction, scoped file selection, native preflight and final owner/review checks. Compatibility command names open the shared Library or tree Inspector. The old Place/Create Assembly specialist ribbon entries are removed; existing Architecture/Wood Library navigation remains.

## Native macOS evidence

Artifacts are under `artifacts/library-placement-review-20261105/`. There are 49 screenshot/accessibility pairs. Captures 01–16 use the initial `28e7f59` runtime; 17–49 use the final `090c2b3` runtime. The initial binary and its digest remain alongside the final bundle.

| Workflow | Observed result |
| --- | --- |
| Common tree assembly | Cancel retains revision 0; Apply creates the receiving assembly. Activating it supplies explicit parent context. |
| Mechanical tree assembly | Cancel retains the reopened source; Apply creates a child under the saved active assembly. |
| Wood Library placement | Direct M3 screw placement reaches native preview. A 31 mm underhead override, numeric pose and an actual X-handle drag retain their inputs. Release and Cancel leave revision 2 unchanged. |
| Mechanical Library placement | Place is visible above the screw's long metadata. Apply inserts one occurrence under the receiving assembly; one toolbar Undo removes it and one toolbar Redo restores it. |
| Multi-definition file | Loading presents both definitions without importing either. Choosing the nested pair imports only that definition and places one occurrence containing two native leaves. Toolbar Undo/Redo acts on that single transaction. |
| Missing source | Refreshing a temporarily unavailable QA Library file shows the path, recovery guidance and disabled Place/Load actions. Restoration is byte-identical and refresh re-enables the actions. |
| Offline cold reopen | The saved project opens with both original screw/assembly Library files temporarily unavailable and retains its geometry. Both original files are restored byte-identically. |
| Compact embedded placement | At 1100×700 logical pixels, an embedded pair can open a native placement preview and Cancel without changing revision 0. |

Independent checked-file readback and a full native rebuild establish:

- `native-after-cancel.tcd` has the empty receiving assembly, zero embedded definitions, zero symbol instances and zero physical leaves.
- `native-library-m3-placed.tcd` has one definition and occurrence. It retains the sourced M3 pin, the 31 mm instance override, X = 24.495198892824824 mm, 30° Z orientation and the receiving parent. Its native leaf volume is 263.89475052866936 mm³.
- `native-library-assembly-placed.tcd` has two definitions and occurrences, with three physical leaves. The pair retains translation [80, 15, 0] mm and 45° Z orientation; its two leaves total 1576 mm³. The unselected adjustable-box definition is absent. Private identities retain private ownership.
- Repeating readback after cold reopen and compact Cancel gives identical container/source hashes, pins, IDs, overrides, parents, placements and native leaf receipts. Files remain unchanged by inspection.

Summed leaf volume counts physical occurrences; it is not a Boolean union or clearance certificate. The readback sources, compiled-tool reproduction script, JSON receipts and temporary-source restoration receipts are retained. All completed QA windows are closed; the final running-window inventory is empty.

## Checks and limits

Focused checks pass: three core transaction tests, eleven MCP symbol tests, nine application file-completion tests, the routing suites, and final symbol/card-placement checks. The final full run includes the complete UI unit suite: 985 passed, zero failed and two intentional ignores, in 64.50 seconds. Formatting, strict locked all-target workspace Clippy and the final release build pass.

The final `cargo test --workspace --locked` run on frozen source `090c2b3` exits **0: 2625 passed, zero failed, four intentional ignores across 297 groups**. It uses the pinned toolchain and committed lockfile, optimized test code, enabled debug assertions and enabled overflow checks. Logs, source identity, timestamps and process receipts are in `checks/`.

Toolbar history passes. Immediate keyboard Undo/Redo after edited placement did not change the document, and the actual compact Inspector still clips the right end of some controls/parent text despite an isolated wrapping assertion. Both observations are recorded on CAD-209 for the final UI pass; their causes are unconfirmed. Captures 28/29 document the keyboard no-op, while 30/31 document the successful toolbar history. Capture 43 only shows a filter with no matching loaded definition and is not incompatible-content proof.

This round does not establish 1024×600 layout, universal keyboard behavior, OS input-to-paint latency, Windows/Linux interaction, certified hardware fit/manufacture, or human acceptance.
