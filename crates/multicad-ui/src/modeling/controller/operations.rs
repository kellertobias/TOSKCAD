//! The cut discovery reply cannot authorize Apply. It only selects targets for
//! a second, complete transaction evaluated on the original source Session.
use super::*;
use multicad_core::model::Operation;

impl ModelToolController {
    pub(super) fn operation_changed(&mut self) {
        if let Some(ModelForm::Extrude(panel)) = &mut self.form {
            panel.changed();
        }
        self.invalidate();
    }
    pub fn set_extrusion_context(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        let result = self
            .operations
            .as_mut()
            .ok_or("Start a new extrusion before choosing operation targets")?
            .set_context(session.document(), session.revision(), scene);
        // A new context must not preserve a receipt prepared with different
        // target choices, even if an external caller refreshes it later.
        self.operation_changed();
        result
    }
    pub(super) fn submit_extrusion(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        channel: Channel,
        form: FormStamp,
        extrusion: &ExtrudeRequest,
    ) -> Result<(), String> {
        self.current(session, resources, true)?;
        if self.stamp() != Some(form) {
            return Err("Extrusion inputs changed; Preview the current values".into());
        }
        let controls = self
            .operations
            .as_ref()
            .ok_or("Start a new extrusion first")?;
        match controls.operation {
            Operation::NewBody => self.submit_final(
                session,
                resources,
                channel,
                form,
                &ModelRequest::Extrude(Box::new(extrusion.clone())),
            ),
            Operation::Cut => {
                let cut = CutDiscovery::prepare(
                    session,
                    Actor::local(channel),
                    extrusion.clone(),
                    controls.component,
                    controls.include.iter().copied().collect(),
                    controls.exclude.iter().copied().collect(),
                )?;
                self.generation = self
                    .generation
                    .checked_add(1)
                    .ok_or("Model preview generation limit reached")?;
                let stamp = SceneStamp {
                    document: session.document().id(),
                    revision: session.revision(),
                    generation: self.generation,
                };
                self.backend
                    .as_mut()
                    .ok_or("The native modeling preview provider is unavailable")?
                    .request_evaluation(cut.prepared.document(), resources, stamp, &cut.options)?;
                self.discovery = Some(Discovery {
                    cut,
                    form,
                    stamp,
                    channel,
                });
                self.candidate = None;
                self.error = None;
                Ok(())
            }
            Operation::Join | Operation::Intersect => {
                let targets = controls.explicit_targets(session.document())?;
                let request = self.identities.compose(
                    extrusion.clone(),
                    controls.operation,
                    targets,
                    controls.keep_tools,
                )?;
                self.submit_final(
                    session,
                    resources,
                    channel,
                    form,
                    &ModelRequest::ExtrudeBoolean(Box::new(request)),
                )
            }
            Operation::Split => {
                Err("Split extrusion outputs are not available in this form".into())
            }
        }
    }
    pub(super) fn receive_discovery(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        reply: crate::viewport::SceneReply,
    ) -> Result<(), String> {
        let discovery = self
            .discovery
            .take()
            .ok_or("The cut query is no longer active")?;
        self.current(session, resources, true)?;
        if self.stamp() != Some(discovery.form)
            || ContentHash::document(discovery.cut.prepared.document())? != discovery.cut.content
        {
            return Err("Extrusion controls changed; Preview the current cut".into());
        }
        let scene = reply.result?;
        let selection = discovery
            .cut
            .selection(session, &scene, discovery.stamp)?
            .clone();
        let controls = self
            .operations
            .as_mut()
            .ok_or("The extrusion operation was cancelled")?;
        if controls.operation != Operation::Cut {
            return Err("The extrusion operation changed; Preview again".into());
        }
        controls.receive(selection.clone());
        let request = self.identities.compose(
            discovery.cut.extrusion().clone(),
            Operation::Cut,
            selection.targets,
            controls.keep_tools,
        )?;
        self.submit_final(
            session,
            resources,
            discovery.channel,
            discovery.form,
            &ModelRequest::ExtrudeBoolean(Box::new(request)),
        )
    }
}
