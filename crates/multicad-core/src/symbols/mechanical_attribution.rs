//! Source-only inspection of a placement draft against its immutable pin.
use super::{SymbolDefinition, SymbolValue, source};
use crate::{
    document::{DocumentError, EntityData, EntityId},
    materials, model, parameters,
    project_file::ContentHash,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct ConfiguredHardwarePart {
    pub feature: EntityId,
    pub name: String,
    pub family: &'static str,
    pub current: model::ResolvedMechanicalPart,
    /// Literal catalog geometry when sourced, otherwise the immutable template defaults.
    pub retained: model::ResolvedMechanicalPart,
    pub attribution: model::MechanicalPartAttribution,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PinnedHardwareBaseline {
    pub pin: ContentHash,
    pub revision: String,
    pub material: Option<String>,
    pub geometry: model::ResolvedMechanicalPart,
}

/// Find the exact template for a current mapped hardware feature. A part name,
/// a matching designation or a shared definition alone cannot establish ownership.
pub fn pinned_hardware_baseline(
    document: &crate::document::Document,
    part: EntityId,
    feature: EntityId,
) -> Result<Option<PinnedHardwareBaseline>, DocumentError> {
    let mut result = None;
    for record in document.entities().filter(|entity| {
        matches!(&entity.data,
        EntityData::Extension { definition, .. } if definition.type_id == super::INSTANCE_TYPE)
    }) {
        let instance = super::read_instance(document, record.id)?;
        let Some(template_part) = instance
            .ids
            .iter()
            .find_map(|(id, mapped)| (*mapped == part).then_some(*id))
        else {
            continue;
        };
        let template_feature = instance
            .ids
            .iter()
            .find_map(|(id, mapped)| (*mapped == feature).then_some(*id))
            .ok_or_else(|| source::invalid("Hardware feature is missing from its Library pin"))?;
        if result.is_some() {
            return Err(source::invalid(
                "Hardware belongs to more than one Library pin",
            ));
        }
        let definition = super::read_definition(document, instance.definition)?;
        if definition.pin()? != instance.pin {
            return Err(source::invalid(
                "Hardware Library pin does not match the embedded definition",
            ));
        }
        let template = model::read_mechanical_part(&definition.template, template_feature)?;
        if template.output.body != template_part {
            return Err(source::invalid(
                "The pinned hardware feature belongs to another Part",
            ));
        }
        let evaluated = parameters::evaluate(&definition.template)
            .map_err(|error| source::invalid(error.to_string()))?;
        let material = materials::resolve(&definition.template, template_part)?
            .material
            .and_then(|id| definition.template.get(id))
            .map(|entity| entity.name.clone());
        result = Some(PinnedHardwareBaseline {
            pin: instance.pin,
            revision: definition.revision,
            material,
            geometry: template.parameters.resolve(&evaluated)?,
        });
    }
    Ok(result)
}

/// Read draft dimensions without changing the pin, placing a Part or granting
/// a native preview receipt. Numeric overrides use the same validation as
/// symbol placement.
pub fn configured_hardware_parts(
    definition: &SymbolDefinition,
    overrides: &BTreeMap<EntityId, SymbolValue>,
) -> Result<Vec<ConfiguredHardwarePart>, DocumentError> {
    let template = &definition.template;
    let features: Vec<_> = template
        .entities()
        .filter(|entity| matches!(&entity.data,
            EntityData::Feature { definition, .. } if definition.type_id == model::MECHANICAL_PART_TYPE))
        .collect();
    if features.is_empty() {
        return Ok(Vec::new());
    }
    definition.validate()?;
    if let Some((_, reason)) = source::override_errors(definition, overrides)
        .into_iter()
        .next()
    {
        return Err(source::invalid(reason));
    }
    let defaults =
        parameters::evaluate(template).map_err(|error| source::invalid(error.to_string()))?;
    let configured = if overrides.is_empty() {
        None
    } else {
        let mut candidate = template.clone();
        for (id, value) in overrides {
            let mut entity = template
                .get(*id)
                .ok_or(DocumentError::Missing(*id))?
                .clone();
            let EntityData::Parameter {
                expression, inputs, ..
            } = &mut entity.data
            else {
                return Err(source::invalid(
                    "A hardware override must target a numeric parameter",
                ));
            };
            *expression = format!("{} {}", value.value, value.unit);
            inputs.clear();
            candidate.replace(entity)?;
        }
        Some(parameters::evaluate(&candidate).map_err(|error| source::invalid(error.to_string()))?)
    };
    let evaluated = configured.as_ref().unwrap_or(&defaults);
    // Numeric overrides cannot change assignments. The effective pinned
    // assignment is also the draft's assignment; catalog text is separate.
    let materials = materials::Resolver::new(template)?;
    features
        .into_iter()
        .map(|entity| {
            let part = model::read_mechanical_part(template, entity.id)?;
            let current = part.parameters.resolve(evaluated)?;
            let retained = part
                .provenance
                .as_ref()
                .map_or(&part.parameters, |reference| &reference.reference_geometry)
                .resolve(&defaults)?;
            let material = materials
                .resolve(part.output.body)?
                .material
                .and_then(|id| template.get(id))
                .map(|entity| entity.name.as_str());
            let mut attribution = part.attribution(evaluated, material, material)?;
            if attribution.reference.is_none() && current != retained {
                attribution.basis = model::MechanicalAttributionBasis::CustomDerived;
            }
            Ok(ConfiguredHardwarePart {
                feature: entity.id,
                name: entity.name.clone(),
                family: part.parameters.label(),
                current,
                retained,
                attribution,
            })
        })
        .collect()
}
