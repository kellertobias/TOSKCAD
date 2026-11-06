# ADR-0037: Thread representation as a retained instance input

Status: accepted implementation design, 2026-11-06. Engineering verification is
in progress under [CAD-234](https://plans.tokenet.de/CAD/234).

A placed cosmetic nut needs an explicit route to a modeled internal helix.
Separate source revisions make this a definition-wide update and can block
conversion when both revisions are already embedded. A representation choice
therefore belongs in the existing per-instance configuration of one pinned
definition.

`ThreadParameters.representation_parameter` optionally binds an existing Count
parameter. Exact value 0 selects `CosmeticEnvelope`; exact value 1 selects
`Modeled`. When bound, that parameter is the sole authority. Missing targets,
wrong entity kinds or units, and other values fail without falling back to the
retained literal `representation`. Unbound recipes keep their existing literal
behavior and omit the new field when serialized.

The binding is an explicit canonical dependency for standalone threads and
nested nut/screw recipes. Template extraction and typed identity remapping
carry it into each instance's private namespace. Existing scalar overrides
configure instances independently without changing the pin or editing private
feature source directly. This introduces no second document or generic enum
value format.

Native construction and manufacturing-history checks use the same effective
representation. History evaluates receiving parameters lazily once per walk;
legacy literal-only history retains its existing path. A valid closed cosmetic
solid cannot authorize manufacturing. The opposite-literal controls are
important: bound 0 with literal Modeled must refuse export, while bound 1 with
literal Cosmetic must construct a real helix.

The Library discovers this semantic input from typed source bindings rather
than parameter names. Placement, instance editing and update repair show
Cosmetic envelope and Modeled helix, with the usual explicit override/reset
behavior. Ordinary numeric inputs retain their editors. Standalone controls
preserve bindings on unrelated edits and require an explicit switch to fixed
representation before manual selection. MCP omission preserves the binding;
setting and clearing it are explicit and mutually exclusive operations.

Source updates retain compatible override identities. If a parameter acquires
or loses its representation role, an existing retained override becomes a
visible conflict and preparation refuses it until explicitly reset. The
transaction enforces this even when a caller bypasses the comparison screen.
Ordinary and architectural symbol instances follow the same rule.

The conditional reader capability is
`model.thread.representation_parameter.1`. Discovery includes suppressed,
template-only, private instance and retained nested provider source. A retained
occurrence of the new field, including explicit null, requires a capable
reader; legacy serialization omits it. Immutable catalog reference geometry
remains literal and self-contained and cannot contain a live binding.

Representation changes preserve dimensional source attribution. A physical
dimension change still becomes configured/custom through the existing reference
comparison. DIN 934:1987 and ISO 4032:2012 reference rows remain distinct; the
binding does not certify thread fit, stock availability, tolerances or omitted
chamfers. Real native section/STEP checks, source/history/reader regressions and
actual macOS configuration must establish the completed workflow separately.
