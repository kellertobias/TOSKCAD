# Extrude and Revolve: selection entry, gizmos and direct numeric entry

Status: implemented for CAD-138 (native interaction follow-up to CAD-64).
Agent-tested with real native previews in unit/integration tests; not yet
reviewed interactively on macOS, and Windows/Linux interaction is unverified.

## Entry paths

- **Preselection.** If exactly one sketch is selected in the tree or viewer
  *outside sketch editing*, Extrude/Revolve opens operation editing at once
  with every closed area of that sketch. If closed areas were chosen, exactly
  those are used. No reselection and no selection-confirmation step.
- **Acquisition.** With nothing usable selected (or only the open sketch while
  editing it), the tool enters acquisition. Area clicks in the viewer select;
  Shift, Command (⌘) or Control adds/removes. Viewer clicks never open the
  operation. The Inspector action *Use selected targets* (the acquisition
  Apply) confirms the input set and opens editing; it creates no feature.
- **Planar model faces (CAD-146).** A planar face selected in the viewer
  opens Extrude/Revolve editing at once with that face. With no sketch
  selected, acquisition clicks pick planar faces (Shift, Command or Control
  adds/removes; a plain click replaces); *Use selected targets* opens editing.
  Each face becomes its own output with a face profile (see below).
- **Unsupported combinations are reported, never dropped silently.** One
  sketch-profile feature reads areas of one sketch. Several selected sketches
  keep the tool in acquisition with a message asking for one sketch. Sketch
  areas and planar faces cannot be mixed in one invocation. Other selected
  items (planes, unrelated Parts) are named in the status message as unused;
  the Part that owns a preselected face is its context, not an unused item.

## Gizmo and numeric field

`modeling/value_entry.rs` is a pure, GUI-free state machine
(`ValueEntry`, `EntryInput`, `EntryEffect`, `AngleTracker`). Its rules:

| Input | Effect |
| --- | --- |
| Press on handle, drag | Signed value follows the pointer along the axis (Extrude) or the unwrapped angle about the axis (Revolve); continuous across zero; each valid value → native preview |
| Digits / `.` `,` `-` `+` while no text field has focus | Typing takes over from the drag without focusing; later pointer motion no longer changes the value |
| Click on the adjacent field | Focus; the first character replaces the shown value |
| Pointer release | Ends the drag; **never commits** |
| Enter | Commits exactly once (after the matching native preview succeeds); later Enter / release / typing are inert |
| Escape while typing (or the global Cancel shortcut) | Cancels the tool; source and Undo history unchanged |
| Zero, nonfinite, out-of-range or incomplete text | `Invalid(message)`: shown next to the field and in the Inspector; Apply disabled; nothing committed |

Enter commits only a value the user dragged or typed, so an unseen default
extent is never committed by Enter.

`modeling/controller/operation_interaction.rs` hosts the handles for
`ModelForm::Extrude` and `ModelForm::Revolve` (new features and feature
edits). Every gizmo value is written into the same Inspector
`DimensionField`, so field, Inspector, preview request and commit share one
source of truth. A commit sets `operation.confirm` to the form stamp; the next
frames poll the native worker and call the ordinary `apply` once
`can_apply` holds. A failed native preview clears the confirmation, reopens
the entry for another Enter and leaves the tool open with the error.

The arrow starts at the profile anchor on the start plane (sketch origin
moved by the start offset along N) and points along the positive sweep
direction (elevation, azimuth, Reverse). The field shows `mm` / `°`.
`modeling/operation_guide.rs` derives this display-only placement from the
solved sketch; it is never manufacturing geometry. The arrow is shown only for
one-sided Distance extents; two-sided, symmetric, To plane and To face keep
their Inspector controls.

## Face profiles (CAD-146)

`ExtrudeDefinition.profile` and `RevolveDefinition.profile` are a
`ModelProfile`: either the historical sketch form `{ sketch, region }`
(serialized unchanged) or `{ face: <reference id> }`. The face form stores
only the identity; the canonical `GeometryIntent` (provenance plus a planar
attachment to the face's single semantic source frame) lives once on
`Feature.inputs` with role `profile/face`. The shared reference inspection
therefore reports it like every other face reference, and repair replaces it
explicitly. `model::capture_profile_face` is the one capture used by the
window and MCP; it refuses non-faces, curved faces, faces without a planar
source frame and multi-frame (ambiguous) faces with a message and no source
change. Documents with a face profile require the reader capability
`model.face_profile.1`; older readers refuse them.

Native evaluation (`multicad-model/src/face_profile.rs`) resolves the
reference through `references::inspect`, requires the current native face to
be planar and to contain the attachment frame, orients the frame so +N is the
current **outward** face normal, and extracts the exact wires (outer and
holes, lines/arcs/circles) with `planar_face_contours_with_sources`. A
missing, ambiguous or stale face fails with "Profile face reference is
unresolved (…); repair the reference" and is never retargeted. New-body side
faces are named from the source edges' distinguishing provenance when that is
unique, otherwise by boundary position.

Join and Cut compose as before (New-body tool plus Boolean), so the face
profile works in every operation and in extrusion operation groups. Positive
distances add material outside the face's body; negative distances (or the
arrow dragged inward) go into it. The Revolve axis is expressed in the face
frame's U/V (the Inspector's U/V axis buttons and typed axis apply); the
viewer offers the face's straight boundary edges as axis picks. The arrow
start and the edge list come from the outward display tessellation and are
display-only.

## Signed extrusion distance (core)

`ExtrudeParameters::resolve_controls` accepts a signed nonzero one-sided
`Termination::Distance`: a negative value sweeps the same magnitude opposite to
the configured direction, measured from the configured start plane. It does
not change New body / Join / Cut, start offset or taper. Zero is rejected with
an actionable error. Two-sided ends remain positive magnitudes. The signed
literal is stored unchanged, so Undo/Redo and project save/reopen keep `-25 mm`.
MCP `extrude` uses the same typed path and therefore also accepts a signed
distance.

## Revolve axis acquisition

A new Revolve no longer guesses its axis (previously sketch V through the
origin). Until an axis is chosen, Preview/Apply report "Choose the revolution
axis" and the viewer highlights straight sketch lines (lines and rectangle
edges from the solved sketch); clicking one adopts it (origin = first end,
bearing = line direction). The Inspector also offers *Sketch U axis*, *Sketch V
axis*, a list of straight sketch lines and *Use typed axis*. Reopening an
existing revolve keeps its saved axis. The angular handle rotates about the
chosen axis (right-hand rule, matching the core sign); sweeps are limited to
nonzero values within ±360°.

## Evidence

- `multicad-core/tests/model_extrude.rs`: signed resolve, zero rejection,
  two-sided positivity, commit/undo/redo/project reopen of `-25 mm`.
- `multicad-model/tests/compiler.rs`: native bounds for `-25 mm` and a tilted
  `-10 mm` sweep with a start offset.
- `multicad-ui` `modeling::value_entry::tests`: drag→type→Enter, release never
  commits, zero crossing, invalid text, retry, angle unwrap.
- `modeling::controller::gizmo_entry_tests` (real egui input, native worker):
  drag + type 25 + Enter commits exactly one 25 mm feature with no duplicate on
  release/second Enter; negative drag and `-25` keep New body; zero/invalid
  text and Escape leave source and history; failed native taper keeps the tool
  open and allows retry; revolve axis pick → drag → type 180 → Enter.
- CAD-146 (planar faces): `multicad-model/tests/face_profiles.rs` (exact
  boundary with hole, New body ±, Join, Cut, Revolve volume by Pappus,
  parameter edit moving the face, save/reopen and older-reader refusal,
  Missing repair diagnostic, curved/duplicate/unanchored refusal),
  `multicad-mcp/tests/face_extrude.rs`, `controller::face_entry_tests` (arrow
  drag + typed ±25 + Enter, face-edge axis + typed 90 for Revolve) and
  `sketch_editor::face_profiles::tests` (real viewer preselection; Control-click
  accumulation, toggle, Apply and commit of two face extrusions).
- `sketch_editor::tests::operation_selection_tests`: preselected sketch opens
  Extrude/Revolve immediately; incompatible preselection is reported;
  acquisition with Control-click accumulation, then Apply opens editing
  without creating a feature.

## Limitations

- No interactive macOS review yet; the GUI was not launched by the agent.
- Multiple sketches per feature, and mixed sketch areas plus faces, are
  unsupported inputs (reported, not dropped). Face picking in acquisition
  needs a current native scene; a face of a placed occurrence is referenced
  on its canonical producer, like face-attached sketches.
- Revolve Inspector labels still say "sketch" U/V for face profiles; the
  axis is the face frame's U/V.
- Arrow and handle are painted overlays; hit-testing uses screen distance.
  A camera change during a drag cancels the gesture.
- The arrow anchor is the mean of the selected boundary points, which can lie
  outside concave profiles.
