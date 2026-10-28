use crate::{controls, egui};
use multicad_core::{
    document::{Document, EntityId, Placement},
    parameters::{self, Unit},
    symbols::{SymbolDefinition, SymbolKind, SymbolValue},
};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(super) struct ValueInput {
    pub text: String,
    pub unit: String,
    pub enabled: bool,
}
#[derive(Clone, Default)]
pub(super) struct Overrides(
    pub BTreeMap<EntityId, ValueInput>,
    BTreeMap<EntityId, SymbolValue>,
);
impl Overrides {
    pub fn new(
        definition: &SymbolDefinition,
        values: &BTreeMap<EntityId, SymbolValue>,
    ) -> Result<Self, String> {
        let evaluated = parameters::evaluate(&definition.template).map_err(|e| e.to_string())?;
        let mut inputs = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        if definition.kind == SymbolKind::Dynamic {
            for id in &definition.exposed {
                let parameter = evaluated
                    .values
                    .get(id)
                    .ok_or("An exposed symbol parameter is missing")?;
                let value = evaluated
                    .value(*id, parameter.unit)
                    .map_err(|e| e.to_string())?;
                defaults.insert(
                    *id,
                    SymbolValue {
                        value,
                        unit: parameter.unit.symbol().into(),
                    },
                );
                inputs.insert(
                    *id,
                    ValueInput {
                        text: value.to_string(),
                        unit: parameter.unit.symbol().into(),
                        enabled: false,
                    },
                );
            }
        }
        for (id, value) in values {
            inputs.insert(
                *id,
                ValueInput {
                    text: value.value.to_string(),
                    unit: value.unit.clone(),
                    enabled: true,
                },
            );
        }
        Ok(Self(inputs, defaults))
    }
    pub fn resolve(
        &self,
        definition: &SymbolDefinition,
    ) -> Result<BTreeMap<EntityId, SymbolValue>, String> {
        let mut values = BTreeMap::new();
        for (id, field) in &self.0 {
            if !field.enabled {
                continue;
            }
            let name = definition
                .template
                .get(*id)
                .map_or("Removed parameter", |e| e.name.as_str());
            if definition.kind != SymbolKind::Dynamic || !definition.exposed.contains(id) {
                return Err(format!(
                    "Remove the override for {name}; it is no longer exposed"
                ));
            }
            let value = number(&field.text, name)?;
            let unit =
                Unit::parse(&field.unit).ok_or_else(|| format!("{name}: unknown override unit"))?;
            let expected = definition
                .template
                .get(*id)
                .and_then(|e| match &e.data {
                    multicad_core::document::EntityData::Parameter { unit, .. } => {
                        Unit::parse(unit)
                    }
                    _ => None,
                })
                .ok_or("Exposed parameter is missing")?;
            if unit.dimension() != expected.dimension() {
                return Err(format!(
                    "{name}: override unit is incompatible; reset it to the new default"
                ));
            }
            values.insert(
                *id,
                SymbolValue {
                    value,
                    unit: field.unit.clone(),
                },
            );
        }
        Ok(values)
    }
    pub fn show(&mut self, ui: &mut egui::Ui, definition: &SymbolDefinition) -> bool {
        if self.0.is_empty() {
            ui.label("Fixed design values · no instance inputs");
            return false;
        }
        let mut changed = false;
        for (id, field) in &mut self.0 {
            changed |= ui
                .push_id(*id, |ui| {
                    show_value(ui, *id, field, definition, self.1.get(id))
                })
                .inner;
            ui.add_space(4.);
        }
        changed
    }
}
fn reset(field: &mut ValueInput, default: Option<&SymbolValue>) {
    field.enabled = false;
    if let Some(default) = default {
        field.text = default.value.to_string();
        field.unit = default.unit.clone();
    }
}
fn show_value(
    ui: &mut egui::Ui,
    id: EntityId,
    field: &mut ValueInput,
    definition: &SymbolDefinition,
    default: Option<&SymbolValue>,
) -> bool {
    let entity = definition.template.get(id);
    let allowed = definition.kind == SymbolKind::Dynamic && definition.exposed.contains(&id);
    let name = entity.map_or("Removed parameter", |e| e.name.as_str());
    let mut changed=ui.checkbox(&mut field.enabled,format!("Override {name}")).on_hover_text("Checked values replace this instance's default formula. Uncheck to restore the definition's default").changed();
    if changed && !field.enabled {
        reset(field, default);
    }
    if !allowed {
        if field.enabled {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "No longer exposed. Uncheck to discard this override explicitly.",
            );
        }
        return changed;
    }
    if field.enabled {
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut field.text)
                        .desired_width((controls::inspector_width(ui) - 65.).max(40.)),
                )
                .on_hover_text("Finite numeric value for this instance only")
                .changed();
            ui.label(&field.unit);
        });
    }
    if let Some(default) = default {
        ui.small(format!("Default: {} {}", default.value, default.unit));
    }
    if let Some(entity) = entity
        && let multicad_core::document::EntityData::Parameter { expression, .. } = &entity.data
    {
        ui.small(format!("Definition: {expression}"));
    }
    if let Some(description) = entity
        .map(|e| e.description.as_str())
        .filter(|description| !description.is_empty())
    {
        ui.small(description);
    }
    let expected = entity.and_then(|e| match &e.data {
        multicad_core::document::EntityData::Parameter { unit, .. } => Unit::parse(unit),
        _ => None,
    });
    if field.enabled
        && expected.is_some_and(|u| {
            Unit::parse(&field.unit).is_none_or(|old| old.dimension() != u.dimension())
        })
    {
        ui.colored_label(egui::Color32::LIGHT_RED, "Override has incompatible units");
        if ui.button("Use new default").clicked() {
            reset(field, default);
            changed = true;
        }
    }
    changed
}

pub(super) fn number(text: &str, label: &str) -> Result<f64, String> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && v.abs() <= 1e9)
        .ok_or_else(|| format!("{label}: enter a finite number no greater than 1,000,000,000"))
}
pub(super) fn text(ui: &mut egui::Ui, label: &str, value: &mut String, help: &str) -> bool {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(controls::inspector_width(ui)))
        .on_hover_text(help)
        .changed()
}
pub(super) fn entity_label(document: &Document, id: EntityId) -> String {
    format!(
        "{} · {}",
        document.get(id).map_or("Missing", |e| e.name.as_str()),
        &id.to_string()[..6]
    )
}
pub(super) fn entity_choice(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    selected: &mut Option<EntityId>,
    entries: &[(EntityId, String)],
    optional: bool,
) -> bool {
    let before = *selected;
    ui.label(label);
    let selected_text = selected
        .and_then(|id| {
            entries
                .iter()
                .find(|(key, _)| *key == id)
                .map(|(_, name)| name.clone())
        })
        .unwrap_or_else(|| {
            if selected.is_some() {
                "Missing selection".into()
            } else if optional {
                "None".into()
            } else {
                "Choose…".into()
            }
        });
    controls::inspector_combo(ui, id)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            if optional && ui.selectable_value(selected, None, "None").clicked() {
                ui.close();
            }
            for (id, name) in entries {
                if ui
                    .selectable_value(selected, Some(*id), name)
                    .on_hover_text(id.to_string())
                    .clicked()
                {
                    ui.close();
                }
            }
        });
    before != *selected
}
#[derive(Clone)]
pub(super) struct PoseInputs {
    pub translation: [String; 3],
    pub axis: [String; 3],
    pub angle: String,
}
impl Default for PoseInputs {
    fn default() -> Self {
        Self {
            translation: std::array::from_fn(|_| "0".into()),
            axis: ["0".into(), "0".into(), "1".into()],
            angle: "0".into(),
        }
    }
}
impl PoseInputs {
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.strong("Placement · world coordinates");
        for (axis, value) in ["X (mm)", "Y (mm)", "Z (mm)"]
            .into_iter()
            .zip(&mut self.translation)
        {
            changed |= text(
                ui,
                axis,
                value,
                "World translation of the symbol's shared 2D/3D instance",
            );
        }
        changed |= text(
            ui,
            "Rotation (deg)",
            &mut self.angle,
            "Rotation about the explicit world axis below",
        );
        ui.collapsing("Rotation axis", |ui| {
            for (axis, value) in ["Axis X", "Axis Y", "Axis Z"]
                .into_iter()
                .zip(&mut self.axis)
            {
                changed |= text(
                    ui,
                    axis,
                    value,
                    "Nonzero direction; normalized for placement",
                );
            }
        });
        changed
    }
    pub fn resolve(&self) -> Result<Placement, String> {
        let mut t = [0.; 3];
        let mut axis = [0.; 3];
        for i in 0..3 {
            t[i] = number(&self.translation[i], "Translation")?;
            axis[i] = number(&self.axis[i], "Rotation axis")?;
        }
        let angle = number(&self.angle, "Rotation")?.to_radians();
        let n = axis.iter().map(|v| v * v).sum::<f64>().sqrt();
        if n <= 1e-12 {
            return Err("Rotation axis must be nonzero".into());
        }
        let s = (angle / 2.).sin() / n;
        Ok(Placement {
            translation_mm: t,
            rotation_xyzw: [axis[0] * s, axis[1] * s, axis[2] * s, (angle / 2.).cos()],
        })
    }
}
