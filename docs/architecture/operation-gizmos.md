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
- **Unsupported combinations are reported, never dropped silently.** One
  Extrude/Revolve feature reads areas of one sketch (canonical
  `ProfileSource { sketch, region }`). Several selected sketches keep the tool
  in acquisition with a message asking for one sketch. Selected non-sketch
  items (Parts, planes, planar model faces) are named in the status message as
  unused: planar-face extrusion is not representable in the canonical
  `ExtrudeDefinition` and is not supported by this command.

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
- `sketch_editor::tests::operation_selection_tests`: preselected sketch opens
  Extrude/Revolve immediately; incompatible preselection is reported;
  acquisition with Control-click accumulation, then Apply opens editing
  without creating a feature.

## Limitations

- No interactive macOS review yet; the GUI was not launched by the agent.
- Planar model faces and multiple sketches per feature are unsupported inputs
  (reported, not dropped).
- Arrow and handle are painted overlays; hit-testing uses screen distance.
  A camera change during a drag cancels the gesture.
- The arrow anchor is the mean of the selected boundary points, which can lie
  outside concave profiles.
