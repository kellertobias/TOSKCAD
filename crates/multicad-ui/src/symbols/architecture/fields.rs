//! Consumer controls show only the authored interface; inheritance keeps the
//! private formulas in the pinned construction untouched.
use super::super::fields::Overrides;
use crate::{controls, egui, modeling::dimension::DimensionField};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::{self, FenceBayLayout, FenceShortPanelPolicy, WallLayerReference, WallLayerSide},
    parameters::{self, Unit},
    symbols::{self, SymbolDefinition, architecture::*},
};
use std::collections::BTreeMap;
struct DimensionRow {
    id: EntityId,
    name: String,
    enabled: bool,
    retargeted: bool,
    field: DimensionField,
    default: String,
}
struct MaterialRow {
    slot: ArchitectureMaterialSlot,
    enabled: bool,
    retargeted: bool,
    value: Option<ArchitectureMaterialValue>,
    choices: Vec<(ArchitectureMaterialValue, String)>,
    default: String,
}
struct OptionRow {
    slot: ArchitectureOptionSlot,
    enabled: bool,
    retargeted: bool,
    value: Option<ArchitectureOptionValue>,
    default: String,
    choices: Vec<(ArchitectureOptionValue, String)>,
}
pub(crate) struct Fields {
    definition: SymbolDefinition,
    numeric: Overrides,
    dimensions: Vec<DimensionRow>,
    materials: Vec<MaterialRow>,
    options: Vec<OptionRow>,
    unavailable: ArchitectureInputs,
    old_names: BTreeMap<EntityId, String>,
}
fn label(document: &Document, id: EntityId) -> String {
    document
        .get(id)
        .map_or_else(|| "Unavailable material".into(), |e| e.name.clone())
}
fn option_label(
    value: &ArchitectureOptionValue,
    construction: &ArchitectureConstruction,
) -> String {
    match value {
        ArchitectureOptionValue::WallReference { reference } => match reference {
            WallLayerReference::Outside => "Outside".into(),
            WallLayerReference::Center => "Center".into(),
            WallLayerReference::Inside => "Inside".into(),
            WallLayerReference::Boundary { layer, side } => {
                let name = match construction {
                    ArchitectureConstruction::Wall(wall) => wall
                        .layers
                        .layers
                        .iter()
                        .find(|l| l.id == *layer)
                        .map_or("Removed layer", |l| l.name.as_str()),
                    _ => "Layer",
                };
                format!(
                    "{name} · {} edge",
                    match side {
                        WallLayerSide::Outside => "outside",
                        WallLayerSide::Inside => "inside",
                    }
                )
            }
        },
        ArchitectureOptionValue::FenceLayout { layout } => match layout {
            FenceBayLayout::Equal => "Equal bays".into(),
            FenceBayLayout::ShortLast => "Short last bay".into(),
        },
        ArchitectureOptionValue::FenceShortPanel { policy } => match policy {
            FenceShortPanelPolicy::Reject => "Reject short bays".into(),
            FenceShortPanelPolicy::Cut => "Cut shorter panels".into(),
        },
    }
}
impl Fields {
    pub(crate) fn new(
        definition: &SymbolDefinition,
        values: &ArchitectureInputs,
        receiving: &Document,
        previous: Option<&SymbolDefinition>,
    ) -> Result<Self, String> {
        let blueprint = blueprint_for_definition(definition)
            .map_err(|e| e.to_string())?
            .ok_or("Choose a wall or fence definition")?;
        let old = previous
            .map(blueprint_for_definition)
            .transpose()
            .map_err(|e| e.to_string())?
            .flatten();
        let parameters = parameters::evaluate(&definition.template).map_err(|e| e.to_string())?;
        let mut construction = ArchitectureConstruction::read(
            &definition.template,
            blueprint.role,
            blueprint.construction,
        )
        .map_err(|e| e.to_string())?;
        let numeric = Overrides::new(definition, &values.parameters)?;
        let mut unavailable = values.clone();
        unavailable.parameters.clear();
        let mut dimensions = Vec::new();
        for slot in &blueprint.dimensions {
            let default = slot
                .target
                .dimension_mut(&mut construction)
                .map_err(|e| e.to_string())?
                .value_in(Unit::Millimeter, Some(&parameters))?;
            let inherited = DimensionField::from_dimension(&model::length(default), "mm")?;
            let default_label = format!(
                "{} {}",
                inherited.text,
                crate::preferences::length_unit().symbol()
            );
            dimensions.push(DimensionRow {
                id: slot.id,
                name: slot.name.clone(),
                enabled: values.dimensions.contains_key(&slot.id),
                retargeted: old
                    .as_ref()
                    .and_then(|b| b.dimensions.iter().find(|s| s.id == slot.id))
                    .is_some_and(|s| s.target != slot.target),
                field: values
                    .dimensions
                    .get(&slot.id)
                    .map(|value| DimensionField::from_dimension(value, "mm"))
                    .transpose()?
                    .unwrap_or(inherited),
                default: default_label,
            });
            unavailable.dimensions.remove(&slot.id);
        }
        let owned = symbols::owned_entities(receiving).map_err(|e| e.to_string())?;
        let mut materials = Vec::new();
        for slot in &blueprint.materials {
            let default = *slot
                .target
                .material_mut(&mut construction)
                .map_err(|e| e.to_string())?;
            let mut choices = slot
                .choices
                .iter()
                .map(|id| {
                    (
                        ArchitectureMaterialValue::Preset { material: *id },
                        format!("{} · definition", label(&definition.template, *id)),
                    )
                })
                .collect::<Vec<_>>();
            if slot.allow_project_materials {
                choices.extend(
                    receiving
                        .entities()
                        .filter(|e| {
                            matches!(e.data, EntityData::Material { .. }) && !owned.contains(&e.id)
                        })
                        .map(|e| {
                            (
                                ArchitectureMaterialValue::Project { material: e.id },
                                format!("{} · project", e.name),
                            )
                        }),
                );
            }
            materials.push(MaterialRow {
                slot: slot.clone(),
                enabled: values.materials.contains_key(&slot.id),
                retargeted: old
                    .as_ref()
                    .and_then(|b| b.materials.iter().find(|s| s.id == slot.id))
                    .is_some_and(|s| s.target != slot.target),
                value: values
                    .materials
                    .get(&slot.id)
                    .cloned()
                    .or_else(|| choices.first().map(|(v, _)| v.clone())),
                choices,
                default: label(&definition.template, default),
            });
            unavailable.materials.remove(&slot.id);
        }
        let mut options = Vec::new();
        for slot in &blueprint.options {
            let default = slot
                .target
                .value(&construction)
                .map_err(|e| e.to_string())?;
            options.push(OptionRow {
                slot: slot.clone(),
                enabled: values.options.contains_key(&slot.id),
                retargeted: old
                    .as_ref()
                    .and_then(|b| b.options.iter().find(|s| s.id == slot.id))
                    .is_some_and(|s| s.target != slot.target),
                value: values
                    .options
                    .get(&slot.id)
                    .cloned()
                    .or_else(|| slot.choices.first().cloned()),
                default: option_label(&default, &construction),
                choices: slot
                    .choices
                    .iter()
                    .map(|v| (v.clone(), option_label(v, &construction)))
                    .collect(),
            });
            unavailable.options.remove(&slot.id);
        }
        let mut old_names = BTreeMap::new();
        if let Some(old) = old {
            old_names.extend(old.dimensions.into_iter().map(|s| (s.id, s.name)));
            old_names.extend(old.materials.into_iter().map(|s| (s.id, s.name)));
            old_names.extend(old.options.into_iter().map(|s| (s.id, s.name)));
        }
        Ok(Self {
            definition: definition.clone(),
            numeric,
            dimensions,
            materials,
            options,
            unavailable,
            old_names,
        })
    }
    pub(crate) fn resolve(&self, receiving: &Document) -> Result<ArchitectureInputs, String> {
        if self.unavailable.input_count() != 0 {
            return Err("Explicitly discard removed construction inputs before applying".into());
        }
        let mut values = ArchitectureInputs {
            parameters: self.numeric.resolve(&self.definition)?,
            ..Default::default()
        };
        for row in &self.dimensions {
            if row.enabled {
                values
                    .dimensions
                    .insert(row.id, row.field.resolve(receiving)?);
            }
        }
        for row in &self.materials {
            if row.enabled {
                values.materials.insert(
                    row.slot.id,
                    row.value.clone().ok_or_else(|| {
                        format!("{}: choose an available material", row.slot.name)
                    })?,
                );
            }
        }
        for row in &self.options {
            if row.enabled {
                values.options.insert(
                    row.slot.id,
                    row.value
                        .clone()
                        .ok_or_else(|| format!("{}: choose an authored option", row.slot.name))?,
                );
            }
        }
        Ok(values)
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        if !self.numeric.0.is_empty() {
            changed |= self.numeric.show(ui, &self.definition);
        }
        for row in &mut self.dimensions {
            changed |= ui
                .push_id(row.id, |ui| {
                    let mut changed = enable(ui, &row.name, &mut row.enabled, row.retargeted);
                    if row.enabled && !row.retargeted {
                        changed |= row.field.show(
                            ui,
                            &row.name,
                            "Value or receiving-project parameter for this placement",
                        );
                    }
                    ui.small(format!("Authored default: {}", row.default));
                    changed
                })
                .inner;
        }
        for row in &mut self.materials {
            changed |= ui
                .push_id(row.slot.id, |ui| {
                    let mut changed = enable(ui, &row.slot.name, &mut row.enabled, row.retargeted);
                    if row.enabled && !row.retargeted {
                        changed |= choice(ui, &mut row.value, &row.choices);
                    }
                    ui.small(format!("Authored default: {}", row.default));
                    changed
                })
                .inner;
        }
        for row in &mut self.options {
            changed |= ui
                .push_id(row.slot.id, |ui| {
                    let mut changed = enable(ui, &row.slot.name, &mut row.enabled, row.retargeted);
                    if row.enabled && !row.retargeted {
                        changed |= choice(ui, &mut row.value, &row.choices);
                    }
                    ui.small(format!("Authored default: {}", row.default));
                    changed
                })
                .inner;
        }
        let removed = self
            .unavailable
            .dimensions
            .keys()
            .chain(self.unavailable.materials.keys())
            .chain(self.unavailable.options.keys())
            .copied()
            .collect::<Vec<_>>();
        for id in removed {
            let name = self
                .old_names
                .get(&id)
                .map_or("Removed input", String::as_str);
            ui.horizontal(|ui| {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("{name} is no longer exposed"),
                );
                if ui.small_button("Discard override").clicked() {
                    self.unavailable.dimensions.remove(&id);
                    self.unavailable.materials.remove(&id);
                    self.unavailable.options.remove(&id);
                    changed = true;
                }
            });
        }
        if self.numeric.0.is_empty()
            && self.dimensions.is_empty()
            && self.materials.is_empty()
            && self.options.is_empty()
        {
            ui.label("This definition exposes no construction settings");
        }
        changed
    }
}
fn enable(ui: &mut egui::Ui, name: &str, enabled: &mut bool, retargeted: bool) -> bool {
    if retargeted {
        ui.label(name);
        if *enabled {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "This input now controls a different setting",
            );
            if ui.button("Reset for this update").clicked() {
                *enabled = false;
                return true;
            }
        } else {
            ui.small("Reset · configure the new setting after applying the update");
        }
        false
    } else {
        ui.checkbox(enabled, format!("Override {name}")).changed()
    }
}
fn choice<T: Clone + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut Option<T>,
    choices: &[(T, String)],
) -> bool {
    let before = value.clone();
    egui::ComboBox::from_id_salt("choice")
        .width(controls::inspector_width(ui))
        .selected_text(
            choices
                .iter()
                .find(|(v, _)| value.as_ref() == Some(v))
                .map_or("Choose a valid value", |(_, name)| name.as_str()),
        )
        .show_ui(ui, |ui| {
            for (candidate, name) in choices {
                ui.selectable_value(value, Some(candidate.clone()), name);
            }
        });
    before != *value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbols::architecture::test_support;
    #[test]
    fn authored_inputs_start_inherited_and_receiving_formula_identity_is_retained() {
        let (definition, blueprint) = test_support::definition(ArchitectureRole::Fence);
        let (mut host, _) = test_support::consumer(&definition);
        let initial = Fields::new(
            &definition,
            &ArchitectureInputs::default(),
            host.document(),
            None,
        )
        .unwrap();
        assert_eq!(
            initial.resolve(host.document()).unwrap(),
            ArchitectureInputs::default()
        );
        assert_eq!(initial.dimensions.len(), 2);
        let parameter = multicad_core::document::Entity::new(
            "rise",
            EntityData::Parameter {
                expression: "2100".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let prepared = host
            .prepare(multicad_core::commands::Request {
                expected_revision: host.revision(),
                actor: multicad_core::commands::Actor::local(multicad_core::commands::Channel::Ui),
                label: "Add receiving input".into(),
                edits: vec![multicad_core::document::Mutation::Insert(parameter.clone())],
            })
            .unwrap();
        host.commit(prepared).unwrap();
        let bound = multicad_core::constraint_sketch::Dimension::Expression {
            expression: parameters::BoundExpression::bind(host.document(), "=$rise", "mm", None)
                .unwrap(),
        };
        let mut values = ArchitectureInputs::default();
        values
            .dimensions
            .insert(blueprint.dimensions[0].id, bound.clone());
        let fields = Fields::new(&definition, &values, host.document(), None).unwrap();
        assert_eq!(
            fields.resolve(host.document()).unwrap().dimensions[&blueprint.dimensions[0].id],
            bound
        );
        assert_eq!(
            fields.materials[0].choices.len(),
            1,
            "the immutable preset is available with no receiving materials"
        );
    }
    #[test]
    fn removed_and_retargeted_inputs_are_retained_until_an_explicit_user_reset() {
        let (old, blueprint) = test_support::definition(ArchitectureRole::Fence);
        let (host, _) = test_support::consumer(&old);
        let mut values = ArchitectureInputs::default();
        values
            .dimensions
            .insert(blueprint.dimensions[0].id, model::length(2200.));
        let mut next = old.clone();
        let mut interface = blueprint.clone();
        interface.dimensions.clear();
        let mut root = next.template.get(next.root).unwrap().clone();
        root.data = EntityData::Extension {
            definition: interface.definition().unwrap(),
            inputs: interface.inputs(),
        };
        next.template = next
            .template
            .edited(&[multicad_core::document::Mutation::Replace(root)])
            .unwrap();
        let mut fields = Fields::new(&next, &values, host.document(), Some(&old)).unwrap();
        assert!(fields.resolve(host.document()).is_err());
        assert_eq!(fields.unavailable.dimensions, values.dimensions);
        fields.unavailable.dimensions.clear();
        assert!(
            fields
                .resolve(host.document())
                .unwrap()
                .dimensions
                .is_empty()
        );
        interface = blueprint.clone();
        interface.dimensions[0].target = ArchitectureDimensionTarget::FencePostWidth;
        interface.dimensions.truncate(1);
        let mut root = next.template.get(next.root).unwrap().clone();
        root.data = EntityData::Extension {
            definition: interface.definition().unwrap(),
            inputs: interface.inputs(),
        };
        next.template = next
            .template
            .edited(&[multicad_core::document::Mutation::Replace(root)])
            .unwrap();
        let fields = Fields::new(&next, &values, host.document(), Some(&old)).unwrap();
        assert!(fields.dimensions[0].retargeted);
        assert!(fields.dimensions[0].enabled);
        assert_eq!(
            fields.resolve(host.document()).unwrap().dimensions,
            values.dimensions
        );
    }
}
