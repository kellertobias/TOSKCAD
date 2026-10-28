//! External metadata and sketch geometry share one checked Apply/Discard lane.
use super::*;
#[derive(Default)]
pub(super) struct NativeCheck {
    pub required: bool,
    pub ready: bool,
    pub error: Option<String>,
}
impl ConstraintEditor {
    pub(crate) fn external_pending(&self) -> bool {
        self.external_reference.is_some()
    }
    pub(crate) fn external_can_apply(&self, session: &Session, id: EntityId) -> bool {
        self.external_reference.is_some()
            && (!self.external_native.required || self.external_native.ready)
            && self.owner == Some(id)
            && self
                .result
                .as_ref()
                .and_then(|edit| edit.preview.as_ref())
                .is_some_and(|preview| session.validate_preview(preview).is_ok())
            && self.validate_frame(session, id).is_ok()
    }
    #[cfg(test)]
    pub(crate) fn accept_external_edit(
        &mut self,
        session: &Session,
        id: EntityId,
        reference: EntityId,
        edit: SketchEdit,
        label: &str,
    ) -> Result<(), String> {
        self.accept_external_edit_with_report(session, id, reference, edit, label, None)
    }
    pub(crate) fn accept_external_edit_with_report(
        &mut self,
        session: &Session,
        id: EntityId,
        reference: EntityId,
        edit: SketchEdit,
        label: &str,
        report: Option<ReferenceReport>,
    ) -> Result<(), String> {
        if self.dirty() {
            return Err("Apply or discard the current sketch draft first".into());
        }
        if label.trim().is_empty() || label.len() > 128 {
            return Err("The reference edit needs a short Apply label".into());
        }
        let preview = edit
            .preview
            .as_ref()
            .ok_or("Reference edit has no valid checked solution")?;
        session
            .validate_preview(preview)
            .map_err(|e| e.to_string())?;
        let semantic = edit
            .semantic
            .as_ref()
            .ok_or("Reference edit requires a semantic sketch result")?;
        let candidate = preview.document();
        if semantic.sketch != id
            || semantic.stamp.document != session.document().id()
            || semantic.stamp.revision != session.revision()
            || !semantic.is_current(candidate, session.revision())
            || semantic.evaluation != edit.evaluation
        {
            return Err("Reference semantic solution is foreign or stale".into());
        }
        let EntityData::Sketch { plane, .. } =
            &session.document().get(id).ok_or("Sketch is missing")?.data
        else {
            return Err("Select a sketch".into());
        };
        let candidate_sketch = candidate.get(id).ok_or("Candidate sketch is missing")?;
        let EntityData::Sketch {
            plane: next_plane,
            definition,
            ..
        } = &candidate_sketch.data
        else {
            return Err("Candidate changed the sketch kind".into());
        };
        if plane != next_plane
            || semantic.plane != *plane
            || definition.type_id != model::source::TYPE
            || definition.schema_version != 4
        {
            return Err("External edit changed its sketch plane or source provider".into());
        }
        let source = Source::from_definition(definition).map_err(|e| e.to_string())?;
        if &source != semantic.canonical() {
            return Err("Reference source and semantic result disagree".into());
        }
        let old = session.document().get(reference);
        let next = candidate.get(reference);
        if old.is_none() && next.is_none() {
            return Err("Reference edit has no metadata owner".into());
        }
        if old.is_some()
            && !multicad_core::sketch_external::bindings(session.document(), id)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|binding| binding.reference == reference)
        {
            return Err("Reference does not belong to the selected sketch".into());
        }
        if next.is_some()
            && !multicad_core::sketch_external::bindings(candidate, id)
                .map_err(|e| e.to_string())?
                .iter()
                .any(|binding| binding.reference == reference)
        {
            return Err("Candidate reference does not belong to the selected sketch".into());
        }
        let mut changes = vec![Mutation::Replace(candidate_sketch.clone())];
        changes.push(match (old, next) {
            (None, Some(entity)) => Mutation::Insert(entity.clone()),
            (Some(_), Some(entity)) => Mutation::Replace(entity.clone()),
            (Some(_), None) => Mutation::Remove(reference),
            _ => unreachable!(),
        });
        let expected = session
            .document()
            .edited(&changes)
            .map_err(|e| e.to_string())?;
        if &expected != candidate {
            return Err(
                "Reference transaction changed unrelated source entities or document metadata"
                    .into(),
            );
        }
        let (_, frame, _) = Source::read_for_evaluation(
            candidate,
            id,
            session.revision(),
            report.as_ref().or(self.evaluation.as_ref()),
        )
        .map_err(|e| e.to_string())?;
        if semantic.frame != frame {
            return Err("Reference preview uses a different current sketch frame".into());
        }
        let solved = semantic
            .solved()
            .ok_or("Reference edit has no solved geometry")?;
        let external = multicad_core::sketch_external::bindings(candidate, id)
            .map_err(|e| e.to_string())?
            .iter()
            .filter_map(|binding| binding.definition.output.curve())
            .collect();
        if !matches!(
            edit.evaluation.diagnostics.state,
            SolveState::FullyConstrained | SolveState::Underconstrained
        ) || !solution_matches_source(&source, solved, &external)
        {
            return Err("Reference solution does not match its canonical sketch identities".into());
        }
        let geometry = semantic.geometry().map_err(|e| e.to_string())?;
        self.owner = Some(id);
        self.revision = session.revision();
        self.draft = Some(source);
        self.result = Some(edit);
        self.preview_geometry = Some(geometry);
        self.error = None;
        self.apply_label = label.into();
        self.canonical_draft = true;
        self.external_reference = Some(reference);
        self.external_report = report;
        self.external_native = Default::default();
        self.point_display = None;
        self.point_position = None;
        self.dimension_fields.clear();
        self.selection.clear();
        self.last_element = None;
        Ok(())
    }
    pub(crate) fn require_external_native(&mut self) {
        self.external_native = NativeCheck {
            required: true,
            ready: false,
            error: None,
        };
    }
    pub(crate) fn complete_external_native(&mut self, result: Result<(), String>) {
        if self.external_reference.is_none() || !self.external_native.required {
            return;
        }
        self.external_native.ready = result.is_ok();
        self.external_native.error = result.err();
    }
    pub(crate) fn external_prepared(&self) -> Option<&multicad_core::commands::Prepared> {
        self.result.as_ref()?.preview.as_ref()
    }
    pub(super) fn show_external_preview(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        id: EntityId,
    ) {
        ui.strong(&self.apply_label);
        if let Some(reference) = self.external_reference
            && let Some(preview) = self.result.as_ref().and_then(|edit| edit.preview.as_ref())
        {
            if let Ok(binding) = multicad_core::sketch_external::read(preview.document(), reference)
            {
                ui.label(format!(
                    "Source: {}",
                    preview
                        .document()
                        .get(binding.source)
                        .map_or("Missing source", |entity| entity.name.as_str())
                ));
                ui.label(format!(
                    "{} · {}",
                    crate::external_references::method_label(binding.definition.method),
                    crate::external_references::behavior_label(binding.definition.behavior)
                ));
            } else if preview.document().get(reference).is_none() {
                ui.label("This reference and its protected output geometry will be removed.");
            }
        }
        ui.small("This checked reference transaction changes geometry and its source links together. Apply or discard it before editing constraints.");
        if self.external_native.required && !self.external_native.ready {
            if let Some(error) = &self.external_native.error {
                ui.colored_label(
                    Color32::LIGHT_RED,
                    format!("Complete native model validation failed: {error}"),
                );
            } else {
                ui.small("Validating the complete downstream native model…");
            }
        }
        let valid = self.external_can_apply(session, id);
        if !valid && (!self.external_native.required || self.external_native.ready) {
            ui.colored_label(Color32::LIGHT_RED,"The reference preview is stale or unavailable. Discard it and select the source again.");
        }
        use crate::inspector_actions::{self, Button, Owner};
        let clicked = inspector_actions::row(
            ui,
            Owner::Constraints,
            None,
            Button::new(
                &self.apply_label,
                valid,
                "Commit this checked source-linked edit as one undoable transaction",
            ),
            Button::new(
                "Discard reference preview",
                true,
                "Leave canonical source unchanged",
            ),
        );
        if clicked.apply
            && let Err(error) = self.apply(session)
        {
            self.error = Some(error);
        }
        if clicked.cancel {
            self.discard();
        }

        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
}
