//! Hardware attribution attached to the physical rows of a current native BOM.
use crate::{ModelError, ModelSnapshot, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    materials,
    model::{self, OccurrencePath, ResolvedMechanicalPart},
    parameters::{self, EvaluatedParameters},
    project_file::ContentHash,
    references::ReferenceFrame,
    symbols::{self, SymbolDefinition, SymbolInstance},
};
use multicad_kernel::Bounds;
use std::collections::{BTreeMap, btree_map::Entry};

#[derive(Clone, Debug, PartialEq)]
pub struct BomHardwarePin {
    pub record: EntityId,
    pub instance: EntityId,
    pub definition: EntityId,
    pub symbol: EntityId,
    pub revision: String,
    pub pin: ContentHash,
    pub template_part: EntityId,
    pub template_feature: EntityId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BomHardwareRole {
    pub connection: EntityId,
    pub role_id: EntityId,
    pub role: model::ScrewConnectionHardwareRole,
    pub instance: EntityId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BomHardwareOccurrence {
    pub path: OccurrencePath,
    /// Current world mounting datum when the hardware recipe is the final producer.
    /// Later operations may move or remove that datum.
    pub mount: Option<ReferenceFrame>,
    pub connection: Option<BomHardwareRole>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BomHardware {
    pub feature: EntityId,
    pub family: String,
    /// Current resolved construction values. Later machining can change the final solid.
    pub resolved_recipe: ResolvedMechanicalPart,
    pub attribution: model::MechanicalPartAttribution,
    /// Literal geometry of the retained catalog reference.
    pub reference_recipe: Option<ResolvedMechanicalPart>,
    pub library: Option<BomHardwarePin>,
    pub final_producer: EntityId,
    /// Native final shape measurements in the component's local coordinates.
    pub final_bounds: Bounds,
    pub final_volume_mm3: f64,
    pub final_solid_count: u32,
    pub occurrences: Vec<BomHardwareOccurrence>,
}

pub(super) fn source_feature(
    document: &Document,
    snapshot: &ModelSnapshot,
    part: EntityId,
    producer: EntityId,
) -> Option<EntityId> {
    let EntityData::Part { features, .. } = &document.get(part)?.data else {
        return None;
    };
    let mut head = features.iter().position(|id| *id == producer)?;
    let target_role = format!("target/{part}/producer");
    let body_role = format!("body/{part}/producer");
    loop {
        let feature = features[head];
        if !snapshot.shapes.contains_key(&feature) {
            return None;
        }
        let EntityData::Feature {
            definition,
            inputs,
            suppressed: false,
        } = &document.get(feature)?.data
        else {
            return None;
        };
        if definition.type_id == model::MECHANICAL_PART_TYPE {
            return Some(feature);
        }
        // Native compilation has validated each provider's canonical inputs.
        // Follow only a unique retained-solid link within this Part's earlier
        // history. Tool, sketch, face and parameter inputs cannot transfer its
        // hardware identity to a replacement solid.
        let mut predecessors = inputs
            .iter()
            .filter(|link| {
                matches!(
                    link.role.as_str(),
                    "source" | "prior_solid" | "receiver_producer"
                ) || link.role == target_role
                    || link.role == body_role
            })
            .filter_map(|link| features[..head].iter().position(|id| *id == link.target));
        let predecessor = predecessors.next()?;
        if predecessors.next().is_some() {
            return None;
        }
        head = predecessor;
    }
}

struct RoleSource {
    role: BomHardwareRole,
    binding: model::ScrewConnectionHardwareBinding,
}

pub(super) struct HardwareIndex<'a> {
    document: &'a Document,
    snapshot: &'a ModelSnapshot,
    evaluated: EvaluatedParameters,
    part_owners: BTreeMap<EntityId, (EntityId, EntityId)>,
    instances: BTreeMap<EntityId, SymbolInstance>,
    definitions: BTreeMap<EntityId, SymbolDefinition>,
    template_parameters: BTreeMap<EntityId, EvaluatedParameters>,
    template_materials: BTreeMap<(EntityId, EntityId), Option<String>>,
    roles: BTreeMap<EntityId, RoleSource>,
}

impl<'a> HardwareIndex<'a> {
    pub(super) fn new(
        document: &'a Document,
        snapshot: &'a ModelSnapshot,
    ) -> Result<Self, ModelError> {
        let mut part_owners = BTreeMap::new();
        let mut instances = BTreeMap::new();
        for entity in document.entities() {
            if matches!(&entity.data, EntityData::Extension { definition, .. }
                if definition.type_id == symbols::INSTANCE_TYPE)
            {
                let instance = symbols::read_instance(document, entity.id)
                    .map_err(|e| error(Some(entity.id), e))?;
                for (template, mapped) in &instance.ids {
                    if matches!(
                        document.get(*mapped).map(|entity| &entity.data),
                        Some(EntityData::Part { .. })
                    ) && part_owners
                        .insert(*mapped, (entity.id, *template))
                        .is_some()
                    {
                        return Err(error(
                            Some(*mapped),
                            "Physical hardware has multiple Library owners",
                        ));
                    }
                }
                instances.insert(entity.id, instance);
            }
        }
        let mut roles = BTreeMap::new();
        for check in &snapshot.screw_connection_checks {
            let connection = model::read_screw_connection(document, check.connection)
                .map_err(|e| error(Some(check.connection), e))?;
            for row in connection.hardware {
                let role = BomHardwareRole {
                    connection: check.connection,
                    role_id: row.role_id,
                    role: row.role,
                    instance: row.instance,
                };
                if roles
                    .insert(
                        row.instance,
                        RoleSource {
                            role,
                            binding: row.source,
                        },
                    )
                    .is_some()
                {
                    return Err(error(
                        Some(row.instance),
                        "Physical hardware has multiple connection roles",
                    ));
                }
            }
        }
        Ok(Self {
            document,
            snapshot,
            evaluated: parameters::evaluate(document).map_err(|e| error(None, e))?,
            part_owners,
            instances,
            definitions: BTreeMap::new(),
            template_parameters: BTreeMap::new(),
            template_materials: BTreeMap::new(),
            roles,
        })
    }

    fn pin(
        &mut self,
        part: EntityId,
        feature: EntityId,
    ) -> Result<(Option<BomHardwarePin>, Option<String>), ModelError> {
        let Some(&(record, template_part)) = self.part_owners.get(&part) else {
            return Ok((None, None));
        };
        let instance = &self.instances[&record];
        let definition = match self.definitions.entry(instance.definition) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(
                symbols::read_definition(self.document, instance.definition)
                    .map_err(|e| error(Some(record), e))?,
            ),
        };
        let Some(EntityData::Part { features, .. }) =
            definition.template.get(template_part).map(|e| &e.data)
        else {
            return Err(error(
                Some(record),
                "Hardware Part does not match its pinned Library template",
            ));
        };
        let template_feature = features
            .iter()
            .copied()
            .find(|id| instance.ids.get(id) == Some(&feature))
            .ok_or_else(|| {
                error(
                    Some(record),
                    "Hardware feature does not match its pinned Library template",
                )
            })?;
        let template_material = match self
            .template_materials
            .entry((instance.definition, template_part))
        {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let resolved = materials::Resolver::new(&definition.template)
                    .and_then(|resolver| resolver.resolve(template_part))
                    .map_err(|e| error(Some(record), e))?;
                let name = resolved
                    .material
                    .and_then(|id| definition.template.get(id))
                    .map(|e| e.name.clone());
                entry.insert(name).clone()
            }
        };
        let pin = BomHardwarePin {
            record,
            instance: instance.instance,
            definition: instance.definition,
            symbol: definition.symbol,
            revision: definition.revision.clone(),
            pin: instance.pin,
            template_part,
            template_feature,
        };
        Ok((Some(pin), template_material))
    }

    pub(super) fn read(
        &mut self,
        part: EntityId,
        feature: EntityId,
        final_producer: EntityId,
        actual_material: Option<&str>,
        occurrences: &[OccurrencePath],
    ) -> Result<BomHardware, ModelError> {
        let source = model::read_mechanical_part(self.document, feature)
            .map_err(|e| error(Some(feature), e))?;
        if source.output.body != part {
            return Err(error(
                Some(feature),
                "Hardware source belongs to another Part",
            ));
        }
        let (library, template_material) = self.pin(part, feature)?;
        let mut attribution = source
            .attribution(
                &self.evaluated,
                actual_material,
                template_material.as_deref(),
            )
            .map_err(|e| error(Some(feature), e))?;
        let resolved_recipe = source
            .parameters
            .resolve(&self.evaluated)
            .map_err(|e| error(Some(feature), e))?;
        let reference_recipe = source
            .provenance
            .as_ref()
            .map(|reference| {
                reference
                    .reference_geometry
                    .resolve(&self.evaluated)
                    .map_err(|e| error(Some(feature), e))
            })
            .transpose()?;
        if let Some(pin) = library.as_ref().filter(|_| attribution.reference.is_none()) {
            let definition = &self.definitions[&pin.definition];
            let parameters = match self.template_parameters.entry(pin.definition) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(
                    parameters::evaluate(&definition.template)
                        .map_err(|e| error(Some(pin.definition), e))?,
                ),
            };
            let template = model::read_mechanical_part(&definition.template, pin.template_feature)
                .map_err(|e| error(Some(pin.definition), e))?;
            let expected = template
                .parameters
                .resolve(parameters)
                .map_err(|e| error(Some(pin.definition), e))?;
            if resolved_recipe != expected {
                attribution.basis = model::MechanicalAttributionBasis::CustomDerived;
            }
        }
        let mount = resolved_recipe.mount_pose();
        let occurrences = occurrences
            .iter()
            .map(|path| {
                let leaf = &self.snapshot.leaf_occurrences[path];
                let owner = std::iter::once(&path.root)
                    .chain(&path.members)
                    .find_map(|id| self.roles.get(id));
                let connection = owner
                    .map(|owner| {
                        let belongs = match owner.binding {
                            model::ScrewConnectionHardwareBinding::Generated {
                                body,
                                feature: expected,
                            } => body == part && expected == feature,
                            model::ScrewConnectionHardwareBinding::Library { record } => {
                                library.as_ref().is_some_and(|pin| pin.record == record)
                            }
                        };
                        if !belongs {
                            return Err(error(
                                Some(part),
                                "Physical hardware does not match its canonical connection role",
                            ));
                        }
                        Ok(owner.role.clone())
                    })
                    .transpose()?;
                let mount = (feature == final_producer)
                    .then(|| -> Result<ReferenceFrame, ModelError> {
                        let world = mount.then(leaf.pose).map_err(|e| error(Some(part), e))?;
                        Ok(ReferenceFrame {
                            origin_mm: world.translation_mm,
                            u: world.vector([1., 0., 0.]),
                            v: world.vector([0., 1., 0.]),
                        })
                    })
                    .transpose()?;
                Ok(BomHardwareOccurrence {
                    path: path.clone(),
                    mount,
                    connection,
                })
            })
            .collect::<Result<Vec<_>, ModelError>>()?;
        let shape = self
            .snapshot
            .shapes
            .get(&final_producer)
            .ok_or_else(|| error(Some(part), "Final hardware geometry is unavailable"))?;
        Ok(BomHardware {
            feature,
            family: source.parameters.label().into(),
            resolved_recipe,
            attribution,
            reference_recipe,
            library,
            final_producer,
            final_bounds: shape.bounds,
            final_volume_mm3: shape.volume_mm3,
            final_solid_count: shape.solid_count,
            occurrences,
        })
    }
}

impl BomHardware {
    pub(super) fn schedule_cells(&self) -> [String; 3] {
        let mut basis = self.attribution.basis.label().to_owned();
        if self.feature != self.final_producer {
            basis.push_str(" · later feature");
        }
        let reference = self
            .attribution
            .reference
            .as_ref()
            .map(|reference| {
                let mut text = format!("{} · r{}", reference.designation, reference.revision);
                if let Some(recipe) = &self.reference_recipe {
                    text.push_str(" · ");
                    text.push_str(&recipe_label(recipe));
                }
                if let Some(material) = &reference.material {
                    text.push_str(" · ");
                    text.push_str(material);
                }
                text
            })
            .unwrap_or_default();
        // The full source strings remain in the record. Drawing-table cells
        // have a bounded length and may share this abbreviated display.
        let reference = if reference.chars().count() > 96 {
            reference.chars().take(95).chain(['…']).collect()
        } else {
            reference
        };
        [basis, recipe_label(&self.resolved_recipe), reference]
    }
}

fn recipe_label(part: &ResolvedMechanicalPart) -> String {
    use super::mm;
    use model::{
        ResolvedCustomConnectionHardware as Custom, ResolvedScrewDrive as Drive,
        ResolvedScrewHead as Head,
    };
    let thread = |thread: &model::ResolvedThread| {
        format!(
            "Ø{} × P{}",
            mm(2. * thread.nominal_major_radius_mm),
            mm(thread.pitch_mm)
        )
    };
    match part {
        ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            ..
        } => format!(
            "Washer Ø{}/{} × {} mm",
            mm(2. * bore_radius_mm),
            mm(2. * outside_radius_mm),
            mm(*thickness_mm)
        ),
        ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            ..
        } => format!(
            "Bearing Ø{}/{} × {} mm",
            mm(2. * bore_radius_mm),
            mm(2. * outside_radius_mm),
            mm(*width_mm)
        ),
        ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread: t,
        } => format!(
            "Nut {} · AF{} × {} mm",
            thread(t),
            mm(*across_flats_mm),
            mm(t.length_mm)
        ),
        ResolvedMechanicalPart::MachineScrew {
            underhead_length_mm,
            head,
            drive,
            thread: t,
            ..
        } => {
            let head = match head {
                Head::SocketCap { .. } => "Cap",
                Head::ExternalHex { .. } => "Hex",
                Head::Pan { .. } => "Pan",
                Head::Button { .. } => "Button",
                Head::Countersunk { .. } => "Countersunk",
            };
            let drive = match drive {
                Drive::HexSocket { .. } => "hex socket",
                Drive::ExternalHex => "external hex",
                Drive::Hexalobular { .. } => "hexalobular",
                Drive::PhillipsH { .. } => "Phillips H",
            };
            format!(
                "{head}/{drive} {} × {} mm underhead",
                thread(t),
                mm(*underhead_length_mm)
            )
        }
        ResolvedMechanicalPart::WoodScrew { parameters } => format!(
            "Wood screw envelope Ø{} × {} mm underhead",
            mm(parameters.nominal_diameter_mm),
            mm(parameters.underhead_length_mm)
        ),
        ResolvedMechanicalPart::CustomConnection { parameters } => match parameters.as_ref() {
            Custom::WoodScrew {
                wood_thread,
                plain_length_mm,
                ..
            } => format!(
                "Wood screw {} × {} mm underhead",
                thread(wood_thread),
                mm(wood_thread.length_mm + plain_length_mm)
            ),
            Custom::WoodInsert {
                outer_thread,
                inner_thread,
                ..
            } => format!(
                "Wood insert outer {} × {} mm · inner {}",
                thread(outer_thread),
                mm(outer_thread.length_mm),
                thread(inner_thread)
            ),
            Custom::HangerBolt {
                wood_thread,
                metric_thread,
                ..
            } => format!(
                "Hanger wood {} × {} mm · metric {} × {} mm",
                thread(wood_thread),
                mm(wood_thread.length_mm),
                thread(metric_thread),
                mm(metric_thread.length_mm)
            ),
        },
    }
}
