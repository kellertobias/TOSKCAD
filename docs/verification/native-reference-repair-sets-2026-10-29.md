# Native reference repair sets, 2026-10-29

These checks record agent verification. They are not human acceptance.

## Source and build

The checks cover commits `b0946e9` (core and model), `bfe09d4` (MCP) and
`b28a122` (window). They used Rust 1.99.1 with the locked lockfile, offline.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 1,375 passed, 0 failed, 2 existing ignored |
| `npm test` | 25 of 25 passed |
| `./build` | Passed |

The build wrote
`artifacts/development/build-MhVY2W/MultiCAD.app`. The SHA-256 of its
executable is
`85afc33daab8a53206b6cf06eb87025a0bf615f519d0ef106eb81d5d1c34d4c1`.
Logs are in `/private/tmp/multicad-cad134/`.

## Fixture

The fixture is `crates/multicad-model/tests/support/broken_references.rs`. The
model, MCP and window tests all share it.

- **Before breaking.** Two original walls, 30 and 60 mm wide, each feed one Z5
  sketch through a finite plane intersection. Each sketch drives a 5 × 7 mm
  profile through a coincident constraint, and each profile is extruded 2 mm.
- **Breaking.** Both walls are lowered to 2 mm, so neither intersection reaches
  its sketch plane. A variant deletes both original walls instead.
- **Replacements.** Walls 40 and 50 mm wide.

## Evidence

### Model: `crates/multicad-model/tests/repair_set.rs`

- **One repair at a time fails.** The existing single-reference repair of
  reference A produces a candidate whose full model still fails to compile.
- **An incomplete set stays blocked.** Staging only row A leaves Apply
  disabled. The blocker names the unresolved reference or the failed rebuild.
- **An invalid row is reported.** A face key for an edge binding is reported on
  row 2 as "wrong kind", and the live document is unchanged.
- **The complete set applies as one transaction.**
  - Before commit: `can_apply`, two affected sketches, a complete reference
    report, and the live document unchanged.
  - Commit: one revision and one activity entry, "Repair 2 sketch references",
    attributed to the actor.
  - Regenerated geometry: both parts are 70 mm³, with minimum X at 40 and
    50 mm.
  - Compatibility: the coincident constraints remain.
- **Undo and Redo.** Undo restores the broken document exactly, and it no
  longer compiles. Redo restores the repaired document.
- **Guards.**
  - Changed resources are refused.
  - A preview committed into a foreign Session fails with `ForeignPreview`.
  - The same set after a later commit is refused with "Source changed".
- **Save and reopen.** The broken project reopens identical and still fails to
  compile. The repaired project reopens identical and regenerates the
  70 mm³ part at X 50.
- **A set can create its replacement and remove dependents.** Replacement wall
  B is created in a `source_edit` row, built with an ordinary command on a
  scratch Session. B is then rebound to it. A's coincident constraint is
  removed explicitly, then reference A is removed. All of this is one commit,
  and Undo restores the broken document.
- **Removal needs explicit dependents.** Removing A alone is refused while its
  constraint exists. Reference output geometry cannot be removed as own items,
  and a source edit cannot remove a reference record.

### MCP: `crates/multicad-mcp/tests/reference_repair.rs`

The test runs on a headless host.

- **Candidates.** `reference_candidates` lists the replacement edges.
- **Blocked request.** A one-row preview reports blockers, and the same apply
  fails with `repair_blocked` without changing the document. A tampering
  source edit is reported on its row.
- **Complete request.** The two-row preview reports `can_apply` and resolved
  references. The apply commits revision 1, and the native model regenerates
  both 70 mm³ parts.
- **Attribution.** History shows the label and `mcp:repair`. The audit records
  `apply_reference_repairs` as applied.
- **Stale and Undo.** Replaying the request fails with `stale_revision`. Undo
  restores the broken document.

### Window: `crates/multicad-ui/src/reference_repair_set_tests.rs`

The test uses the real compiler through the injected checker.

- **Listing.**
  - Deleted sources list both references as "Source geometry is unavailable".
  - Lowered walls resolve topologically, so they list only when their failed
    sketches are supplied.
- **Staging.** Duplicate staging is refused. A one-row check stays blocked and
  `apply` is refused. Staging the second row clears the earlier verdict.
- **Apply.** The two-row check passes and Apply commits one transaction. The
  model regenerates 70 mm³ at X 40 and 50. Undo and Redo are exact.
- **Discard.** Changed resources disable Apply. An unrelated commit discards
  the whole set and the check in flight, and so does a resource change.
- **Removal.** It stages the explicit dependent removal before the reference
  removal.

## Limits

- **No interactive check.** No interactive macOS workflow ran, because
  launching the newly built GUI needs the user's approval. The window evidence
  is a native unit test of the panel controller, not a click-through.
- **Occurrence references in the window.** The window stages them for removal
  only. Rebinding them needs the canonical request through
  `rebind_occurrence`: core, model and MCP.
- **Occurrence tests.** No native test exercises an occurrence row inside a
  set. Occurrence rows reuse the existing checked
  `occurrence_capture_edits` and `remove_occurrence_edits`.
- **Source-edit rows.** They are API and MCP only. The window needs replacement
  geometry that already exists.
- **Candidate selection.** In the window it is list-based, not a 3D pick.
- **No Windows or Linux interaction.**
