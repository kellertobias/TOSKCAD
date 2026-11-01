use super::*;
use crate::{parameters::EvaluatedParameters, symbols};

#[derive(Clone, Debug)]
pub struct ResolvedEquipment {
    pub record: EntityId,
    pub instance: EntityId,
    pub template: Template,
    pub size_mm: [f64; 3],
    pub settings: InstanceSettings,
    pub attachment: Option<(EntityId, Attachment)>,
}
/// Source inspection remains available when a host is missing. Resolving the
/// attachment pose is a separate operation that reports the required repair.
pub fn resolve(
    document: &Document,
    parameters: &EvaluatedParameters,
    record: EntityId,
) -> Result<Option<ResolvedEquipment>, DocumentError> {
    let symbol = symbols::read_instance(document, record)?;
    let source = symbols::read_definition(document, symbol.definition)?;
    let Some(template) = template_for(&source.template, source.root)? else {
        return Ok(None);
    };
    let size_mm = [
        template.width_parameter,
        template.depth_parameter,
        template.height_parameter,
    ]
    .map(|id| {
        parameters
            .value(symbol.ids[&id], Unit::Millimeter)
            .map_err(|e| invalid(e.to_string()))
    });
    let [w, d, h] = size_mm;
    let settings = document
        .entities()
        .find_map(|e| match &e.data {
            EntityData::Extension { definition, .. } if definition.type_id == INSTANCE_TYPE => {
                InstanceSettings::from_definition(definition)
                    .ok()
                    .filter(|s| s.symbol_instance == record)
            }
            _ => None,
        })
        .unwrap_or(InstanceSettings {
            symbol_instance: record,
            installation: if template.supports_standalone {
                Installation::Standalone
            } else {
                Installation::BuiltIn
            },
            label: None,
            service_envelope: None,
            door_envelope: None,
            service_visible: false,
            door_visible: false,
        });
    let attachment = match document.get(symbol.instance).map(|e| &e.data) {
        Some(EntityData::Instance { placement_source: Some(id), .. }) if document.get(*id).is_some_and(|e| matches!(&e.data, EntityData::Extension { definition, .. } if definition.type_id == ATTACHMENT_TYPE)) => Some((*id, read_attachment(document, *id)?)),
        _ => None,
    };
    Ok(Some(ResolvedEquipment {
        record,
        instance: symbol.instance,
        template,
        size_mm: [w?, d?, h?],
        settings,
        attachment,
    }))
}
