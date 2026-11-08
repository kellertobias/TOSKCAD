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

    /// Retarget a new scale batch while retaining its numeric controls and
    /// copy choice. An empty set keeps those controls but cannot produce Apply.
    pub fn retarget_scale(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if self.scale_interaction.active() {
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
    /// and before target acquisition. Releasing stages a preview for Apply.
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
            self.candidate = None;
            self.keyboard_confirm = None;
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
        let evaluated = session.parameters();
        let gesture = self
            .scale_interaction
            .show(ui, &input, &parameters, evaluated)?;
        let Some(ModelForm::Scale(panel)) = &mut self.form else {
            unreachable!()
        };
        let modifier_changed = gesture.moved && panel.set_copy_modifier(gesture.copy_modifier);
        let controls_changed = gesture.changed.is_some();
        if let Some(parameters) = gesture.changed {
            panel.set_drag_parameters(&parameters)?;
        }
        if gesture.released {
            panel.finish_copy_gesture();
        }
        if controls_changed || modifier_changed {
            self.invalidate();
            self.preview(session, resources, Channel::Ui)?;
        }
        if (gesture.moved || self.pending())
            && !self.preview_visible(session, resources)
            && let Some(ModelForm::Scale(panel)) = &self.form
        {
            let transform = panel
                .parameters(session.document())?
                .resolve(evaluated)
                .and_then(model::ResolvedScale::affine)
                .map_err(|error| error.to_string())?;
            super::placement_interaction::paint_placement_guide(
                ui,
                scene,
                input.camera,
                input.rect,
                panel.selected(),
                |point| {
                    std::array::from_fn(|axis| {
                        super::super::gizmo::math::dot(transform.0[axis], point) + transform.1[axis]
                    })
                },
            );
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
