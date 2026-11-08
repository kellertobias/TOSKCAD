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

    pub fn scale_origin_picking(&self) -> bool {
        matches!(&self.form, Some(ModelForm::Scale(panel)) if panel.picking_origin())
    }

    pub fn scale_origin_viewport(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        view: super::super::TerminationViewport<'_>,
    ) -> Result<Option<&'static str>, String> {
        if !self.scale_origin_picking() {
            return Ok(None);
        }
        let Some(scene) = view.scene else {
            return if view.clicked {
                Err("Wait for the current model before placing a scale origin".into())
            } else {
                Ok(None)
            };
        };
        if scene.stamp().revision != session.revision() || !scene.is_full_model() {
            return Ok(None);
        }
        let Some(pointer) = view.pointer else {
            return Ok(None);
        };
        let Some((point, label, screen)) = origin_snap(scene, &view, pointer) else {
            return if view.clicked {
                Err("Choose a visible vertex, straight edge midpoint or the world origin".into())
            } else {
                Ok(None)
            };
        };
        let color = egui::Color32::from_rgb(120, 211, 255);
        view.painter
            .circle_stroke(screen, 7., egui::Stroke::new(2., color));
        view.painter.circle_filled(screen, 2., color);
        view.painter.text(
            screen + egui::vec2(11., -9.),
            egui::Align2::LEFT_BOTTOM,
            label,
            egui::FontId::proportional(12.),
            color,
        );
        if !view.clicked {
            return Ok(None);
        }
        self.verify_context(session, resources, scene)?;
        let refresh = self.candidate.is_some();
        let Some(ModelForm::Scale(panel)) = &mut self.form else {
            return Ok(None);
        };
        panel.pick_origin(point);
        let nonunit_scale = panel
            .parameters(session.document())
            .and_then(|parameters| {
                parameters
                    .resolve(session.parameters())
                    .map_err(|e| e.to_string())
            })
            .is_ok_and(|parameters| parameters.factors != [1.; 3]);
        self.invalidate();
        if refresh || nonunit_scale {
            let _ = self.preview(session, resources, Channel::Ui);
        }
        Ok(Some(label))
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
        let gesture = self.scale_interaction.show(
            ui,
            &input,
            &parameters,
            evaluated,
            crate::preferences::scale_step(),
        )?;
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

fn origin_snap(
    scene: &Scene,
    view: &super::super::TerminationViewport<'_>,
    pointer: egui::Pos2,
) -> Option<([f64; 3], &'static str, egui::Pos2)> {
    use crate::viewport::{PickFilter, PickOptions};
    use multicad_core::references::AnalyticGeometry;

    let hit = |filter| {
        scene
            .pick_screen_with(
                view.camera,
                view.rect,
                pointer,
                PickOptions {
                    filter,
                    section: view.section,
                    ..Default::default()
                },
            )
            .and_then(|hit| scene.analytic_hit(&hit))
    };
    let marker = |point: [f64; 3], label| {
        let [x, y, depth] = view.camera.project(point.map(|v| v as f32), view.rect);
        let screen = egui::pos2(x, y);
        (depth >= 0.1
            && view.rect.contains(screen)
            && screen.distance(pointer) <= 8.
            && view.section.is_none_or(|section| section.contains(point)))
        .then_some((point, label, screen))
    };
    if let Some(AnalyticGeometry::Point { position_mm }) = hit(PickFilter::Vertices)
        && let Some(vertex) = marker(position_mm, "Model vertex")
    {
        return Some(vertex);
    }
    if let Some(AnalyticGeometry::Line { a_mm, b_mm }) = hit(PickFilter::Edges)
        && let Some(midpoint) = marker(
            std::array::from_fn(|axis| a_mm[axis] + (b_mm[axis] - a_mm[axis]) * 0.5),
            "Edge midpoint",
        )
    {
        return Some(midpoint);
    }
    marker([0.; 3], "World origin")
}
