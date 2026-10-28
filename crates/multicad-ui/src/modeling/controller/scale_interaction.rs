use super::*;
use std::collections::BTreeSet;
impl ModelToolController {
    pub fn begin_scale(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
        copy: bool,
        channel: Channel,
    ) -> Result<(), String> {
        let panel = super::super::scaling::ScalePanel::new(
            session.document(),
            session.revision(),
            scene,
            selected,
            copy,
        )?;
        self.begin_form(
            session,
            resources,
            channel,
            ModelForm::Scale(Box::new(panel)),
        )?;
        if let Err(error) = self.verify_context(session, resources, scene) {
            self.cancel();
            return Err(error);
        }
        Ok(())
    }
}
impl ModelToolController {
    pub fn scaling_active(&self) -> bool {
        matches!(self.form, Some(ModelForm::Scale(_)))
    }

    /// Explicit canonical targets, without expanding Part definitions to their
    /// dependent occurrences. The target-acquisition host owns selection UX.
    pub fn scale_targets(&self) -> Option<&BTreeSet<EntityId>> {
        match &self.form {
            Some(ModelForm::Scale(panel)) => Some(panel.selected()),
            _ => None,
        }
    }

    pub fn set_scale_copy_modifier(&mut self, active: bool) -> bool {
        if self.scale_interaction.active() || self.scale_confirm.is_some() {
            return false;
        }
        let changed = match &mut self.form {
            Some(ModelForm::Scale(panel)) => panel.set_copy_modifier(active),
            _ => false,
        };
        if changed {
            self.invalidate();
        }
        changed
    }

    /// Retarget a new scale batch while retaining its numeric controls and
    /// copy choice. An empty set keeps those controls but cannot produce Apply.
    pub fn retarget_scale(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if self.scale_interaction.active() || self.scale_confirm.is_some() {
            return Err(
                "Finish or cancel the current scale gesture before changing targets".into(),
            );
        }
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Scale targets require the complete current native model".into());
        }
        let Some(ModelForm::Scale(panel)) = self.form.as_mut() else {
            return Err("Start Scale before choosing its targets".into());
        };
        if panel.selected() == selected {
            return Ok(());
        }
        panel.retarget(session.document(), session.revision(), scene, selected)?;
        self.invalidate();
        Ok(())
    }

    /// Paint and operate the scale handles after the canonical scene
    /// and before the host processes target acquisition. Each release waits for
    /// its matching native receipt, then commits once through the ordinary API.
    pub fn scale_viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        input: super::super::ScaleViewport<'_>,
    ) -> Result<PlacementViewportResult, String> {
        let result = self.scale_viewport_inner(ui, session, resources, input);
        if let Err(error) = &result {
            self.scale_interaction = Default::default();
            self.scale_confirm = None;
            self.candidate = None;
            if let Some(ModelForm::Scale(panel)) = &mut self.form {
                panel.set_copy_modifier(false);
            }
            self.error = Some(error.clone());
        }
        result
    }

    fn scale_viewport_inner(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        mut input: super::super::ScaleViewport<'_>,
    ) -> Result<PlacementViewportResult, String> {
        if !self.scaling_active() {
            return Ok(PlacementViewportResult::default());
        }
        self.current(session, resources, true)?;
        if let Some(expected) = self.scale_confirm {
            if self.stamp() != Some(expected) {
                self.scale_confirm = None;
                return Err(
                    "Scale controls changed; drag again or Preview the current controls".into(),
                );
            }
            self.poll(session, resources);
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            if self.can_apply(session, resources) {
                let instances = self.candidate_instances();
                let parts = self.apply(session, resources)?;
                return Ok(PlacementViewportResult {
                    outcome: Some(if instances.is_empty() {
                        ModelToolOutcome::Applied(parts)
                    } else {
                        ModelToolOutcome::Placed { parts, instances }
                    }),
                    input_owned: true,
                });
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
            return Ok(PlacementViewportResult {
                input_owned: true,
                ..Default::default()
            });
        }
        let new_basis = self.scale_host_basis.is_none_or(|old| old != input.basis);
        let first_edit = self.scale_host_basis.is_none() && self.editing();
        if new_basis
            && !first_edit
            && !self.scale_interaction.active()
            && let Some(ModelForm::Scale(panel)) = &mut self.form
            && panel.set_axes(input.basis.axes)?
        {
            self.invalidate();
        }
        if new_basis && self.scale_interaction.active() {
            return Err("Scale coordinates changed during a gesture; drag again".into());
        }
        self.scale_host_basis = Some(input.basis);
        if let Some(ModelForm::Scale(panel)) = &self.form {
            input.basis = super::super::GizmoBasis {
                axes: panel.parameters(session.document())?.axes,
            };
        }
        let Some(scene) = input.scene else {
            if self.scale_interaction.active() {
                return Err(
                    "The canonical source model is unavailable; the scale gesture was cancelled"
                        .into(),
                );
            }
            return Ok(PlacementViewportResult::default());
        };
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Scale handles require the complete canonical native model".into());
        }
        let Some(ModelForm::Scale(panel)) = &self.form else {
            unreachable!()
        };
        if panel.selected().is_empty() {
            return Ok(PlacementViewportResult::default());
        }
        let parameters = panel.parameters(session.document())?;
        let evaluated =
            multicad_core::parameters::evaluate(session.document()).map_err(|e| e.to_string())?;
        let gesture = self
            .scale_interaction
            .show(ui, &input, &parameters, &evaluated)?;
        let Some(ModelForm::Scale(panel)) = &mut self.form else {
            unreachable!()
        };
        let modifier_changed =
            gesture.input_owned && panel.set_copy_modifier(gesture.copy_modifier);
        let controls_changed = gesture.changed.is_some();
        if let Some(parameters) = gesture.changed {
            panel.set_drag_parameters(&parameters)?;
        }
        if controls_changed || modifier_changed {
            self.invalidate();
        }
        if gesture.moved && (controls_changed || modifier_changed) {
            self.preview(session, resources, Channel::Ui)?;
        }
        if gesture.released && gesture.moved {
            self.scale_confirm = self.stamp();
            ui.ctx().request_repaint();
        } else if gesture.released {
            self.set_scale_copy_modifier(false);
        }
        if gesture.started {
            ui.ctx().request_repaint();
        }
        Ok(PlacementViewportResult {
            input_owned: gesture.input_owned,
            ..Default::default()
        })
    }
}
