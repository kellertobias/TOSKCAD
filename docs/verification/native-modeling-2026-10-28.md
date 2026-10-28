# Native modeling checkpoint — 2026-10-28

CAD-64 remains In progress. This checkpoint verifies initial independent-body
extrusion in the production application and additional native model providers.
It does not record human acceptance or completion of the modeling workspace.

## Source and native geometry

The canonical core now defines extrusion, box/cylinder primitives, revolution
and explicit Join/Cut/Intersect histories. Preparations preserve typed units,
parameter identity bindings, output identities and transactional Undo. They
validate source; the native compiler supplies the additional geometry gate.

The production application supplies separate canonical and candidate workers.
Display meshes and picking structures are built on the worker. The tool retains
its prepared transaction and requires a matching successful native result before
Apply. Source, session, resources, form generation and output producer checks
reject stale replies. The exact transaction is committed once.

The model suite reports **22 passing tests**. It checks genuine native volumes,
bounds, profile identity, parameter edits, generated-face attachments, exact
STEP write/read, Boolean history and version-scoped tool consumption. Missing
history cannot silently fall back to an earlier producer. Worker tests cover
superseded requests, nonblocking shutdown and terminal failure after an
unexpected worker exit; native work is not retried after a panic.

## Interactive macOS check

The reviewed executable was the production `multicad` binary in
`artifacts/development/MultiCAD-model-review.app`, with SHA-256:

```
2727b399b94c80d94ffa08e55bcd358082594588acf530ada830b573a14d9455
```

It includes the initial UI/controller and production adapter. The later
worker-exit guard is covered by automated tests and was not included in this
interactive executable. The local review bundle has a separate identifier;
nothing was installed or published.

1. Run the packaged `--self-test artifacts/model-review`. It creates two exact
   independent bodies from two rectangles in one canonical sketch, writes STEP
   bodies and saves `parametric-solids.multicad`.
2. Open that project through the native File menu. Model mode shows both solids
   and their independent Part/feature entries. The viewport fits the model.
3. Open the shared sketch, expand Profiles, and Shift-select its 1,600 mm² and
   4,800 mm² regions. Switch to Model and start Extrude. The Inspector identifies
   two selected profiles; Apply is disabled before native validation.
4. Preview a 135° direction, 30 mm normal start offset and 25 mm one-sided
   distance. The viewport shows the candidate and marks it as unapplied. Cancel
   restores the two-body canonical model at revision 0.
5. Start again with 45°, a 30 mm start offset and symmetric **40 mm total**.
   Preview succeeds. Changing the offset to 31 mm disables Apply immediately;
   restoring 30 mm still requires a new Preview.
6. Apply that current preview. Two independent bodies are added in one
   transaction at revision 1. One Undo removes both at revision 2; Redo restores
   both at revision 3. Save As writes `symmetric-applied.multicad` atomically.
7. Quit with both tabs saved. A clean launch with corrected bundle metadata and
   an ad-hoc seal shows a fresh window promptly. Open the saved project: all four
   solids regenerate at the new session's revision 0. Quit with clean tabs.

The independent `model_file_review` executable regenerates the saved source with
the production compiler. Its report is
`artifacts/model-review/symmetric-applied-report.json`:

| Body | Volume, mm³ | Z bounds, mm |
|---|---:|---|
| Original 80 × 60 rectangle, 10 mm extrusion | 48,000 | 0–10 |
| Original 40 × 40 rectangle, 10 mm extrusion | 16,000 | 0–10 |
| New large symmetric body | 135,764.50198781712 | 15.85786437626905–44.14213562373095 |
| New small symmetric body | 45,254.833995939036 | 15.85786437626905–44.14213562373095 |

Each result contains one native solid. The new bounds agree with equal 20 mm
sweeps about the offset plane at 45°; their volume is profile area multiplied by
the normal component of the 40 mm total sweep.

## Packaging and remaining evidence

The initial generated bundle lacked `NSPrincipalClass` and a valid resource seal.
Launch attempts reached an idle AppKit loop without a visible window. Adding
`NSPrincipalClass=NSApplication`, sealing the completed bundle and cleanly
relaunching produced the observed window. This verifies the corrected
combination; it does not isolate either change as the sole cause.

`cargo fmt --all -- --check`, workspace all-target Clippy with warnings denied,
and the locked offline workspace tests pass at this checkpoint. The workspace
test log reports 522 passes and two ignored cases before the subsequent tool
form increments. The ignored explicit GPU probe is separate from the native UI
checks above; the writer interruption helper is exercised by its parent test.

Remaining CAD-64 work includes exposing primitives, revolve and Booleans in the
shared Inspector, editing applied feature controls, default intersecting cut
targets and explicit scope overrides, referenced termination, taper and
independent Split outputs. Current production scene publication explicitly
rejects assembly occurrence placement. General manufacturing exports and Room
workflows remain later stages. No Windows/Linux interactive or low-power
performance result is claimed.
