use super::{
    materialize::{generated, record_entity},
    source::invalid,
    *,
};
use crate::document::{Document, DocumentError, Entity, EntityId, Mutation, Placement};
use std::collections::BTreeMap;
pub(crate) type OwnedSymbolCopy = (EntityId, Vec<Mutation>, BTreeMap<EntityId, EntityId>);
/// Clone private source in the same transaction as the ordinary placement copy.
/// The ordinary placement module retains authority over the outer pose/driver.
pub(crate) fn copy_owned_variant(
    doc: &Document,
    source: EntityId,
    outer: EntityId,
    name: &str,
) -> Result<Option<OwnedSymbolCopy>, DocumentError> {
    let mut selected = None;
    for (id, _) in definitions(doc)? {
        for (record, value) in instances(doc, id)? {
            if value.instance == source {
                if selected.is_some() {
                    return Err(invalid("Symbol source has ambiguous ownership"));
                }
                selected = Some((record, value));
            }
        }
    }
    let Some((old_record, old)) = selected else {
        return Ok(None);
    };
    let def = read_definition(doc, old.definition)?;
    let mut request = SymbolPlaceRequest::new(
        doc,
        old.definition,
        name,
        old.overrides.clone(),
        Placement::default(),
    )?;
    request.instance = outer;
    let value = SymbolInstance {
        definition: old.definition,
        pin: def.pin()?,
        instance: outer,
        wrapper: request.wrapper,
        wrapper_metadata: request.wrapper_metadata,
        member: request.member,
        ids: request.ids,
        overrides: request.overrides,
    };
    let mut rekey = old
        .ids
        .iter()
        .map(|(source, mapped)| (*mapped, value.ids[source]))
        .collect::<BTreeMap<_, _>>();
    rekey.extend([
        (old.instance, outer),
        (old.wrapper, value.wrapper),
        (old.member, value.member),
        (old.wrapper_metadata, value.wrapper_metadata),
        (old_record, request.record),
    ]);
    let mut edits = generated(&def, &value)?
        .into_values()
        .map(Mutation::Insert)
        .collect::<Vec<_>>();
    edits.push(Mutation::Insert(record_entity(request.record, &value)?));
    Ok(Some((value.wrapper, edits, rekey)))
}
pub(crate) fn rekey_copied_driver(
    entity: &Entity,
    map: &BTreeMap<EntityId, EntityId>,
) -> Result<Entity, DocumentError> {
    let crate::document::EntityData::Extension { definition, .. } = &entity.data else {
        return Err(invalid("Copied symbol pose contains a non-driver entity"));
    };
    match definition.type_id.as_str() {
        crate::model::INSTANCE_PLACEMENT_TYPE | crate::model::INSTANCE_ALIGNMENT_TYPE => {
            let mut rekey = map.clone();
            // Driver identity was allocated by the existing placement copier.
            rekey.insert(entity.id, entity.id);
            super::remap::remap(entity, &rekey)
        }
        _ => Err(invalid(
            "This symbol placement driver cannot yet be copied with private semantic inputs",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constraint_sketch::Dimension,
        document::EntityData,
        model::{
            AlignmentAnchor, AlignmentInput, AlignmentParameters, InstanceAlignmentDefinition,
            PlacementBase,
        },
        parameters::{BoundExpression, ParameterBinding},
        references::{GeometryIntent, Origin, TopologyKind},
    };
    use std::collections::BTreeSet;
    #[test]
    fn typed_alignment_rekey_preserves_global_targets_and_new_chain_ids() {
        let owner = EntityId::new();
        let driver = EntityId::new();
        let previous = EntityId::new();
        let old_feature = EntityId::new();
        let new_feature = EntityId::new();
        let old_curve = EntityId::new();
        let new_curve = EntityId::new();
        let old_parameter = EntityId::new();
        let new_parameter = EntityId::new();
        let global = EntityId::new();
        let moving = GeometryIntent {
            id: EntityId::new(),
            kind: TopologyKind::Face,
            origins: BTreeSet::from([Origin {
                source: old_feature,
                role: format!("curve/{old_curve}/side"),
            }]),
            boundary: None,
            attachment: None,
        };
        let target = GeometryIntent {
            id: EntityId::new(),
            kind: TopologyKind::Face,
            origins: BTreeSet::from([Origin {
                source: global,
                role: "face/top".into(),
            }]),
            boundary: None,
            attachment: None,
        };
        let mut parameters = AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: moving.id,
            },
            target: AlignmentAnchor::Geometry {
                reference: target.id,
            },
            ..Default::default()
        };
        parameters.offset[0] = Dimension::Expression {
            expression: BoundExpression {
                text: "=$private".into(),
                unit: "mm".into(),
                bindings: vec![ParameterBinding {
                    alias: "private".into(),
                    target: old_parameter,
                }],
            },
        };
        let source = InstanceAlignmentDefinition {
            owner,
            base: PlacementBase::Previous { driver: previous },
            parameters,
        };
        let refs = vec![
            AlignmentInput {
                source: old_feature,
                intent: moving,
            },
            AlignmentInput {
                source: global,
                intent: target.clone(),
            },
        ];
        let mut entity = Entity::new(
            "Copied alignment",
            EntityData::Extension {
                definition: source.definition().unwrap(),
                inputs: source.inputs(&refs).unwrap(),
            },
        );
        entity.id = driver;
        let changed = rekey_copied_driver(
            &entity,
            &BTreeMap::from([
                (old_feature, new_feature),
                (old_curve, new_curve),
                (old_parameter, new_parameter),
            ]),
        )
        .unwrap();
        assert_eq!(changed.id, driver);
        let EntityData::Extension { definition, inputs } = changed.data else {
            unreachable!()
        };
        let changed = InstanceAlignmentDefinition::from_definition(&definition).unwrap();
        assert_eq!(changed.owner, owner);
        assert_eq!(changed.base, PlacementBase::Previous { driver: previous });
        assert_eq!(
            changed.parameters.offset[0].expression().unwrap().bindings[0].target,
            new_parameter
        );
        let moving = inputs
            .iter()
            .find(|l| l.target == new_feature)
            .unwrap()
            .geometry
            .as_ref()
            .unwrap();
        assert_eq!(
            moving.origins,
            BTreeSet::from([Origin {
                source: new_feature,
                role: format!("curve/{new_curve}/side")
            }])
        );
        assert!(
            inputs
                .iter()
                .any(|l| l.target == global && l.geometry.as_ref() == Some(&target))
        );
    }
}
