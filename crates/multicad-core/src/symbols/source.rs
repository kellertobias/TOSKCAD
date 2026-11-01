use super::*;
use crate::{
    document::{Definition, Document, DocumentError, Entity, EntityData, EntityId, Link, Mutation},
    parameters,
    project_file::ContentHash,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
pub(super) fn invalid(message: impl Into<String>) -> DocumentError {
    DocumentError::Invalid(message.into())
}
pub(super) fn entity(id: EntityId, name: impl Into<String>, data: EntityData) -> Entity {
    Entity {
        id,
        name: name.into(),
        description: String::new(),
        data,
    }
}
pub(super) fn encode<T: serde::Serialize>(
    type_id: &str,
    value: &T,
) -> Result<Definition, DocumentError> {
    Ok(Definition {
        type_id: type_id.into(),
        schema_version: 1,
        data: serde_json::to_value(value).map_err(|e| invalid(e.to_string()))?,
    })
}
pub(super) fn decode<T: serde::de::DeserializeOwned>(
    definition: &Definition,
    kind: &str,
) -> Result<T, DocumentError> {
    if definition.type_id != kind || definition.schema_version != 1 {
        return Err(invalid("Unsupported symbol provider/version"));
    }
    serde_json::from_value(definition.data.clone())
        .map_err(|e| invalid(format!("Invalid symbol source: {e}")))
}
impl SymbolDefinition {
    pub fn pin(&self) -> Result<ContentHash, DocumentError> {
        self.validate()?;
        Ok(ContentHash::of(
            &serde_json::to_vec(self).map_err(|e| invalid(e.to_string()))?,
        ))
    }
    pub fn definition(&self) -> Result<Definition, DocumentError> {
        self.validate()?;
        encode(DEFINITION_TYPE, self)
    }
    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.symbol.is_nil()
            || self.revision.trim().is_empty()
            || self.revision.len() > 256
            || self.name.trim().is_empty()
            || self.name.len() > 256
        {
            return Err(invalid("Symbol needs valid identity, name and revision"));
        }
        if self.template.entities().count() > MAX_SYMBOL_ENTITIES
            || self.template.entities().count() == 0
            || self.exposed.len() > MAX_SYMBOL_PARAMETERS
        {
            return Err(invalid("Symbol source/exposure exceeds its budget"));
        }
        if !matches!(
            self.template.get(self.root).map(|e| &e.data),
            Some(EntityData::Part { .. } | EntityData::Assembly { .. })
        ) {
            return Err(invalid(
                "A symbol root must be an authored Part or Assembly",
            ));
        }
        if self.exposed.iter().collect::<BTreeSet<_>>().len() != self.exposed.len() {
            return Err(invalid("Exposed symbol parameters must be unique"));
        }
        for id in &self.exposed {
            if !matches!(
                self.template.get(*id).map(|e| &e.data),
                Some(EntityData::Parameter { .. })
            ) {
                return Err(invalid(
                    "Exposed symbol parameters must exist in its template",
                ));
            }
        }
        if let Some(plan) = &self.plan
            && !matches!(
                self.template.get(plan.sketch).map(|e| &e.data),
                Some(EntityData::Sketch { .. })
            )
        {
            return Err(invalid("Symbol plan must be a captured constrained sketch"));
        }
        if let Some(plant) = crate::plants::template_for(&self.template, self.root)?
            && self.plan.as_ref().map(|p| p.sketch) != Some(plant.plan)
        {
            return Err(invalid(
                "Plant definition and its linked plan do not share one source",
            ));
        }
        if let Some(equipment) = crate::equipment::template_for(&self.template, self.root)?
            && self.plan.as_ref().map(|p| p.sketch) != Some(equipment.footprint)
        {
            return Err(invalid(
                "Equipment definition must retain its true-size footprint as its linked plan",
            ));
        }
        for e in self.template.entities() {
            super::remap::validate_entity(e)?;
        }
        if !self.template.reference_diagnostics().is_empty() {
            return Err(invalid(
                "Symbol definition has missing or incompatible source references",
            ));
        }
        parameters::evaluate(&self.template).map_err(|e| invalid(e.to_string()))?;
        Ok(())
    }
}
pub fn read_definition(doc: &Document, id: EntityId) -> Result<SymbolDefinition, DocumentError> {
    let Some(EntityData::Extension { definition, inputs }) = doc.get(id).map(|e| &e.data) else {
        return Err(invalid("Select an embedded symbol definition"));
    };
    if !inputs.is_empty() {
        return Err(invalid(
            "An embedded symbol definition cannot depend on external project entities",
        ));
    }
    // Reject recursive embedded providers before deserializing their Documents.
    if let Some(values) = definition
        .data
        .get("template")
        .and_then(|v| v.get("entities"))
        .and_then(|v| v.as_object())
        && values.values().any(|e| {
            e.pointer("/data/definition/type_id")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.starts_with("symbols."))
        })
    {
        return Err(invalid(
            "Nested embedded symbol definitions are unsupported; author their canonical Part or Assembly instead",
        ));
    }
    let value: SymbolDefinition = decode(definition, DEFINITION_TYPE)?;
    value.validate()?;
    Ok(value)
}
pub fn definitions(doc: &Document) -> Result<Vec<(EntityId, SymbolDefinition)>, DocumentError> {
    doc.entities().filter(|e|matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==DEFINITION_TYPE)).map(|e|read_definition(doc,e.id).map(|d|(e.id,d))).collect()
}
pub(super) fn instance_links(value: &SymbolInstance) -> Vec<Link> {
    let mut links = vec![
        Link::new(value.definition, "definition"),
        Link::new(value.instance, "instance"),
        Link::new(value.wrapper, "wrapper"),
        Link::new(value.wrapper_metadata, "wrapper_metadata"),
        Link::new(value.member, "root_member"),
    ];
    // Only canonical entity mappings are dependencies; local sketch identities
    // are validated within their provider and do not denote Document entities.
    links.sort_by(|a, b| a.role.cmp(&b.role));
    links
}
pub fn read_instance(doc: &Document, id: EntityId) -> Result<SymbolInstance, DocumentError> {
    let record = if matches!(
        doc.get(id).map(|e| &e.data),
        Some(EntityData::Instance { .. })
    ) {
        let values = doc
            .entities()
            .filter_map(|e| match &e.data {
                EntityData::Extension { definition, .. } if definition.type_id == INSTANCE_TYPE => {
                    decode::<SymbolInstance>(definition, INSTANCE_TYPE)
                        .ok()
                        .filter(|s| s.instance == id)
                        .map(|_| e.id)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if values.len() != 1 {
            return Err(invalid(
                "The selected Instance does not have one symbol ownership record",
            ));
        }
        values[0]
    } else {
        id
    };
    let Some(EntityData::Extension { definition, inputs }) = doc.get(record).map(|e| &e.data)
    else {
        return Err(invalid("Select a symbol Instance or ownership record"));
    };
    let value: SymbolInstance = decode(definition, INSTANCE_TYPE)?;
    if *inputs != instance_links(&value) {
        return Err(invalid(
            "Symbol instance canonical inputs disagree with its ownership record",
        ));
    }
    Ok(value)
}
pub fn instances(
    doc: &Document,
    definition: EntityId,
) -> Result<Vec<(EntityId, SymbolInstance)>, DocumentError> {
    doc.entities().filter(|e|matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==INSTANCE_TYPE)).map(|e|read_instance(doc,e.id).map(|v|(e.id,v))).filter(|v|v.as_ref().is_err() || v.as_ref().is_ok_and(|(_,s)|s.definition==definition)).collect()
}
pub(super) fn author(
    doc: &Document,
    request: &SymbolAuthorRequest,
) -> Result<SymbolDefinition, DocumentError> {
    let bound = parameters::evaluate(doc)
        .map_err(|e| invalid(e.to_string()))?
        .document;
    let mut queue = VecDeque::from([request.root]);
    queue.extend(request.exposed.iter().copied());
    if let Some(plan) = &request.plan {
        queue.push_back(plan.sketch);
    }
    let mut component_metadata: std::collections::BTreeMap<EntityId, Vec<EntityId>> =
        std::collections::BTreeMap::new();
    for metadata in bound.entities() {
        if let EntityData::Extension { definition, .. } = &metadata.data {
            let component = match definition.type_id.as_str() {
                crate::plants::TEMPLATE_TYPE => {
                    Some(crate::plants::PlantTemplate::from_definition(definition)?.component)
                }
                crate::equipment::TEMPLATE_TYPE => {
                    Some(crate::equipment::Template::from_definition(definition)?.component)
                }
                _ => None,
            };
            if let Some(component) = component {
                component_metadata
                    .entry(component)
                    .or_default()
                    .push(metadata.id);
            }
        }
    }
    let mut keep = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if !keep.insert(id) {
            continue;
        }
        if keep.len() > MAX_SYMBOL_ENTITIES {
            return Err(invalid(
                "Symbol source dependency closure exceeds its budget",
            ));
        }
        let e = bound.get(id).ok_or(DocumentError::Missing(id))?;
        super::remap::validate_entity(e)?;
        queue.extend(e.references().iter().map(|l| l.target));
        if matches!(
            e.data,
            EntityData::Part { .. } | EntityData::Assembly { .. }
        ) && let Some(metadata) = component_metadata.get(&id)
        {
            queue.extend(metadata);
        }
        if matches!(e.data, EntityData::Assembly { .. }) {
            for m in bound.entities() {
                if let EntityData::Extension { definition, .. } = &m.data
                    && definition.type_id == crate::model::ASSEMBLY_TYPE
                    && crate::model::read_assembly(&bound, m.id)?.assembly == id
                {
                    queue.push_back(m.id);
                }
            }
        }
        if matches!(
            e.data,
            EntityData::Part { .. } | EntityData::Material { .. }
        ) {
            for metadata in bound.entities() {
                let owner = match crate::materials::read(metadata)? {
                    Some(crate::materials::LibraryDefinition::MaterialStyle(style)) => {
                        Some(style.material)
                    }
                    Some(crate::materials::LibraryDefinition::PartProperties(properties)) => {
                        Some(properties.part)
                    }
                    _ => None,
                };
                if owner == Some(id) {
                    queue.push_back(metadata.id);
                }
            }
        }
    }
    let template = Document::new(&request.name)?.edited(
        &keep
            .iter()
            .map(|id| Mutation::Insert(bound.get(*id).unwrap().clone()))
            .collect::<Vec<_>>(),
    )?;
    let mut wire = serde_json::to_value(&template).map_err(|e| invalid(e.to_string()))?;
    wire["id"] = serde_json::to_value(bound.id()).map_err(|e| invalid(e.to_string()))?;
    let template: Document = serde_json::from_value(wire).map_err(|e| invalid(e.to_string()))?;
    let value = SymbolDefinition {
        symbol: request.symbol,
        revision: request.revision.clone(),
        name: request.name.clone(),
        kind: request.kind,
        template,
        root: request.root,
        exposed: request.exposed.clone(),
        plan: request.plan.clone(),
    };
    value.validate()?;
    Ok(value)
}
pub(super) fn override_errors(
    def: &SymbolDefinition,
    values: &BTreeMap<EntityId, SymbolValue>,
) -> Vec<(EntityId, String)> {
    values
        .iter()
        .filter_map(|(id, value)| {
            let reason = if def.kind == SymbolKind::Static {
                Some("Static symbols do not accept parameter overrides".into())
            } else if !def.exposed.contains(id) {
                Some("Parameter is no longer exposed by this definition".into())
            } else {
                match def.template.get(*id).map(|e| &e.data) {
                    Some(EntityData::Parameter { unit, .. }) => match (
                        parameters::Unit::parse(unit),
                        parameters::Unit::parse(&value.unit),
                    ) {
                        (Some(a), Some(b))
                            if a.dimension() == b.dimension() && value.value.is_finite() =>
                        {
                            None
                        }
                        _ => Some("Override requires a finite value in compatible units".into()),
                    },
                    _ => Some("Exposed parameter is missing".into()),
                }
            };
            reason.map(|r| (*id, r))
        })
        .collect()
}

/// Resolve an instance's authored simplified plan. Consumers obtain the actual
/// world pose from the current native occurrence receipt, including later moves.
pub fn symbol_plan(
    doc: &Document,
    id: EntityId,
) -> Result<Option<(EntityId, EntityId)>, DocumentError> {
    let instance = read_instance(doc, id)?;
    let def = read_definition(doc, instance.definition)?;
    Ok(def
        .plan
        .map(|plan| (instance.instance, instance.ids[&plan.sketch])))
}
