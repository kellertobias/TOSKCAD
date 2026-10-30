# File metadata and the installation library (CAD-141)

## File metadata

A project file has one authored identity: **name**, **description** and an optional **preview image**.

- The name is the canonical document name (`Mutation::RenameDocument`), which the native file also records in its checked manifest identity.
- Description and preview live in one optional `core.file_info` extension entity (schema 1) in the canonical source. The preview is a PNG stored as canonical base64, at most 1 MiB and 2048×2048 px. Core checks the PNG signature, the IHDR dimensions and the size limits. The UI decodes the pixels for display.
- Edits go through `project_file::info::edits`, which returns typed mutations for the shared transaction API. The UI and any other client therefore get the same validation, Undo/Redo, dirty state and save path. An empty description with no preview removes the record.
- **Versioning:** the record is optional, so it requires no core capability. Older readers open the file normally and keep the extension entity unchanged. A record with a newer schema is kept as opaque source; this build shows it read-only and refuses to edit it.
- `project_file::card` reads the manifest and the checked canonical source, but no payloads or caches. It returns the name, description, preview and symbol-definition count. The home screen and the library both use it, so they show the same values as the file itself. No separate home-screen metadata exists.

## Parameters sidebar

Parameters keeps its full parameter table and the Apply/Rebuild and Discard ribbon. A fixed 300 px right sidebar has two sections:

- **File:** name, description and the preview image (*Choose image…* opens the native dialog filtered to PNG; *Remove image* clears it). These fields are part of the Parameters draft. Apply/Rebuild commits them together with the table as one undoable transaction (label "Edit parameters and file information"). Discard reverts them, and unapplied drafts block Save and Undo just like table edits.
- **Library:** *Add to Library*, the latest registration result (green on success, red on failure, also shown in the status bar) and the entries of the installation library. An entry that contains symbol definitions offers *Use symbols*, which loads it through the existing CAD-70 pinned import/update workflow (Sketch › Symbols).

This revises the earlier rule that Parameters has no sidebar, for this metadata sidebar only. No modeling inspectors or parts sidebars appear in Parameters.

## Installation library

- **One library per user installation**, at `<per-user app data>/MultiCAD/library/`, next to recovery data: `~/Library/Application Support` on macOS, `%LOCALAPPDATA%` on Windows, `$XDG_STATE_HOME` or `~/.local/state` on Linux. It is never inside the application bundle. The UI offers no library selection or management. `LibraryFileStore::new(root)` accepts a different root for tests.
- **Layout:** `library.json` holds the index (application `multicad.library`, version 1). `entries/<entry-id>.multicad` holds the managed snapshots. `.library.json.lock` is a persistent lock shared by all application instances. The index and the snapshots are written atomically with temporary file, fsync, rename and a directory fsync. The snapshot is written before the index, so a failure leaves the previous state intact.
- **Explicit only:** registration runs only from *Add to Library*. Save never registers a file.
- **Requires a saved, clean file:** the tab needs a path and a file stamp, no unsaved or unapplied edits, and no running file operation. Otherwise the button is disabled and shows the reason. The store re-reads the file and checks the manifest hash and length against the stamp this window last opened or wrote. It then runs full native validation, rejects recovery snapshots and checks the document identity.
- **Managed snapshot, not a reference:** the exact saved bytes are copied into the library. The entry keeps working when the original file moves, changes or disappears. The original path is shown only for information. Library changes never propagate back to the original.
- **Duplicates:** an entry is identified by document id.
  - Registering identical bytes again gives *Unchanged*: a visible message and nothing written.
  - Different content gives *Updated*: the same entry gets the new snapshot. The message names the previous source path when the file differs, for example a Save As copy, which keeps the document id.
  - A missing or damaged snapshot is repaired by registering again.
- **Consuming projects:** registering or updating an entry never edits open documents. Projects that already placed symbols keep their embedded pinned definitions (CAD-70). Upgrades stay the explicit Compare update review.
- **Failures** are reported as actionable messages:
  - missing source: save again
  - changed on disk: reopen or use Save As
  - unreadable or oversized image: choose a PNG of at most 1 MiB
  - damaged index or snapshot: register again
  - storage failure: check that the folder is writable and has free space

## Limitations

- The preview is a user-chosen PNG file. Capturing the current 3D view and JPEG input are not implemented.
- The library can be browsed only in the Parameters sidebar. It has no removal or rename command, no search, and no automatic synchronization or cloud sharing.
- Library entries hold whole project files. Only files that contain authored symbol definitions can be placed, through the CAD-70 workflow.
- No dedicated MCP tool edits file information. Clients can use the generic typed document mutations.
- Evidence comes from native unit and app tests with fake and real temporary-directory stores. Interactive macOS review and Windows/Linux runtime are not verified.
