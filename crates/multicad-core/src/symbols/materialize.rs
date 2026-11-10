use super::{
    source::{encode, entity, instance_links, invalid, override_errors},
    *,
};
use crate::{
    document::{Document, DocumentError, Entity, EntityData, EntityId, Placement},
    model::AssemblyDefinition,
};
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn mapped_source(
    def: &SymbolDefinition,
    ids: &BTreeMap<EntityId, EntityId>,
    overrides: &BTreeMap<EntityId, SymbolValue>,
) -> Result<BTreeMap<EntityId, Entity>, DocumentError> {
    def.validate()?;
    let keys = super::remap::identities(def)?;
    if keys != ids.keys().copied().collect()
        || ids.values().collect::<BTreeSet<_>>().len() != ids.len()
        || ids.values().any(EntityId::is_nil)
    {
        return Err(invalid(
            "Symbol identity map is incomplete, duplicated or malformed",
        ));
    }
    if let Some((_, reason)) = override_errors(def, overrides).into_iter().next() {
        return Err(invalid(reason));
    }
    let mut result = BTreeMap::new();
    for source in def.template.entities() {
        let mut mapped = super::remap::remap(source, ids)?;
        if let EntityData::Parameter {
            expression,
            inputs,
            exposed,
            ..
        } = &mut mapped.data
        {
            *exposed = false;
            if let Some(v) = overrides.get(&source.id) {
                *expression = format!("{} {}", v.value, v.unit);
                inputs.clear();
            }
        }
        result.insert(mapped.id, mapped);
    }
    Ok(result)
}
pub(super) fn generated(
    def: &SymbolDefinition,
    value: &SymbolInstance,
) -> Result<BTreeMap<EntityId, Entity>, DocumentError> {
    let mut result = mapped_source(def, &value.ids, &value.overrides)?;
    if super::architecture::blueprint_for_definition(def)?.is_some() {
        return Err(invalid(
            "Draw a wall or fence to supply the architectural symbol's path and placement context",
        ));
    }
    let source_root = value.ids[&def.root];
    let mut consumed = BTreeSet::new();
    for entity in def.template.entities() {
        if let EntityData::Feature {
            definition,
            suppressed: false,
            ..
        } = &entity.data
            && definition.type_id == crate::model::BOOLEAN_TYPE
        {
            consumed.extend(
                crate::model::BooleanDefinition::from_definition(definition)?.consumed_bodies(),
            );
        }
    }
    // Consumed tools remain as producer metadata, not physical occurrences.
    let parts = def
        .template
        .entities()
        .filter(|e| matches!(e.data, EntityData::Part { .. }) && !consumed.contains(&e.id))
        .map(|e| value.ids[&e.id])
        .collect::<Vec<_>>();
    let assembly = AssemblyDefinition {
        assembly: value.wrapper,
        definition_parts: parts,
    };
    let additions = [
        entity(
            value.member,
            "Symbol content",
            EntityData::Instance {
                component: source_root,
                placement: Placement::default(),
                placement_source: None,
            },
        ),
        entity(
            value.wrapper,
            format!("{} (private definition)", def.name),
            EntityData::Assembly {
                instances: vec![value.member],
            },
        ),
        entity(
            value.wrapper_metadata,
            "Symbol component ownership",
            EntityData::Extension {
                definition: assembly.definition()?,
                inputs: assembly.inputs()?,
            },
        ),
    ];
    for e in additions {
        if result.insert(e.id, e).is_some() {
            return Err(invalid(
                "Symbol wrapper collides with a generated source identity",
            ));
        }
    }
    Ok(result)
}
pub(super) fn record_entity(
    record: EntityId,
    value: &SymbolInstance,
) -> Result<Entity, DocumentError> {
    Ok(entity(
        record,
        "Symbol instance source",
        EntityData::Extension {
            definition: encode(INSTANCE_TYPE, value)?,
            inputs: instance_links(value),
        },
    ))
}
pub fn owned_entities(doc: &Document) -> Result<BTreeSet<EntityId>, DocumentError> {
    let mut ids = super::architecture::private_entities(doc)?;
    for (definition, _) in definitions(doc)? {
        for (_, record) in instances(doc, definition)? {
            let def = read_definition(doc, definition)?;
            ids.extend(
                def.template
                    .entities()
                    .filter_map(|e| record.ids.get(&e.id).copied()),
            );
            ids.extend([record.wrapper, record.wrapper_metadata, record.member]);
        }
    }
    Ok(ids)
}
/// Read ownership from a validated session without reparsing every embedded
/// definition. Session construction and transactional preparation still run
/// full symbol validation, including pins, generated source and the rule that
/// local sketch identities cannot alias canonical document entities.
///
/// Keep `owned_entities` for arbitrary, potentially edited Documents: those do
/// not carry the session's validation guarantee.
pub fn owned_entities_in_session(
    session: &crate::commands::Session,
) -> Result<BTreeSet<EntityId>, DocumentError> {
    let document = session.document();
    let mut owned = super::architecture::private_entities(document)?;
    for entity in document.entities() {
        if matches!(&entity.data, EntityData::Extension { definition, .. } if definition.type_id == INSTANCE_TYPE)
        {
            let record = read_instance(document, entity.id)?;
            owned.extend(
                record
                    .ids
                    .values()
                    .copied()
                    .filter(|id| document.get(*id).is_some()),
            );
            owned.extend([record.wrapper, record.wrapper_metadata, record.member]);
        }
    }
    Ok(owned)
}
pub fn validate(doc: &Document) -> Result<(), DocumentError> {
    super::architecture::validate_blueprints(doc)?;
    let defs = definitions(doc)?;
    let mut revisions = BTreeSet::new();
    let mut owned = BTreeSet::new();
    let mut outers = BTreeSet::new();
    let mut namespaced = BTreeSet::new();
    let mut count = 0;
    super::architecture::validate_instances(doc, &mut namespaced, &mut owned, &mut count)?;
    for (id, def) in &defs {
        if !revisions.insert((def.symbol, def.revision.clone())) {
            return Err(invalid(
                "A symbol revision must have exactly one embedded definition",
            ));
        }
        let pin = def.pin()?;
        for (record, value) in instances(doc, *id)? {
            count += 1;
            if count > MAX_SYMBOL_INSTANCES {
                return Err(invalid("Embedded symbol instance budget exceeded"));
            }
            if value.pin != pin {
                return Err(invalid(
                    "Symbol instance pin differs from its embedded definition; use explicit update",
                ));
            }
            if !outers.insert(value.instance)
                || [
                    record,
                    value.instance,
                    value.wrapper,
                    value.wrapper_metadata,
                    value.member,
                ]
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                    != 5
            {
                return Err(invalid(
                    "Symbol instance/wrapper identities must be distinct and uniquely owned",
                ));
            }
            for id in value.ids.values().copied().chain([
                record,
                value.instance,
                value.wrapper,
                value.wrapper_metadata,
                value.member,
            ]) {
                if !namespaced.insert(id) {
                    return Err(invalid(
                        "Symbol placements must have disjoint private identity namespaces",
                    ));
                }
            }
            for (source, mapped) in &value.ids {
                if def.template.get(*source).is_none() && doc.get(*mapped).is_some() {
                    return Err(invalid(
                        "Local symbol identities cannot alias canonical document entities",
                    ));
                }
            }
            let Some(EntityData::Instance { component, .. }) =
                doc.get(value.instance).map(|e| &e.data)
            else {
                return Err(invalid("Symbol outer Instance is missing"));
            };
            if *component != value.wrapper {
                return Err(invalid(
                    "Symbol outer Instance must retain its owned component",
                ));
            }
            if doc.entities().any(|e|e.id!=value.instance&&matches!(&e.data,EntityData::Instance{component,..}if *component==value.wrapper)){
                return Err(invalid("A symbol copy requires its own private source graph and ownership record; use Move/Copy or place it from the Library"));
            }
            for (entity_id, expected) in generated(def, &value)? {
                if !owned.insert(entity_id) || doc.get(entity_id) != Some(&expected) {
                    return Err(invalid(format!(
                        "Symbol-owned entity {entity_id} was independently edited, removed or reused; edit the symbol overrides or definition instead"
                    )));
                }
            }
        }
    }
    super::project::validate_sources(doc)?;
    // A malformed/dangling ownership record cannot disappear from a per-definition list.
    for e in doc.entities() {
        if matches!(&e.data,EntityData::Extension{definition,..}if definition.type_id==INSTANCE_TYPE)
        {
            let v = read_instance(doc, e.id)?;
            if !defs.iter().any(|(id, _)| *id == v.definition) {
                return Err(invalid("Symbol instance embedded definition is missing"));
            }
        }
    }
    Ok(())
}
