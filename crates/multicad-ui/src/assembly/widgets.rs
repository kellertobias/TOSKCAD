use crate::{controls, egui, modeling::dimension::DimensionField};
use multicad_core::{
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{JointCoordinate, JointKind, JointLimit, JointLimits, JointValues, PrimitiveFrame},
};
use std::collections::BTreeMap;
#[derive(Default)]
pub(super) struct Fields {
    fields: BTreeMap<String, DimensionField>,
    pub error: Option<String>,
    pub changed: bool,
}
impl Fields {
    pub fn dimension(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        key: &str,
        label: &str,
        value: &mut Dimension,
        unit: &'static str,
    ) {
        let field = self.fields.entry(key.into()).or_insert_with(|| {
            DimensionField::from_dimension(value, unit).expect("canonical dimension unit")
        });
        if field.show(
            ui,
            label,
            "Enter a value or an expression beginning with = and reference a parameter with $name",
        ) {
            self.changed = true;
        }
        match field.resolve(document) {
            Ok(next) => {
                *value = next;
                field.remember(value);
            }
            Err(error) => {
                ui.colored_label(egui::Color32::LIGHT_RED, &error);
                self.error.get_or_insert(error);
            }
        }
    }
    pub fn reset(&mut self) {
        self.fields.clear();
        self.changed = true;
    }
    pub fn start(&mut self) {
        self.error = None;
        self.changed = false;
    }
    pub fn vector(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        key: &str,
        label: &str,
        value: &mut [Dimension; 3],
    ) {
        controls::inspector_collapsing(ui, key, label, |ui| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                self.dimension(
                    ui,
                    document,
                    &format!("{key}/{i}"),
                    axis,
                    &mut value[i],
                    "mm",
                );
            }
        });
    }
    pub fn frame(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        key: &str,
        value: &mut PrimitiveFrame,
    ) {
        self.vector(
            ui,
            document,
            &format!("{key}/origin"),
            "Origin",
            &mut value.origin,
        );
        let old = (value.u, value.v);
        ui.horizontal_wrapped(|ui| {
            for (label, u, v) in [
                ("XY", [1., 0., 0.], [0., 1., 0.]),
                ("YZ", [0., 1., 0.], [0., 0., 1.]),
                ("XZ", [1., 0., 0.], [0., 0., 1.]),
            ] {
                if ui
                    .small_button(label)
                    .on_hover_text("Set explicit local frame directions; keep the origin")
                    .clicked()
                {
                    value.u = u;
                    value.v = v;
                }
            }
        });
        controls::inspector_collapsing(ui, format!("{key}/directions"), "Frame directions", |ui| {
            for (name, vector) in [("X direction", &mut value.u), ("Y direction", &mut value.v)] {
                ui.label(name);
                for (axis, v) in ["X", "Y", "Z"].iter().zip(vector.iter_mut()) {
                    ui.horizontal(|ui| {
                        ui.label(*axis);
                        self.changed |= ui.add(egui::DragValue::new(v).speed(0.01)).changed();
                    });
                }
            }
        });
        self.changed |= old != (value.u, value.v);
        if let Err(e) = value.validate() {
            self.error.get_or_insert(e.to_string());
        }
    }
    pub fn values(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        key: &str,
        kind: JointKind,
        value: &mut JointValues,
    ) {
        for (i, (coordinate, label)) in [
            (JointCoordinate::X, "Local X"),
            (JointCoordinate::Y, "Local Y"),
            (JointCoordinate::Z, "Local Z"),
        ]
        .into_iter()
        .enumerate()
        {
            if kind.allows(coordinate) {
                self.dimension(
                    ui,
                    document,
                    &format!("{key}/{i}"),
                    label,
                    &mut value.translation[i],
                    "mm",
                );
            }
        }
        if kind.allows(JointCoordinate::Rotation) {
            self.dimension(
                ui,
                document,
                &format!("{key}/rotation"),
                "About local Z",
                &mut value.rotation,
                "deg",
            );
        }
    }
    pub fn limits(
        &mut self,
        ui: &mut egui::Ui,
        document: &Document,
        kind: JointKind,
        value: &mut JointLimits,
    ) {
        controls::inspector_collapsing(ui, "joint-limits", "Inclusive motion limits", |ui| {
            for (i, (coordinate, label)) in [
                (JointCoordinate::X, "X travel"),
                (JointCoordinate::Y, "Y travel"),
                (JointCoordinate::Z, "Z travel"),
                (JointCoordinate::Rotation, "Rotation"),
            ]
            .into_iter()
            .enumerate()
            {
                if !kind.allows(coordinate) {
                    continue;
                }
                let limit = if i < 3 {
                    &mut value.translation[i]
                } else {
                    &mut value.rotation
                };
                let mut enabled = limit.is_some();
                if ui.checkbox(&mut enabled, label).changed() {
                    self.changed = true;
                    *limit = enabled.then(|| JointLimit {
                        min: literal(0., if i < 3 { "mm" } else { "deg" }),
                        max: literal(
                            if i < 3 { 100. } else { 90. },
                            if i < 3 { "mm" } else { "deg" },
                        ),
                    });
                }
                if let Some(limit) = limit {
                    for (name, v) in [("Minimum", &mut limit.min), ("Maximum", &mut limit.max)] {
                        self.dimension(
                            ui,
                            document,
                            &format!("limit/{i}/{name}"),
                            name,
                            v,
                            if i < 3 { "mm" } else { "deg" },
                        );
                    }
                }
            }
        });
    }
}
pub(super) fn literal(value: f64, unit: &str) -> Dimension {
    Dimension::Literal {
        value,
        unit: unit.into(),
    }
}
pub(super) fn name(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map_or_else(|| "Missing item".into(), |e| e.name.clone())
}
pub(super) fn choose(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    selected: &mut Option<EntityId>,
    choices: &[(EntityId, String)],
    none: Option<&str>,
) -> bool {
    ui.label(label);
    let mut changed = false;
    controls::inspector_combo(ui, id)
        .selected_text(
            selected
                .and_then(|id| {
                    choices
                        .iter()
                        .find(|(i, _)| *i == id)
                        .map(|(_, n)| n.as_str())
                })
                .unwrap_or(none.unwrap_or("Choose…")),
        )
        .show_ui(ui, |ui| {
            if let Some(none) = none
                && ui.selectable_label(selected.is_none(), none).clicked()
            {
                changed |= selected.take().is_some();
                ui.close();
            }
            for (id, name) in choices {
                if ui.selectable_label(*selected == Some(*id), name).clicked() {
                    changed |= *selected != Some(*id);
                    *selected = Some(*id);
                    ui.close();
                }
            }
        });
    changed
}

pub(super) fn limit_labels(kind: JointKind, limits: &JointLimits) -> Vec<String> {
    let text = |value: &Dimension, unit| {
        DimensionField::from_dimension(value, unit)
            .map(|field| field.text)
            .unwrap_or_else(|_| "Invalid limit".into())
    };
    let mut lines = vec![];
    for (coordinate, name, unit, limit) in [
        (
            JointCoordinate::X,
            "Local X",
            "mm",
            limits.translation[0].as_ref(),
        ),
        (
            JointCoordinate::Y,
            "Local Y",
            "mm",
            limits.translation[1].as_ref(),
        ),
        (
            JointCoordinate::Z,
            "Local Z",
            "mm",
            limits.translation[2].as_ref(),
        ),
        (
            JointCoordinate::Rotation,
            "About local Z",
            "deg",
            limits.rotation.as_ref(),
        ),
    ] {
        if kind.allows(coordinate) {
            lines.push(match limit {
                Some(limit) => format!(
                    "{name} limit: {} to {} {unit}",
                    text(&limit.min, unit),
                    text(&limit.max, unit)
                ),
                None => format!("{name}: no travel limit"),
            });
        }
    }
    if lines.is_empty() {
        lines.push("Fixed relationship: no motion coordinates".into());
    }
    lines
}

#[cfg(test)]
mod limit_text_tests {
    use super::*;
    #[test]
    fn motion_limits_use_user_units_and_only_available_coordinates() {
        let limits = JointLimits {
            rotation: Some(JointLimit {
                min: multicad_core::model::angle(0.),
                max: multicad_core::model::angle(90.),
            }),
            ..Default::default()
        };
        assert_eq!(
            limit_labels(JointKind::Revolute, &limits),
            vec!["About local Z limit: 0 to 90 deg"]
        );
        assert_eq!(
            limit_labels(JointKind::Slider, &limits),
            vec!["Local Z: no travel limit"]
        );
        assert_eq!(
            limit_labels(JointKind::Fixed, &limits),
            vec!["Fixed relationship: no motion coordinates"]
        );
    }
}
