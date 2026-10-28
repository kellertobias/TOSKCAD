//! Legacy adoption is an explicit one-producer conversion, not a search for
//! sibling extrusions with equal sketches or parameters.
use super::*;
use multicad_core::model::{self, BodyInput, Operation};
impl ModelToolController {
    pub(super) fn begin_legacy_extrusion(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        feature: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let source =
            model::read_extrude(session.document(), feature).map_err(|error| error.to_string())?;
        let input = BodyInput {
            body: source.output.body,
            producer: feature,
        };
        let name = GeneratedNames::new(session.document()).numbered("Extrusion operation", "");
        let draft = model::ExtrudeGroupDraft::new(name, None);
        let state = model::inspect_ungrouped_extrude(session.document(), &input, &draft)
            .map_err(|error| error.to_string())?;
        let panel = GroupEditPanel::legacy(session.document(), state, draft.name.clone())?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::ExtrudeGroup(Box::new(panel)),
        )?;
        let scope = self.scope.as_mut().expect("opened exact legacy source");
        scope.execution = Execution::LegacyExtrusion {
            feature,
            group: draft.id,
        };
        scope.adoption = Some(draft);
        Ok(())
    }
    pub(super) fn prepare_legacy_extrusion(
        &self,
        session: &Session,
        channel: Channel,
        feature: EntityId,
        group: EntityId,
        request: &ModelRequest,
    ) -> Result<(model::ModelPrepared, SceneProof), String> {
        let ModelRequest::ExtrudeGroupEdit(edit) = request else {
            return Err("Preview the selected extrusion's controls".into());
        };
        if edit.group != group {
            return Err("The selected extrusion edit belongs to another operation scope".into());
        }
        if edit.operation == Operation::NewBody {
            if !edit.targets.is_empty() || edit.keep_tools {
                return Err(
                    "New body source edits require no Boolean targets or tool retention flag"
                        .into(),
                );
            }
            let prepared = match &edit.faces {
                Some(faces) => model::prepare_extrude_edit_with_faces(
                    session,
                    session.revision(),
                    Actor::local(channel),
                    feature,
                    &edit.parameters,
                    faces,
                ),
                None => model::prepare_extrude_edit(
                    session,
                    session.revision(),
                    Actor::local(channel),
                    feature,
                    &edit.parameters,
                ),
            }
            .map_err(|error| error.to_string())?;
            let proof = SceneProof::edited(prepared.outputs.clone(), &[], vec![]);
            return Ok((prepared, proof));
        }
        let source =
            model::read_extrude(session.document(), feature).map_err(|error| error.to_string())?;
        let draft = self
            .scope
            .as_ref()
            .and_then(|scope| scope.adoption.as_ref())
            .ok_or("The explicit adoption scope is missing")?;
        let mut adoption = model::AdoptExtrudeGroupRequest::new(
            session.document(),
            BodyInput {
                body: source.output.body,
                producer: feature,
            },
            draft.clone(),
        )
        .map_err(|error| error.to_string())?;
        adoption.edit = (**edit).clone();
        let prepared = model::prepare_adopt_extrude_group(
            session,
            session.revision(),
            Actor::local(channel),
            &adoption,
        )
        .map_err(|error| error.to_string())?;
        Self::group_prepared(prepared, group)
    }
}
