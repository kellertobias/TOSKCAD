# File metadata and the installation library (CAD-141)

## File metadata

A project file has one authored identity: **name**, **description** and an optional **preview image**.

- The name is the canonical document name (`Mutation::RenameDocument`), which the native file also records in its checked manifest identity.
- Description, preview and the **capture-on-save** setting live in one optional `core.file_info` extension entity (schema 1) in the canonical source.
  - The preview is a PNG stored as canonical base64, at most 1 MiB and 2048×2048 px.
  - Core checks the PNG signature, the IHDR dimensions and the size limits. The UI decodes the pixels for display.
  - Capture on save defaults to on and is written only when it is off.
- Edits go through `project_file::info::edits`, which returns typed mutations for the shared transaction API. The UI and any other client therefore get the same validation, Undo/Redo, dirty state and save path. Default information (empty description, no preview, capture on) removes the record.
- **Versioning:** the record is optional, so it requires no core capability. Older readers open the file normally and keep the extension entity unchanged. A record with a newer schema is kept as opaque source; this build shows it read-only and refuses to edit it.
- `project_file::card` reads the manifest and the checked canonical source, but no payloads or caches. It returns the name, description, preview and symbol-definition count. The home screen, the Examples gallery ([demo projects](demo-projects.md#bundling-and-the-examples-gallery-cad-186)) and the library all read files through it, so they show the same values as the file itself. No separate home-screen metadata exists.

## Preview capture on save

Each Save or Save As captures the visible 3D view and stores it as the file's preview, so the saved file, the home screen and the library show it.

1. The scene painter (`viewport/gpu.rs`) records the rect of the 3D view each time it draws (`preview_capture::record_viewport`).
2. When the tab's capture setting is on, the mode is Sketch or Model, and a 3D view was drawn in the latest frame, Save first sends `ViewportCommand::Screenshot` to the window. Other file operations wait meanwhile, because `file_busy` is true.
3. When the screenshot arrives, it is cropped to the viewport rect (in physical pixels), downscaled by area averaging to fit 640×480 and encoded as an RGB PNG.
4. The image is stored with `Session::replace_file_preview`, then the ordinary save job runs with this snapshot. The save receipt sets the saved state, so the tab is clean afterwards.

Details:

- **No Undo entry:** the preview counts as presentation state. `replace_file_preview` writes it into the current source and into every Undo/Redo snapshot. Undo and Redo therefore never bring back an older picture, never dirty the file and need no extra history step. The revision advances so open previews and editors revalidate. The activity log records "Capture file preview".
- **No visible 3D view** (Parameters, Drawings or Export mode, or a headless or MCP save with no window): the previous preview is kept and the save receipt is the ordinary one.
- **Capture failures never block saving.** If no screenshot arrives within 120 frames, the crop is off-screen, or encoding fails, the save still runs and the receipt reports "preview not updated (…)".
- **Manual override:** *Choose image…* is still available. It sets the chosen PNG as the draft preview and turns *Capture the 3D view on save* off, so that image is kept. The checkbox turns capture back on. Both are part of the Parameters draft (Apply/Rebuild).
- **Tests run without a GPU:** they inject the captured image through `finish_capture`, and the crop, downscale and encode helpers work on a plain `ColorImage`.

## Parameters sidebar

Parameters keeps its full parameter table and the Apply/Rebuild and Discard ribbon. A fixed 300 px right sidebar has two sections:

- **File:** name, description, the preview, *Choose image…*, *Remove image* and *Capture the 3D view on save*. These fields are part of the Parameters draft. Apply/Rebuild commits them together with the table as one undoable transaction. Discard reverts them, and unapplied drafts block Save and Undo just like table edits.
- **Library:** *Add to Library*, then the listing status:
  - *Listed in the library · <path>* when the file is listed here;
  - *Listed from another location: <path>* when the same document is listed at a different path;
  - *Not listed in the library* otherwise.

  Below that come the latest result (green on success, red on failure, also shown in the status bar) and all library entries.

This revises the earlier rule that Parameters has no sidebar, for this metadata sidebar only. No modeling inspectors or parts sidebars appear in Parameters.

## Installation library: listing original files

The user's decision of 2026-10-30: Add to Library marks the file as listed in the library, and the library lists the original file's location. Nothing is copied.

- **One library per user installation.** The index lives at `<per-user app data>/MultiCAD/library/library.json`, next to recovery data: `~/Library/Application Support` on macOS, `%LOCALAPPDATA%` on Windows, `$XDG_STATE_HOME` or `~/.local/state` on Linux. It is never inside the application bundle.
  - A persistent `.library.json.lock` is shared by all application instances, and the index is replaced atomically (temporary file, fsync, rename, directory fsync).
  - The UI offers no library selection. `LibraryFileStore::new(root)` accepts a different root for tests.
- **Paths are kept as listed.** Entries and recent projects that point at legacy `.multicad` files keep opening from that path; neither list rewrites or renames files when `.tcd` became the native extension (CAD-179).
- **An entry** (index version 2) holds the entry id, the document id, the absolute canonical path, the last known name and description, and the time it was first listed.
- **"Marked as listed" means the index entry plus the sidebar status.** Nothing is written into the project file: no marker, no copy. A marker would change and dirty the user's file.
- **Explicit only:** listing happens only through *Add to Library*. Save never lists a file. Listing never edits open documents, so consuming projects keep their embedded pinned definitions (CAD-70).
- **Requires a saved, clean file:** the tab needs a path and a file stamp, no unsaved or unapplied edits, and no running file operation. Otherwise the button is disabled and shows the reason. The store re-reads the file and checks the manifest hash and length against the stamp. It then runs full native validation, rejects recovery snapshots and checks the document identity.
- **Duplicates:** an entry is identified by document id.
  - Adding the same file again gives *already listed*: a visible message and the listing is unchanged.
  - The same document from another path (a moved file or a Save As copy) gives *relocated*. The one entry now points at the new path, and the message names both paths.
  - A listing for a different document at the same path is dropped, because that file was replaced by this project.
- **Browsing reads each listed file live** through `project_file::card`. Edits saved to the original show up without adding it again. The library reloads after a listed file is saved. An entry is never dropped silently. It keeps its last known path, name and description, and shows an actionable problem:
  - missing (moved or deleted): open it from its new location and Add to Library, or remove the entry;
  - a different project now at that path: re-add the original, or remove the entry;
  - unreadable: fix or re-save the file, or remove the entry.

  *Remove from library* removes only the listing.
- **Use symbols** loads an entry's symbol definitions from the original file through the existing CAD-70 explicit import and Compare update workflow. Nothing updates placed instances automatically.
- **Migration:** a version 1 index written by the earlier development build (managed copies) is read by taking each entry's recorded `source_path` as its listed location. Entries without one are skipped. Old copies in `entries/` are ignored and left untouched; the index is rewritten as version 2 on the next change.
- **Failures** are reported as actionable messages: missing source, changed on disk, an unreadable or oversized image, a damaged index, and storage write or lock failures.

## Limitations

- The library can be browsed only in the Parameters sidebar, with no search. Moved files are not discovered automatically. There is no synchronization or cloud sharing.
- The capture uses the window screenshot of the drawn 3D view, so overlays drawn inside the viewport rect are included.
- No dedicated MCP tool edits file information. Clients can use the generic typed document mutations.
- Evidence comes from native unit and app tests with fake and real temporary-directory stores, plus an injected screenshot. Interactive macOS review, real GPU screenshot capture and Windows/Linux runtime are not verified.
