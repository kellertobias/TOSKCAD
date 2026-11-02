use super::{
    materialize::{generated, record_entity},
    source::{author, entity, invalid, override_errors},
    *,
};
use crate::{
    commands::{Actor, CommandError, ErrorCode, Request, Session},
    document::{Document, DocumentError, EntityData, EntityId, Mutation, Placement},
};
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn stale(session: &Session, revision: u64) -> Result<(), CommandError> {
    if session.revision() != revision {
        Err(CommandError {
            code: ErrorCode::StaleRevision,
            message: "Symbol request uses an outdated document revision".into(),
            involved: vec![],
            parameter: None,
        })
    } else {
        Ok(())
    }
}
pub(super) fn finish(
    session: &Session,
    revision: u64,
    actor: Actor,
    label: &str,
    edits: Vec<Mutation>,
    definition: EntityId,
    values: &[SymbolInstance],
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    let prepared = session.prepare(Request {
        expected_revision: revision,
        actor,
        label: label.into(),
        edits,
    })?;
    super::validate(prepared.document())?;
    let def = read_definition(prepared.document(), definition)?;
    Ok(SymbolPrepared {
        prepared,
        definition,
        instances: values.iter().map(|v| v.instance).collect(),
        plans: values
            .iter()
            .filter_map(|v| def.plan.as_ref().map(|p| (v.instance, v.ids[&p.sketch])))
            .collect(),
    })
}
impl SymbolPlaceRequest {
    pub fn new(
        doc: &Document,
        definition: EntityId,
        name: impl Into<String>,
        overrides: BTreeMap<EntityId, SymbolValue>,
        placement: Placement,
    ) -> Result<Self, DocumentError> {
        let def = read_definition(doc, definition)?;
        if super::architecture::blueprint_for_definition(&def)?.is_some() {
            return Err(invalid(
                "Draw a wall or fence to supply the architectural symbol's path and placement context",
            ));
        }
        Ok(Self {
            record: EntityId::new(),
            definition,
            instance: EntityId::new(),
            wrapper: EntityId::new(),
            wrapper_metadata: EntityId::new(),
            member: EntityId::new(),
            ids: super::remap::identities(&def)?
                .into_iter()
                .map(|id| (id, EntityId::new()))
                .collect(),
            name: name.into(),
            overrides,
            placement,
        })
    }
}
impl SymbolUpdateRequest {
    pub fn new(
        doc: &Document,
        definition: EntityId,
        replacement: SymbolDefinition,
    ) -> Result<Self, DocumentError> {
        validate_replacement(doc, definition, &replacement)?;
        let keys = super::remap::identities(&replacement)?;
        let mut maps = BTreeMap::new();
        let mut instance_overrides = BTreeMap::new();
        for (record, old) in instances(doc, definition)? {
            maps.insert(
                record,
                keys.iter()
                    .map(|id| (*id, old.ids.get(id).copied().unwrap_or_default()))
                    .collect(),
            );
            instance_overrides.insert(record, old.overrides);
        }
        let mut architecture_inputs = BTreeMap::new();
        for (record, old) in architecture::architecture_instances(doc, definition)? {
            maps.insert(
                record,
                keys.iter()
                    .map(|id| (*id, old.ids.get(id).copied().unwrap_or_default()))
                    .collect(),
            );
            architecture_inputs.insert(record, old.inputs);
        }
        Ok(Self {
            definition,
            replacement,
            maps,
            instance_overrides,
            architecture_inputs,
            architecture_contexts: BTreeMap::new(),
        })
    }
}
fn embed(
    session: &Session,
    revision: u64,
    actor: Actor,
    value: &SymbolDefinition,
    id: EntityId,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    value.validate()?;
    let pin = value.pin()?;
    for (existing, old) in definitions(session.document())? {
        if old.symbol == value.symbol && old.pin()? == pin {
            return finish(
                session,
                revision,
                actor,
                "Reuse pinned symbol definition",
                vec![],
                existing,
                &[],
            );
        }
        if old.symbol == value.symbol && old.revision == value.revision {
            return Err(invalid("Symbol revision collision: the same symbol/revision has different content; author a new revision explicitly").into());
        }
    }
    finish(
        session,
        revision,
        actor,
        "Embed pinned symbol definition",
        vec![Mutation::Insert(entity(
            id,
            value.name.clone(),
            EntityData::Extension {
                definition: value.definition()?,
                inputs: vec![],
            },
        ))],
        id,
        &[],
    )
}
pub fn prepare_import(
    session: &Session,
    revision: u64,
    actor: Actor,
    value: &SymbolDefinition,
) -> Result<SymbolPrepared, CommandError> {
    embed(session, revision, actor, value, EntityId::new())
}
/// Retain a composite transaction's chosen import identity across previews.
pub(crate) fn prepare_import_at(
    session: &Session,
    revision: u64,
    actor: Actor,
    value: &SymbolDefinition,
    id: EntityId,
) -> Result<SymbolPrepared, CommandError> {
    embed(session, revision, actor, value, id)
}
pub fn prepare_author(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &SymbolAuthorRequest,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    let value = author(session.document(), request)?;
    embed(session, revision, actor, &value, request.definition)
}
pub fn prepare_place(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &SymbolPlaceRequest,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    super::validate(session.document())?;
    let def = read_definition(session.document(), request.definition)?;
    crate::model::RigidPose::from_placement(request.placement)?;
    let value = SymbolInstance {
        definition: request.definition,
        pin: def.pin()?,
        instance: request.instance,
        wrapper: request.wrapper,
        wrapper_metadata: request.wrapper_metadata,
        member: request.member,
        ids: request.ids.clone(),
        overrides: request.overrides.clone(),
    };
    let mut edits = generated(&def, &value)?
        .into_values()
        .map(Mutation::Insert)
        .collect::<Vec<_>>();
    edits.push(Mutation::Insert(entity(
        request.instance,
        request.name.clone(),
        EntityData::Instance {
            component: request.wrapper,
            placement: request.placement,
            placement_source: None,
        },
    )));
    edits.push(Mutation::Insert(record_entity(request.record, &value)?));
    finish(
        session,
        revision,
        actor,
        "Place pinned symbol",
        edits,
        request.definition,
        &[value],
    )
}
fn record_id(doc: &Document, id: EntityId) -> Result<EntityId, DocumentError> {
    if matches!(
        doc.get(id).map(|e| &e.data),
        Some(EntityData::Instance { .. })
    ) {
        let value = read_instance(doc, id)?;
        instances(doc, value.definition)?
            .into_iter()
            .find(|(_, v)| v.instance == id)
            .map(|(r, _)| r)
            .ok_or_else(|| invalid("Symbol ownership record is missing"))
    } else {
        Ok(id)
    }
}
pub fn prepare_edit(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &SymbolEditRequest,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    super::validate(session.document())?;
    let record = record_id(session.document(), request.record)?;
    let mut value = read_instance(session.document(), record)?;
    let def = read_definition(session.document(), value.definition)?;
    value.overrides = request.overrides.clone();
    let mut edits = generated(&def, &value)?
        .into_values()
        .map(Mutation::Replace)
        .collect::<Vec<_>>();
    edits.push(Mutation::Replace(record_entity(record, &value)?));
    finish(
        session,
        revision,
        actor,
        "Edit symbol instance values",
        edits,
        value.definition,
        &[value],
    )
}
fn validate_replacement(
    doc: &Document,
    definition: EntityId,
    replacement: &SymbolDefinition,
) -> Result<SymbolDefinition, DocumentError> {
    let old = read_definition(doc, definition)?;
    replacement.validate()?;
    if old.symbol != replacement.symbol {
        return Err(invalid("An update must retain the stable symbol identity"));
    }
    let old_pin = old.pin()?;
    let new_pin = replacement.pin()?;
    if old.revision == replacement.revision && old_pin != new_pin {
        return Err(invalid("Changed symbol content requires a new revision"));
    }
    architecture::compatible_roles(&old, replacement)?;
    Ok(old)
}
pub fn compare_update(
    doc: &Document,
    definition: EntityId,
    replacement: &SymbolDefinition,
) -> Result<SymbolUpgradeReport, DocumentError> {
    let request = SymbolUpdateRequest::new(doc, definition, replacement.clone())?;
    compare_update_request(doc, &request)
}
/// Compare an explicitly repaired update, including receiving context changes.
/// The comparison prepares source only and carries no native geometry receipt.
pub fn compare_update_request(
    doc: &Document,
    request: &SymbolUpdateRequest,
) -> Result<SymbolUpgradeReport, DocumentError> {
    let definition = request.definition;
    let replacement = &request.replacement;
    let old = validate_replacement(doc, definition, replacement)?;
    let old_pin = old.pin()?;
    let new_pin = replacement.pin()?;
    let previous = old.exposed.iter().copied().collect::<BTreeSet<_>>();
    let next = replacement.exposed.iter().copied().collect::<BTreeSet<_>>();
    let all = old
        .template
        .entities()
        .map(|e| e.id)
        .chain(replacement.template.entities().map(|e| e.id))
        .collect::<BTreeSet<_>>();
    let placed = instances(doc, definition)?;
    let mut conflicts: Vec<_> = placed
        .iter()
        .flat_map(|(record, v)| {
            override_errors(
                replacement,
                request
                    .instance_overrides
                    .get(record)
                    .unwrap_or(&v.overrides),
            )
            .into_iter()
            .map(|(parameter, reason)| SymbolOverrideConflict {
                instance: v.instance,
                parameter,
                reason,
            })
        })
        .collect();
    for (record, value) in architecture::architecture_instances(doc, definition)? {
        conflicts.extend(
            override_errors(
                replacement,
                &request
                    .architecture_inputs
                    .get(&record)
                    .unwrap_or(&value.inputs)
                    .parameters,
            )
            .into_iter()
            .map(|(parameter, reason)| SymbolOverrideConflict {
                instance: value.output.anchors()[1],
                parameter,
                reason,
            }),
        );
    }
    let mut report = SymbolUpgradeReport {
        old_pin,
        new_pin,
        affected_instances: placed.iter().map(|(_, v)| v.instance).collect(),
        added_parameters: next.difference(&previous).copied().collect(),
        removed_parameters: previous.difference(&next).copied().collect(),
        changed_parameters: previous
            .intersection(&next)
            .filter(|id| old.template.get(**id) != replacement.template.get(**id))
            .copied()
            .collect(),
        changed_entities: all
            .into_iter()
            .filter(|id| old.template.get(*id) != replacement.template.get(*id))
            .collect(),
        conflicts,
        architecture_changes: architecture::upgrade_changes(doc, definition, &old, replacement)?,
        architecture_error: None,
    };
    for change in &report.architecture_changes {
        report.affected_instances.push(change.output.anchors()[1]);
    }
    if !report.architecture_changes.is_empty() {
        let candidate = (|| -> Result<_, CommandError> {
            let (edits, _) = update_edits(doc, request)?;
            let session = Session::new(doc.clone())?;
            session.prepare(Request {
                expected_revision: 0,
                actor: Actor::local(crate::commands::Channel::CommandLine),
                label: "Compare architectural symbol update".into(),
                edits,
            })
        })();
        match candidate {
            Ok(candidate) => {
                for change in &mut report.architecture_changes {
                    if let architecture::ArchitectureOutput::Fence { run, .. } = change.output {
                        change.removed_members = crate::model::fence_members(doc, run)?
                            .into_iter()
                            .filter_map(|(_, _, body)| {
                                candidate.document().get(body).is_none().then_some(body)
                            })
                            .collect();
                    }
                }
            }
            Err(error) => {
                let reason = error.to_string();
                if reason.contains("removed fence member")
                    && let Ok(removals) = architecture::planned_removals(doc, request, &old)
                {
                    for change in &mut report.architecture_changes {
                        change.removed_members =
                            removals.get(&change.record).cloned().unwrap_or_default();
                    }
                }
                report.architecture_error = Some(reason);
            }
        }
    }
    Ok(report)
}
pub fn prepare_update(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &SymbolUpdateRequest,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    let (edits, values) = update_edits(session.document(), request)?;
    let mut result = finish(
        session,
        revision,
        actor,
        "Update pinned symbol definition",
        edits,
        request.definition,
        &values,
    )?;
    result.instances.extend(
        architecture::architecture_instances(result.prepared.document(), request.definition)?
            .iter()
            .map(|(_, value)| value.output.anchors()[1]),
    );
    Ok(result)
}
/// The checked source edits of an explicit definition update and the
/// resulting instance records, before they are prepared as one transaction.
pub(super) fn update_edits(
    doc: &Document,
    request: &SymbolUpdateRequest,
) -> Result<(Vec<Mutation>, Vec<SymbolInstance>), CommandError> {
    super::validate(doc)?;
    validate_replacement(doc, request.definition, &request.replacement)?;
    for (id, other) in definitions(doc)? {
        if id != request.definition
            && other.symbol == request.replacement.symbol
            && other.revision == request.replacement.revision
        {
            return Err(invalid("This incoming revision is already embedded separately; select the incoming file as an update snapshot instead of duplicating a definition").into());
        }
    }
    let old = read_definition(doc, request.definition)?;
    let previous = instances(doc, request.definition)?;
    let records = previous.iter().map(|(r, _)| *r).collect::<BTreeSet<_>>();
    let all_records = records
        .iter()
        .copied()
        .chain(
            architecture::architecture_instances(doc, request.definition)?
                .into_iter()
                .map(|(id, _)| id),
        )
        .collect::<BTreeSet<_>>();
    if request.maps.keys().copied().collect::<BTreeSet<_>>() != all_records
        || request
            .instance_overrides
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != records
    {
        return Err(
            invalid("An update must explicitly cover each affected symbol instance").into(),
        );
    }
    let keys = super::remap::identities(&request.replacement)?;
    let mut edits = vec![Mutation::Replace(entity(
        request.definition,
        request.replacement.name.clone(),
        EntityData::Extension {
            definition: request.replacement.definition()?,
            inputs: vec![],
        },
    ))];
    let mut values = vec![];
    for (record, mut value) in previous {
        let before = generated(&old, &value)?;
        let map = &request.maps[&record];
        for id in keys.iter().filter(|id| value.ids.contains_key(id)) {
            if map.get(id) != value.ids.get(id) {
                return Err(invalid(
                    "A compatible source identity cannot be reallocated during symbol update",
                )
                .into());
            }
        }
        for id in keys.iter().filter(|id| !value.ids.contains_key(id)) {
            if let Some(mapped) = map.get(id)
                && (value.ids.values().any(|old| old == mapped) || doc.get(*mapped).is_some())
            {
                return Err(invalid("New source identities require fresh owned IDs; removed source identities cannot be silently reused").into());
            }
        }
        value.ids = map.clone();
        value.overrides = request.instance_overrides[&record].clone();
        value.pin = request.replacement.pin()?;
        let after = generated(&request.replacement, &value)?;
        for id in before.keys().filter(|id| !after.contains_key(id)) {
            edits.push(Mutation::Remove(*id));
        }
        for (id, e) in after {
            if before.contains_key(&id) {
                edits.push(Mutation::Replace(e));
            } else {
                if doc.get(id).is_some() {
                    return Err(
                        invalid("New symbol identity collides with an existing entity").into(),
                    );
                }
                edits.push(Mutation::Insert(e));
            }
        }
        edits.push(Mutation::Replace(record_entity(record, &value)?));
        values.push(value);
    }
    edits.extend(architecture::upgrade_edits(doc, request, &old)?);
    Ok((edits, values))
}
