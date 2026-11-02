//! Author-selected interface over the existing graphical construction source.
use crate::egui;
use multicad_core::{
    document::{Document, EntityData, EntityId},
    model::*,
    symbols::architecture::*,
};
struct DimensionChoice {
    enabled: bool,
    slot: ArchitectureDimensionSlot,
}
struct MaterialChoice {
    enabled: bool,
    slot: ArchitectureMaterialSlot,
}
struct OptionChoice {
    enabled: bool,
    slot: ArchitectureOptionSlot,
    available: Vec<(ArchitectureOptionValue, String)>,
}
pub(crate) struct Interface {
    dimensions: Vec<DimensionChoice>,
    materials: Vec<MaterialChoice>,
    options: Vec<OptionChoice>,
    material_names: Vec<(EntityId, String)>,
}
fn short_name(value: &str) -> String {
    let mut value = value.to_string();
    while value.len() > 128 {
        value.pop();
    }
    value
}
fn dimension(
    name: impl Into<String>,
    target: ArchitectureDimensionTarget,
    enabled: bool,
) -> DimensionChoice {
    DimensionChoice {
        enabled,
        slot: ArchitectureDimensionSlot {
            id: EntityId::new(),
            name: name.into(),
            target,
        },
    }
}
fn material(
    name: impl Into<String>,
    target: ArchitectureMaterialTarget,
    default: EntityId,
) -> MaterialChoice {
    MaterialChoice {
        enabled: false,
        slot: ArchitectureMaterialSlot {
            id: EntityId::new(),
            name: name.into(),
            target,
            choices: vec![default],
            allow_project_materials: false,
        },
    }
}
pub(crate) fn construction_role(document: &Document, id: EntityId) -> Option<ArchitectureRole> {
    match &document.get(id)?.data {
        EntityData::Extension { definition, .. }
            if definition.type_id == WALL_TYPE_KIND
                && definition.schema_version == LAYERED_WALL_TYPE_SCHEMA =>
        {
            Some(ArchitectureRole::Wall)
        }
        EntityData::Extension { definition, .. }
            if definition.type_id == FENCE_TYPE_KIND
                && definition.schema_version == FENCE_SCHEMA =>
        {
            Some(ArchitectureRole::Fence)
        }
        _ => None,
    }
}
impl Interface {
    pub(crate) fn new(
        document: &Document,
        construction: EntityId,
    ) -> Result<(Self, ArchitectureBlueprint), String> {
        let role = construction_role(document, construction)
            .ok_or("Choose an authored layered wall or fence construction")?;
        let source = ArchitectureConstruction::read(document, role, construction)
            .map_err(|e| e.to_string())?;
        let mut dimensions = Vec::new();
        let mut materials = Vec::new();
        let mut options = Vec::new();
        match source {
            ArchitectureConstruction::Wall(wall) => {
                dimensions.push(dimension(
                    "Height",
                    ArchitectureDimensionTarget::WallHeight,
                    true,
                ));
                let mut references = vec![
                    (
                        ArchitectureOptionValue::WallReference {
                            reference: WallLayerReference::Outside,
                        },
                        "Outside".into(),
                    ),
                    (
                        ArchitectureOptionValue::WallReference {
                            reference: WallLayerReference::Center,
                        },
                        "Center".into(),
                    ),
                    (
                        ArchitectureOptionValue::WallReference {
                            reference: WallLayerReference::Inside,
                        },
                        "Inside".into(),
                    ),
                ];
                for (index, layer) in wall.layers.layers.iter().enumerate() {
                    let name = format!("Layer {} · {}", index + 1, short_name(&layer.name));
                    if layer.construction.thickness().is_some() {
                        dimensions.push(dimension(
                            format!("{name} thickness"),
                            ArchitectureDimensionTarget::WallLayerThickness { layer: layer.id },
                            false,
                        ));
                    }
                    if let Some(value) = layer.construction.material() {
                        materials.push(material(
                            format!("{name} material"),
                            ArchitectureMaterialTarget::WallLayer { layer: layer.id },
                            value,
                        ));
                    }
                    for (side, label) in [
                        (WallLayerSide::Outside, "outside"),
                        (WallLayerSide::Inside, "inside"),
                    ] {
                        references.push((
                            ArchitectureOptionValue::WallReference {
                                reference: WallLayerReference::Boundary {
                                    layer: layer.id,
                                    side,
                                },
                            },
                            format!("{name} · {label} edge"),
                        ));
                    }
                }
                let mut choices = references
                    .iter()
                    .take(3)
                    .map(|(v, _)| v.clone())
                    .collect::<Vec<_>>();
                let inherited = ArchitectureOptionValue::WallReference {
                    reference: wall.reference,
                };
                if !choices.contains(&inherited) {
                    choices.push(inherited);
                }
                options.push(OptionChoice {
                    enabled: false,
                    slot: ArchitectureOptionSlot {
                        id: EntityId::new(),
                        name: "Path reference".into(),
                        target: ArchitectureOptionTarget::WallReference,
                        choices,
                    },
                    available: references,
                });
            }
            ArchitectureConstruction::Fence(fence) => {
                dimensions.extend([
                    dimension("Height", ArchitectureDimensionTarget::FenceHeight, true),
                    dimension(
                        "Post spacing",
                        ArchitectureDimensionTarget::FenceSpacing,
                        true,
                    ),
                ]);
                match fence.post {
                    FencePostProfile::Rectangular { .. } => dimensions.extend([
                        dimension(
                            "Post width",
                            ArchitectureDimensionTarget::FencePostWidth,
                            false,
                        ),
                        dimension(
                            "Post depth",
                            ArchitectureDimensionTarget::FencePostDepth,
                            false,
                        ),
                    ]),
                    FencePostProfile::Round { .. } => dimensions.push(dimension(
                        "Post diameter",
                        ArchitectureDimensionTarget::FencePostDiameter,
                        false,
                    )),
                }
                materials.push(material(
                    "Post material",
                    ArchitectureMaterialTarget::FencePost,
                    fence.post_material,
                ));
                for (index, recipe) in fence.infills.iter().enumerate() {
                    let name = format!("Infill {} · {}", index + 1, short_name(&recipe.name));
                    dimensions.extend([
                        dimension(
                            format!("{name} bottom"),
                            ArchitectureDimensionTarget::FenceInfillBottom { recipe: recipe.id },
                            false,
                        ),
                        dimension(
                            format!("{name} height"),
                            ArchitectureDimensionTarget::FenceInfillHeight { recipe: recipe.id },
                            false,
                        ),
                        dimension(
                            format!("{name} offset"),
                            ArchitectureDimensionTarget::FenceInfillOffset { recipe: recipe.id },
                            false,
                        ),
                    ]);
                    materials.push(material(
                        format!("{name} material"),
                        ArchitectureMaterialTarget::FenceInfill { recipe: recipe.id },
                        recipe.material,
                    ));
                    match &recipe.construction {
                        FenceInfillKind::Panel { width, .. } => {
                            dimensions.push(dimension(
                                format!("{name} thickness"),
                                ArchitectureDimensionTarget::FencePanelThickness {
                                    recipe: recipe.id,
                                },
                                false,
                            ));
                            if matches!(width, FencePanelWidth::Purchased { .. }) {
                                dimensions.push(dimension(
                                    format!("{name} purchased width"),
                                    ArchitectureDimensionTarget::FencePurchasedPanelWidth {
                                        recipe: recipe.id,
                                    },
                                    false,
                                ));
                                let available = vec![
                                    (
                                        ArchitectureOptionValue::FenceShortPanel {
                                            policy: FenceShortPanelPolicy::Reject,
                                        },
                                        "Reject short bays".into(),
                                    ),
                                    (
                                        ArchitectureOptionValue::FenceShortPanel {
                                            policy: FenceShortPanelPolicy::Cut,
                                        },
                                        "Cut shorter panels".into(),
                                    ),
                                ];
                                options.push(OptionChoice {
                                    enabled: false,
                                    slot: ArchitectureOptionSlot {
                                        id: EntityId::new(),
                                        name: format!("{name} short bays"),
                                        target: ArchitectureOptionTarget::FenceShortPanel {
                                            recipe: recipe.id,
                                        },
                                        choices: available.iter().map(|(v, _)| v.clone()).collect(),
                                    },
                                    available,
                                });
                            }
                        }
                        FenceInfillKind::Rail { .. } => dimensions.push(dimension(
                            format!("{name} depth"),
                            ArchitectureDimensionTarget::FenceRailDepth { recipe: recipe.id },
                            false,
                        )),
                        FenceInfillKind::Pickets { .. } => dimensions.extend([
                            dimension(
                                format!("{name} width"),
                                ArchitectureDimensionTarget::FencePicketWidth { recipe: recipe.id },
                                false,
                            ),
                            dimension(
                                format!("{name} depth"),
                                ArchitectureDimensionTarget::FencePicketDepth { recipe: recipe.id },
                                false,
                            ),
                            dimension(
                                format!("{name} gap"),
                                ArchitectureDimensionTarget::FencePicketGap { recipe: recipe.id },
                                false,
                            ),
                        ]),
                    }
                }
                let available = vec![
                    (
                        ArchitectureOptionValue::FenceLayout {
                            layout: FenceBayLayout::Equal,
                        },
                        "Equal bays".into(),
                    ),
                    (
                        ArchitectureOptionValue::FenceLayout {
                            layout: FenceBayLayout::ShortLast,
                        },
                        "Short last bay".into(),
                    ),
                ];
                options.push(OptionChoice {
                    enabled: true,
                    slot: ArchitectureOptionSlot {
                        id: EntityId::new(),
                        name: "Bay layout".into(),
                        target: ArchitectureOptionTarget::FenceLayout,
                        choices: available.iter().map(|(v, _)| v.clone()).collect(),
                    },
                    available,
                });
            }
        }
        let material_names = document
            .entities()
            .filter(|e| matches!(e.data, EntityData::Material { .. }))
            .map(|e| (e.id, e.name.clone()))
            .collect();
        let value = Self {
            dimensions,
            materials,
            options,
            material_names,
        };
        let mut blueprint = ArchitectureBlueprint::new(role, construction);
        value.write(&mut blueprint);
        Ok((value, blueprint))
    }
    pub(crate) fn restore(&mut self, blueprint: &ArchitectureBlueprint) {
        for row in &mut self.dimensions {
            row.enabled = false;
            if let Some(slot) = blueprint
                .dimensions
                .iter()
                .find(|s| s.target == row.slot.target)
            {
                row.slot = slot.clone();
                row.enabled = true;
            }
        }
        for row in &mut self.materials {
            row.enabled = false;
            if let Some(slot) = blueprint
                .materials
                .iter()
                .find(|s| s.target == row.slot.target)
            {
                row.slot = slot.clone();
                row.enabled = true;
            }
        }
        for row in &mut self.options {
            row.enabled = false;
            if let Some(slot) = blueprint
                .options
                .iter()
                .find(|s| s.target == row.slot.target)
            {
                row.slot = slot.clone();
                row.enabled = true;
            }
        }
    }
    pub(crate) fn write(&self, blueprint: &mut ArchitectureBlueprint) {
        blueprint.dimensions = self
            .dimensions
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.slot.clone())
            .collect();
        blueprint.materials = self
            .materials
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.slot.clone())
            .collect();
        blueprint.options = self
            .options
            .iter()
            .filter(|r| r.enabled)
            .map(|r| r.slot.clone())
            .collect();
    }
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        blueprint: &mut ArchitectureBlueprint,
    ) -> bool {
        let mut changed = false;
        ui.label("Receiving projects draw their own path and choose level/base placement. Check the construction settings they may override.");
        ui.strong("Dimensions");
        for row in &mut self.dimensions {
            changed |= ui.checkbox(&mut row.enabled, &row.slot.name).changed();
            if row.enabled {
                ui.indent(row.slot.id, |ui| {
                    ui.label("Input name");
                    changed |= ui.text_edit_singleline(&mut row.slot.name).changed();
                });
            }
        }
        ui.strong("Materials");
        for row in &mut self.materials {
            changed |= ui.checkbox(&mut row.enabled, &row.slot.name).changed();
            if row.enabled {
                ui.indent(row.slot.id, |ui| {
                    ui.label("Input name");
                    changed |= ui.text_edit_singleline(&mut row.slot.name).changed();
                    changed |= ui
                        .checkbox(
                            &mut row.slot.allow_project_materials,
                            "Allow receiving-project materials",
                        )
                        .changed();
                    ui.label("Authored choices");
                    for (id, name) in &self.material_names {
                        let mut enabled = row.slot.choices.contains(id);
                        if ui.checkbox(&mut enabled, name).changed() {
                            if enabled {
                                row.slot.choices.push(*id);
                            } else {
                                row.slot.choices.retain(|v| v != id);
                            }
                            changed = true;
                        }
                    }
                });
            }
        }
        ui.strong("Options");
        for row in &mut self.options {
            changed |= ui.checkbox(&mut row.enabled, &row.slot.name).changed();
            if row.enabled {
                ui.indent(row.slot.id, |ui| {
                    ui.label("Input name");
                    changed |= ui.text_edit_singleline(&mut row.slot.name).changed();
                    for (value, name) in &row.available {
                        let mut enabled = row.slot.choices.contains(value);
                        if ui.checkbox(&mut enabled, name).changed() {
                            if enabled {
                                row.slot.choices.push(value.clone());
                            } else {
                                row.slot.choices.retain(|v| v != value);
                            }
                            changed = true;
                        }
                    }
                });
            }
        }
        if changed {
            self.write(blueprint);
        }
        ui.small(format!("{} public settings", blueprint.input_count()));
        changed
    }
}
