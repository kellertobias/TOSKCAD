//! Explicit group records identify shared operations; matching tool arrays or
//! activity history never infer ownership after a document is reopened.
use super::*;
use multicad_core::model::{self, ModelPrepared};
impl ModelToolController {
    pub(super) fn begin_group(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        group: EntityId,
        channel: Channel,
    ) -> Result<(), String> {
        let state = model::inspect_extrude_group(session.document(), group)
            .map_err(|error| error.to_string())?;
        let panel = GroupEditPanel::new(session.document(), state)?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::ExtrudeGroup(Box::new(panel)),
        )?;
        self.scope
            .as_mut()
            .expect("opened operation scope")
            .execution = Execution::EditGroup { group };
        Ok(())
    }
    pub fn set_feature_context(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
    ) -> Result<(), String> {
        let result = (|| {
            self.verify_context(session, resources, scene)?;
            if let Some(ModelForm::ExtrudeGroup(panel)) = &mut self.form {
                panel.set_context(session.document(), session.revision(), scene)?;
            }
            Ok(())
        })();
        self.context_result(scene.stamp(), &result);
        result
    }
    pub(super) fn prepare_group(
        &self,
        session: &Session,
        channel: Channel,
        group: EntityId,
        request: &ModelRequest,
    ) -> Result<(ModelPrepared, SceneProof), String> {
        let ModelRequest::ExtrudeGroupEdit(edit) = request else {
            return Err("Preview the recorded operation's shared controls".into());
        };
        if edit.group != group {
            return Err("The operation edit belongs to a different group".into());
        }
        let prepared = model::prepare_extrude_group_edit(
            session,
            session.revision(),
            Actor::local(channel),
            edit,
        )
        .map_err(|error| error.to_string())?;
        let state = model::inspect_extrude_group(prepared.prepared.document(), group)
            .map_err(|error| error.to_string())?;
        let mut producers: Vec<_> = state
            .definition
            .tools
            .iter()
            .map(|tool| CreatedBody {
                body: tool.body,
                feature: tool.producer,
            })
            .collect();
        producers.extend(prepared.outputs.iter().cloned());
        let proof = SceneProof::edited(producers, &prepared.restored, prepared.removed);
        Ok((
            ModelPrepared {
                prepared: prepared.prepared,
                outputs: prepared.outputs,
            },
            proof,
        ))
    }
}
