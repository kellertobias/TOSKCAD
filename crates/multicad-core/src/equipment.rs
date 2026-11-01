//! Apartment equipment semantics share the pinned symbol and canonical
//! placement model. Catalog defaults never stand in for manufacturer data.
mod preset;
pub use preset::generic_definition;
mod prepare;
mod resolve;
pub use prepare::{
    EnvelopeSource, EquipmentEditRequest, EquipmentPlaceRequest, check_mount, prepare_edit,
    prepare_place,
};
pub use resolve::{ResolvedEquipment, resolve};
mod attachment;
pub(crate) use attachment::attachment_base;
pub use attachment::{
    ATTACHMENT_CAPABILITY, ATTACHMENT_TYPE, Attachment, CeilingDatum, HostPlacement,
    ResolvedAttachment, WallSide, read_attachment,
};
mod catalog;
use crate::{
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link},
    parameters::Unit,
};
pub use catalog::{
    Category, Installation, Kind, MountClass, PlanConvention, PowerConnector, Preset,
};
use serde::{Deserialize, Serialize};
pub const TEMPLATE_TYPE: &str = "architecture.equipment_template";
pub const INSTANCE_TYPE: &str = "architecture.equipment_instance";
pub const CAPABILITY: &str = "architecture.equipment.1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub component: EntityId,
    pub kind: Kind,
    pub width_parameter: EntityId,
    pub depth_parameter: EntityId,
    pub height_parameter: EntityId,
    /// True-size physical footprint, also the symbol definition's linked plan.
    pub footprint: EntityId,
    /// Optional normalized authored electrical mark, separate from footprint.
    pub mark: Option<EntityId>,
    pub convention: PlanConvention,
    pub power_connector: Option<PowerConnector>,
    pub mark_paper_mm: Option<f64>,
    pub mount: MountClass,
    pub supports_built_in: bool,
    pub supports_standalone: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceSettings {
    pub symbol_instance: EntityId,
    pub installation: Installation,
    pub label: Option<String>,
    pub service_envelope: Option<EntityId>,
    pub door_envelope: Option<EntityId>,
    pub service_visible: bool,
    pub door_visible: bool,
}
fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
fn encode<T: Serialize>(kind: &str, value: &T) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: kind.into(),
        schema_version: 1,
        data: serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?,
    })
}
fn decode<T: serde::de::DeserializeOwned>(
    source: &Definition,
    kind: &str,
) -> Result<T, DocumentError> {
    if source.type_id != kind || source.schema_version != 1 {
        return Err(invalid("Unsupported apartment equipment source/version"));
    }
    serde_json::from_value(source.data.clone()).map_err(|e| invalid(e.to_string()))
}
fn links_match(actual: &[Link], expected: &[Link]) -> Result<(), DocumentError> {
    if actual.len() != expected.len() || expected.iter().any(|l| !actual.contains(l)) {
        return Err(invalid(
            "Equipment source links disagree with its typed payload",
        ));
    }
    Ok(())
}
fn entity(id: EntityId, name: &str, definition: Definition, inputs: Vec<Link>) -> Entity {
    let mut value = Entity::new(name, EntityData::Extension { definition, inputs });
    value.id = id;
    value
}
impl Template {
    pub fn links(&self) -> Vec<Link> {
        let mut values = vec![
            Link::new(self.component, "component"),
            Link::new(self.width_parameter, "width"),
            Link::new(self.depth_parameter, "depth"),
            Link::new(self.height_parameter, "height"),
            Link::new(self.footprint, "footprint"),
        ];
        if let Some(id) = self.mark {
            values.push(Link::new(id, "mark"));
        }
        values
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        let ids = self
            .links()
            .into_iter()
            .map(|l| l.target)
            .collect::<Vec<_>>();
        if ids.iter().any(EntityId::is_nil)
            || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
        {
            return Err(invalid(
                "Equipment component, dimensions and plan sources require distinct non-nil identities",
            ));
        }
        match (self.mark, self.mark_paper_mm) {
            (None, None) => {}
            (Some(_), Some(size)) if size.is_finite() && (1.0..=20.).contains(&size) => {}
            _ => {
                return Err(invalid(
                    "An equipment paper mark requires an authored sketch and a bounded explicit paper size",
                ));
            }
        }
        if self.power_connector.is_some()
            && self.kind.preset().category != Category::ElectricalLighting
        {
            return Err(invalid(
                "Power connector convention belongs only to electrical equipment",
            ));
        }
        if !self.supports_built_in && !self.supports_standalone {
            return Err(invalid(
                "Equipment type requires a supported installation mode",
            ));
        }
        encode(TEMPLATE_TYPE, self)
    }
    pub fn from_definition(source: &Definition) -> Result<Self, DocumentError> {
        let value: Self = decode(source, TEMPLATE_TYPE)?;
        value.definition()?;
        Ok(value)
    }
    pub fn entity(&self, id: EntityId) -> Result<Entity, DocumentError> {
        Ok(entity(
            id,
            "Equipment template",
            self.definition()?,
            self.links(),
        ))
    }
    pub fn check_document(&self, document: &Document) -> Result<(), DocumentError> {
        if !matches!(
            document.get(self.component).map(|e| &e.data),
            Some(EntityData::Part { .. } | EntityData::Assembly { .. })
        ) {
            return Err(invalid("Equipment template requires a Part or assembly"));
        }
        for id in [
            self.width_parameter,
            self.depth_parameter,
            self.height_parameter,
        ] {
            if !matches!(document.get(id).map(|e|&e.data),Some(EntityData::Parameter{unit,..}) if Unit::parse(unit).is_some_and(|u|u.dimension()==Unit::Millimeter.dimension()))
            {
                return Err(invalid(
                    "Equipment dimensions require canonical length parameters",
                ));
            }
        }
        for id in std::iter::once(self.footprint).chain(self.mark) {
            if !matches!(
                document.get(id).map(|e| &e.data),
                Some(EntityData::Sketch { .. })
            ) {
                return Err(invalid("Equipment plans require canonical sketches"));
            }
        }
        Ok(())
    }
}
impl InstanceSettings {
    pub fn links(&self) -> Vec<Link> {
        let mut links = vec![Link::new(self.symbol_instance, "symbol_instance")];
        for (id, role) in [
            (self.service_envelope, "service_envelope"),
            (self.door_envelope, "door_envelope"),
        ] {
            if let Some(id) = id {
                links.push(Link::new(id, role));
            }
        }
        links
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        if self.symbol_instance.is_nil()
            || self.label.as_ref().is_some_and(|s| {
                s.trim().is_empty() || s.chars().count() > 200 || s.chars().any(char::is_control)
            })
            || self.service_envelope.is_some_and(|id| id.is_nil())
            || self.door_envelope.is_some_and(|id| id.is_nil())
            || self.service_envelope.is_some() && self.service_envelope == self.door_envelope
        {
            return Err(invalid(
                "Invalid equipment ownership, label or envelope identities",
            ));
        }
        encode(INSTANCE_TYPE, self)
    }
    pub fn from_definition(source: &Definition) -> Result<Self, DocumentError> {
        let value: Self = decode(source, INSTANCE_TYPE)?;
        value.definition()?;
        Ok(value)
    }
    pub fn entity(&self, id: EntityId) -> Result<Entity, DocumentError> {
        Ok(entity(
            id,
            "Equipment instance settings",
            self.definition()?,
            self.links(),
        ))
    }
}
pub fn template_for(
    document: &Document,
    component: EntityId,
) -> Result<Option<Template>, DocumentError> {
    let mut found = None;
    for entity in document.entities() {
        if let EntityData::Extension { definition, inputs } = &entity.data
            && definition.type_id == TEMPLATE_TYPE
        {
            let value = Template::from_definition(definition)?;
            links_match(inputs, &value.links())?;
            if value.component == component {
                if found.is_some() {
                    return Err(invalid("Duplicate equipment template ownership"));
                }
                value.check_document(document)?;
                found = Some(value);
            }
        }
    }
    Ok(found)
}
pub fn validate(document: &Document) -> Result<(), DocumentError> {
    let mut components = std::collections::BTreeSet::new();
    let mut instances = std::collections::BTreeSet::new();
    for entity in document.entities() {
        if let EntityData::Extension { definition, inputs } = &entity.data {
            match definition.type_id.as_str() {
                ATTACHMENT_TYPE => {
                    read_attachment(document, entity.id)?;
                }
                TEMPLATE_TYPE => {
                    let value = Template::from_definition(definition)?;
                    links_match(inputs, &value.links())?;
                    value.check_document(document)?;
                    if !components.insert(value.component) {
                        return Err(invalid("Duplicate equipment template ownership"));
                    }
                }
                INSTANCE_TYPE => {
                    let value = InstanceSettings::from_definition(definition)?;
                    links_match(inputs, &value.links())?;
                    if !matches!(document.get(value.symbol_instance).map(|e|&e.data),Some(EntityData::Extension{definition,..}) if definition.type_id==crate::symbols::INSTANCE_TYPE)
                    {
                        return Err(invalid(
                            "Equipment settings require the canonical symbol ownership record",
                        ));
                    }
                    let instance = crate::symbols::read_instance(document, value.symbol_instance)?;
                    let source = crate::symbols::read_definition(document, instance.definition)?;
                    let template = template_for(&source.template, source.root)?
                        .ok_or_else(|| invalid("Equipment settings require an equipment symbol"))?;
                    if value.installation == Installation::Standalone
                        && !template.supports_standalone
                    {
                        return Err(invalid(
                            "This equipment type does not support standalone placement",
                        ));
                    }
                    if value.installation == Installation::BuiltIn && !template.supports_built_in {
                        return Err(invalid(
                            "This equipment type does not support built-in placement",
                        ));
                    }
                    if !instances.insert(value.symbol_instance) {
                        return Err(invalid("Duplicate equipment instance settings"));
                    }
                    for id in [value.service_envelope, value.door_envelope]
                        .into_iter()
                        .flatten()
                    {
                        if crate::model::read_envelope(document, id)?.target != instance.instance {
                            return Err(invalid(
                                "Equipment envelope belongs to another placed object",
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
pub fn required_capability(document: &Document) -> bool {
    let direct = |d: &Document| {
        d.entities().any(|e|matches!(&e.data,EntityData::Extension{definition,..} if matches!(definition.type_id.as_str(),TEMPLATE_TYPE|INSTANCE_TYPE)))
    };
    direct(document)
        || crate::symbols::definitions(document)
            .is_ok_and(|defs| defs.iter().any(|(_, d)| direct(&d.template)))
}
/// Display visibility only. Hidden access envelopes still participate in the
/// explicit geometric clearance check.
pub fn hidden_envelopes(
    document: &Document,
) -> Result<std::collections::BTreeSet<EntityId>, DocumentError> {
    let mut hidden = std::collections::BTreeSet::new();
    for e in document.entities() {
        if let EntityData::Extension { definition, .. } = &e.data
            && definition.type_id == INSTANCE_TYPE
        {
            let settings = InstanceSettings::from_definition(definition)?;
            if !settings.service_visible {
                hidden.extend(settings.service_envelope);
            }
            if !settings.door_visible {
                hidden.extend(settings.door_envelope);
            }
        }
    }
    Ok(hidden)
}
