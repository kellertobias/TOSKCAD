use super::*;
use std::collections::BTreeSet;

#[derive(Default)]
pub struct PlacementViewportResult {
    pub outcome: Option<ModelToolOutcome>,
    /// The host must not use this press, drag or release to acquire a target.
    pub input_owned: bool,
}

impl ModelToolController {
    pub fn placement_active(&self) -> bool {
        matches!(self.form, Some(ModelForm::Placement(_)))
    }

    /// Explicit canonical targets, without expanding Part definitions to their
    /// dependent occurrences. The target-acquisition host owns selection UX.
    pub fn placement_targets(&self) -> Option<&BTreeSet<EntityId>> {
        match &self.form {
            Some(ModelForm::Placement(panel)) => Some(panel.selected()),
            _ => None,
        }
    }

    /// Retarget a new placement batch while retaining its numeric controls and
    /// copy choice. An empty set keeps those controls but cannot produce Apply.
    pub fn retarget_placement(
        &mut self,
        session: &Session,
        resources: &ProjectResources,
        scene: &Scene,
        selected: &BTreeSet<EntityId>,
    ) -> Result<(), String> {
        if self.placement_interaction.active() {
            return Err(
                "Finish or cancel the current placement gesture before changing targets".into(),
            );
        }
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Placement targets require the complete current native model".into());
        }
        let Some(ModelForm::Placement(panel)) = self.form.as_mut() else {
            return Err("Start Move / Copy before choosing its targets".into());
        };
        if panel.selected() == selected {
            return Ok(());
        }
        panel.retarget(session.document(), session.revision(), scene, selected)?;
        self.invalidate();
        Ok(())
    }

    /// Paint and operate the rigid-placement handles after the canonical scene
    /// and before target acquisition. Releasing stages a preview for Apply.
    pub fn placement_viewport(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        input: super::super::PlacementViewport<'_>,
    ) -> Result<PlacementViewportResult, String> {
        let result = self.placement_viewport_inner(ui, session, resources, input);
        if let Err(error) = &result {
            self.placement_interaction = Default::default();
            self.candidate = None;
            self.keyboard_confirm = None;
            if let Some(ModelForm::Placement(panel)) = &mut self.form {
                panel.set_copy_modifier(false);
            }
            self.error = Some(error.clone());
        }
        result
    }

    fn placement_viewport_inner(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        resources: &ProjectResources,
        input: super::super::PlacementViewport<'_>,
    ) -> Result<PlacementViewportResult, String> {
        if !self.placement_active() {
            return Ok(PlacementViewportResult::default());
        }
        self.current(session, resources, true)?;
        let Some(scene) = input.scene else {
            if self.placement_interaction.active() {
                return Err("The canonical source model is unavailable; the placement gesture was cancelled".into());
            }
            return Ok(PlacementViewportResult::default());
        };
        self.verify_context(session, resources, scene)?;
        if !scene.is_full_model() {
            return Err("Placement handles require the complete canonical native model".into());
        }
        let Some(ModelForm::Placement(panel)) = &self.form else {
            unreachable!()
        };
        if panel.selected().is_empty() {
            return Ok(PlacementViewportResult::default());
        }
        let parameters = panel.parameters(session.document())?;
        let evaluated = session.parameters();
        let gesture = self
            .placement_interaction
            .show(ui, &input, &parameters, evaluated)?;
        let Some(ModelForm::Placement(panel)) = &mut self.form else {
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
            && let Some(ModelForm::Placement(panel)) = &self.form
        {
            let pose = panel
                .parameters(session.document())?
                .resolve(evaluated)
                .map_err(|error| error.to_string())?;
            paint_placement_guide(
                ui,
                scene,
                input.camera,
                input.rect,
                panel.selected(),
                |point| pose.point(point),
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

/// An immediate wire outline of the targeted geometry while the exact model
/// builds. Display lines carry no candidate receipt and cannot enable Apply.
pub(super) fn paint_placement_guide(
    ui: &egui::Ui,
    scene: &Scene,
    camera: crate::camera::Camera,
    rect: egui::Rect,
    targets: &BTreeSet<EntityId>,
    transform: impl Fn([f64; 3]) -> [f64; 3],
) {
    let painter = ui.painter_at(rect);
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(104, 200, 249));
    let mut budget = 4096;
    for instance in scene
        .instances()
        .iter()
        .filter(|item| targets.contains(&item.id))
    {
        let Some(geometry) = scene.producer_geometry(instance.geometry) else {
            continue;
        };
        let line = |a, b| {
            if let Some(segment) = camera.segment(transform(a), transform(b), rect) {
                painter.line_segment(segment, stroke);
            }
        };
        let segments = geometry
            .edges()
            .iter()
            .map(|edge| edge.points_mm.len().saturating_sub(1))
            .sum::<usize>();
        if segments > 0 && segments <= budget {
            budget -= segments;
            for edge in geometry.edges() {
                for pair in edge.points_mm.windows(2) {
                    line(
                        instance.transform.point(pair[0]),
                        instance.transform.point(pair[1]),
                    );
                }
            }
        } else if let Some([low, high]) = scene.instance_bounds(instance.id) {
            let corner = |bits: usize| {
                std::array::from_fn(|i| {
                    if bits & (1 << i) == 0 {
                        low[i]
                    } else {
                        high[i]
                    }
                })
            };
            for bits in 0..8 {
                for axis in [1, 2, 4] {
                    if bits & axis == 0 {
                        line(corner(bits), corner(bits | axis));
                    }
                }
            }
        }
    }
}
