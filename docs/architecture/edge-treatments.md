# Parametric Fillet and Chamfer

The [edge feature](../../crates/multicad-core/src/model/edge_treatment.rs)
persists a Fillet radius or Chamfer controls against selected native edges of
one Part definition. Chamfer supports symmetric distance, two distances and a
distance/angle pair. Linked occurrences follow that definition; Instance-only
treatment remains unsupported.

Both profiles use the same Inspector and prepared transaction boundary. Native
B-reps supply the result geometry; configured
[numerical tolerances](0006-geometry-policy.md) still govern validity.

## Frozen source and captured edges

Each `model.edge_treatment` feature records a preceding `BodyInput` producer and
retains its output Part identity. The parameter payload stores the profile, a
bound millimeter size and the identities of its selected edge captures. Native
topology ordinals never enter that payload.

Symmetric controls retain schema 1. Schema 2 adds bound second-distance or angle
settings and an explicit edge-to-adjacent-face mapping. Canonical links own both
edge and face intents. The native builder verifies adjacency; neither display
order nor an implicit choice of side defines the angle or second distance.

Canonical input links own each complete `GeometryIntent` and target the frozen
producer. The [source reader](../../crates/multicad-core/src/model/edge_treatment.rs)
checks capture identities, link roles and source ownership together. Parameter
references use the same stable bindings as other modeling features.

The [native evaluator](../../crates/multicad-model/src/edge_treatment.rs) resolves
each capture against the current rebuilt upstream topology. Only those transient
keys reach the kernel adapter. Missing or ambiguous edges report a repair
diagnostic instead of selecting another edge by position or display order.

## Native history and result ownership

The [kernel adapter](../../crates/multicad-kernel/src/edge_treatment.rs) deep-copies
the input before invoking the native fillet or chamfer builder. It rejects
invalid sizes, duplicated keys and degenerate or unsupported edge choices. A
result must pass the native solid checks before it can replace any candidate.

Exact native copy and modification history preserve the original geometry
origins. Supplied treatment origins identify generated edge faces; incident
selected-edge origins identify vertex-generated corner faces. The adapter does
not derive persistent names from result ordinals or geometric proximity.

Planar results can receive frames established by native planar geometry.
Curved blends receive no invented planar descriptor. The
[native fixtures](../../crates/multicad-kernel/tests/edge_treatment.rs) cover
generated corners, planar chamfer references after dimension changes, source
preservation and STEP readback.

## Selection and checked application

The [host](../../crates/multicad-ui/src/sketch_edge_host.rs) starts with a selected
Part or requests one through the shared target stage. The
[Inspector](../../crates/multicad-ui/src/modeling/edge_treatment.rs) accepts an
empty edge draft while disabling Preview and Apply. Shift-click adds or removes
edges while retaining unchanged capture identities.

Sided Chamfer presents a face-selection stage for each edge. A single native
face capture can serve several incident edges. Selecting a side preserves its
edge binding; switching back to Fillet or symmetric Chamfer removes the sided
settings and retains the compatible edge source.

Edge selection displays the frozen upstream producer, including when editing a
saved feature whose current result no longer contains the original edges. This
source-only view supplies display and picking data. The complete current native
scene still validates the source context and candidate receipt.

Preview turns off edge picking and evaluates the complete candidate model.
Apply requires its matching source, Session and form stamp before committing
one transaction. Cancel preserves canonical source. A failed native receipt
keeps the editable draft and disables Apply; it does not substitute a mesh result.

The [compiler fixtures](../../crates/multicad-model/tests/edge_treatment.rs)
check bound source rebuilding, linked occurrence propagation, missing/ambiguous
captures, Undo/Redo, container reopening and exact STEP regeneration. Automated
[Inspector/canvas tests](../../crates/multicad-ui/src/sketch_edge_host_tests.rs)
exercise native edge selection, profile switching, saved source reselection and
checked Preview/Apply. Physical application review and manufacturing acceptance
remain separate evidence.

The [native build allowlist](../../crates/multicad-native-build/src/lib.rs)
includes `TKFillet` under the existing OCCT source pin. Its configuration identity
selects a separate completed native cache. Cross-platform runtime behavior and
low-power performance require their own verification.
