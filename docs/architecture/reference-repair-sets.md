# Reference repair sets

A document can have several independently broken sketch references. Repairing
them one at a time does not work. The single-reference repair must pass the
complete-model gate, and that gate fails while any other driving reference
stays broken. A repair set stages every change against an uncommitted
candidate. It commits only when the complete candidate passes, as one
transaction.

The window and external control use the same core, model and transaction APIs.

## Rows

`sketch_external::RepairSet` holds a stamp and ordered rows
(`RepairStep`). Each row is built against the cumulative candidate, so a later
row can use geometry that an earlier row created.

| Row | Effect |
| --- | --- |
| `rebind` | Binds a feature-source reference to an explicitly chosen element of a source feature's current topology. The element must have the same topology kind. The reference keeps its output IDs and the constraints that use them. |
| `rebind_occurrence` | Replaces an occurrence reference's query and semantic selector. The owning sketch, output IDs, family and pole count must stay compatible. |
| `remove` | Removes a reference and its output geometry. Every dependent constraint and curve must already be gone. |
| `remove_sketch_items` | Explicitly removes named own constraints, curves and points. It refuses reference output geometry, and it refuses to orphan a constraint or curve that it does not also name. |
| `source_edit` | Canonical insert, replace and remove edits that make a replacement possible, such as creating replacement geometry. New insertions are scoped into the hierarchy. It refuses a change to any reference record, a new sketch reference and a document rename. |

No row retargets a reference that it does not name. A family or pole-count
change is not a rebind. It is a `remove` row, preceded by `remove_sketch_items`
for its dependents, and then a fresh capture.

## Staging and the gate

`stage_repair_set` builds the candidate in five steps:

1. It checks the stamp: document, revision, document content hash, and an
   optional project-resource fingerprint.
2. It stages every row. Native data comes from the `RepairNative` adapter:
   - topology for `rebind`, from a source-only compile of the cumulative
     candidate;
   - a catalogue for `rebind_occurrence`.
3. If a row fails, it records the diagnostic and leaves the candidate
   unchanged. Later rows are still staged, so every problem appears at once.
4. Each staged row reports:
   - its old binding;
   - the replacement or removal;
   - the affected entities;
   - its error, if any.
5. It returns the cumulative mutations, the candidate document and every
   affected sketch.

`Compiler::prepare_repair_set` then decides whether Apply is possible.

- **Preparation.** It prepares one Session request from the staged mutations.
  The label is "Repair N sketch references" and the request carries the
  caller's actor. It then confirms that this request reproduces the staged
  candidate exactly.
- **Gate.** The full model must compile. The driving reference report must be
  complete: no unresolved feature reference and no external error. Every
  affected sketch must solve.
- **Result.** It returns a commit-capable `Prepared` only when every row staged
  and the gate passed. Otherwise it returns the blockers.

The live document never changes during staging or checking. Committing the
preview applies every row at once. Undo restores the original broken document,
and Redo restores the repaired one.

## Guards

- **Source and Session.** The stamp rejects a set staged against another
  document, revision or source hash. Session commit rejects a stale or foreign
  preview.
- **Project resources.** The resource fingerprint (`ProjectResources::fingerprint`)
  covers authored payloads, project metadata and required capabilities. A
  change invalidates the set. Derived caches are excluded.
- **Window.** A set staged in the window is discarded as a whole when its stamp
  stops matching: any commit, Undo, Redo or resource change. Staging,
  unstaging or discarding a row cancels the check in flight. A verdict is
  adopted only for exactly the rows it checked.

## Window

The sketch editor Inspector shows **Staged reference repairs** whenever the
reference evaluation reports a broken sketch reference. It also lists the
references of a sketch whose evaluation fails while its bindings still
resolve, for example an intersection whose edge no longer reaches the sketch
plane.

- **Stage.** **Stage…** opens a draft for one reference.
  - For a feature-source reference, the draft offers **Stage replacement**.
    Choose a current source feature and an element of the same kind.
  - **Stage removal** names the dependent constraints and curves it will
    remove first.
- **Review.** Staged rows list their old binding, the replacement, the affected
  count, any error, and the blockers.
- **Check.** **Check repairs** runs the native check on a worker thread. The
  app supplies it as a `RepairSetChecker`; the UI crate has no native model
  dependency.
- **Apply.** **Apply all** is enabled only for a passing verdict that still
  matches the current source and resources. **Discard all** leaves the
  document unchanged.

## External control

The MCP server exposes the same rows:

- **`reference_candidates`** lists a source feature's current exact faces,
  edges and vertices. Each comes with its element key and analytic geometry.
- **`preview_reference_repairs`** is read-only. It stages and checks the rows
  and reports `rows`, `blockers`, `sketches`, `references_resolved` and
  `can_apply`. An optional `at_revision` refuses a moved document.
- **`apply_reference_repairs`** requires `expected_revision`. It commits, or
  queues for review, only a passing candidate. A blocked set fails with
  `repair_blocked`. A stale revision or changed resources fail as
  `stale_revision`. In every failure the document is unchanged.

## Limits

- **Occurrence rows in the window.** The window stages occurrence references
  for removal only. Rebinding an occurrence reference needs the canonical
  occurrence capture request, which `rebind_occurrence` accepts through the
  API.
- **Source-edit rows.** In the window, replacement geometry must already exist
  in the document. Source-edit rows are available through the typed API and
  MCP.
- **Candidate selection.** Candidates are selected from a list with analytic
  geometry hover text. They are not picked in the 3D view.
- **Element keys.** An element key is valid only for the revision and candidate
  at which it was listed. A `rebind` that follows a source edit of the same
  feature should use keys read from that candidate.
