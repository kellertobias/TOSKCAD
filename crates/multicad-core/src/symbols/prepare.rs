use super::{
    materialize::{generated, record_entity},
    source::{author, entity, invalid, override_errors, representation_role_changes},
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
        let content = read_definition(doc, definition)?;
        Self::from_definition(definition, &content, name, overrides, placement)
    }
    /// Allocate stable placement identities before an immutable Library pin is embedded.
    pub fn from_definition(
        definition: EntityId,
        content: &SymbolDefinition,
        name: impl Into<String>,
        overrides: BTreeMap<EntityId, SymbolValue>,
        placement: Placement,
    ) -> Result<Self, DocumentError> {
        content.validate()?;
        if definition.is_nil() {
            return Err(invalid("A placement needs a non-nil definition identity"));
        }
        if super::architecture::blueprint_for_definition(content)?.is_some() {
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
            ids: super::remap::identities(content)?
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
/// Find an exact embedded pin without implicitly upgrading another revision.
pub fn library_placement_definition(
    document: &Document,
    content: &SymbolDefinition,
) -> Result<Option<EntityId>, DocumentError> {
    content.validate()?;
    let pin = content.pin()?;
    let mut older = false;
    for (id, existing) in definitions(document)? {
        if existing.symbol != content.symbol {
            continue;
        }
        if existing.pin()? == pin {
            return Ok(Some(id));
        }
        older = true;
    }
    if older {
        return Err(invalid(
            "Another revision of this symbol is embedded; explicitly compare and update its pin before placing this Library revision",
        ));
    }
    Ok(None)
}

fn placement_edits(
    document: &Document,
    definition: &SymbolDefinition,
    request: &SymbolPlaceRequest,
    parent: Option<EntityId>,
) -> Result<(Vec<Mutation>, SymbolInstance), DocumentError> {
    if super::architecture::blueprint_for_definition(definition)?.is_some() {
        return Err(invalid(
            "Draw a wall or fence to supply the architectural symbol's path and placement context",
        ));
    }
    crate::model::RigidPose::from_placement(request.placement)?;
    let value = SymbolInstance {
        definition: request.definition,
        pin: definition.pin()?,
        instance: request.instance,
        wrapper: request.wrapper,
        wrapper_metadata: request.wrapper_metadata,
        member: request.member,
        ids: request.ids.clone(),
        overrides: request.overrides.clone(),
    };
    let mut edits = generated(definition, &value)?
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
    if let Some(parent) = parent {
        if parent != document.id() {
            let container = document.get(parent).ok_or_else(|| {
                invalid("The placement parent is missing; choose a current assembly, group or root")
            })?;
            if crate::hierarchy::container_kind(&container.data).is_none() {
                return Err(invalid(
                    "The placement parent must be an assembly, group or root",
                ));
            }
            if owned_entities(document)?.contains(&parent) {
                return Err(invalid(
                    "A symbol's private container cannot own a new Library placement",
                ));
            }
        }
        let mut hierarchy = document.hierarchy().clone();
        hierarchy
            .members
            .entry(parent)
            .or_default()
            .push(request.instance);
        edits.push(Mutation::SetHierarchy(hierarchy));
    }
    Ok((edits, value))
}

/// Place an embedded definition at root, preserving the original public contract.
pub fn prepare_place(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &SymbolPlaceRequest,
) -> Result<SymbolPrepared, CommandError> {
    prepare_place_in(session, revision, actor, request, None)
}

/// Place one visible instance in an explicit parent without exposing private members.
pub fn prepare_place_in(
    session: &Session,
    revision: u64,
    actor: Actor,
    request: &SymbolPlaceRequest,
    parent: Option<EntityId>,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    super::validate(session.document())?;
    let definition = read_definition(session.document(), request.definition)?;
    let (edits, value) = placement_edits(session.document(), &definition, request, parent)?;
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

/// Embed a selected immutable Library revision and place it in one transaction.
pub fn prepare_library_place(
    session: &Session,
    revision: u64,
    actor: Actor,
    content: &SymbolDefinition,
    request: &SymbolPlaceRequest,
    parent: Option<EntityId>,
) -> Result<SymbolPrepared, CommandError> {
    stale(session, revision)?;
    super::validate(session.document())?;
    let existing = library_placement_definition(session.document(), content)?;
    if existing.is_some_and(|id| id != request.definition) {
        return Err(invalid("Placement must reuse the exact embedded definition identity").into());
    }
    let (mut edits, value) = placement_edits(session.document(), content, request, parent)?;
    if existing.is_none() {
        edits.insert(
            0,
            Mutation::Insert(entity(
                request.definition,
                content.name.clone(),
                EntityData::Extension {
                    definition: content.definition()?,
                    inputs: vec![],
                },
            )),
        );
    }
    finish(
        session,
        revision,
        actor,
        "Place Library symbol",
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
    for (record, value) in &placed {
        let proposed = request
            .instance_overrides
            .get(record)
            .unwrap_or(&value.overrides);
        conflicts.extend(
            representation_role_changes(&old, replacement, &value.overrides, proposed)?
                .into_iter()
                .map(|(parameter, reason)| SymbolOverrideConflict {
                    instance: value.instance,
                    parameter,
                    reason,
                }),
        );
    }
    for (record, value) in architecture::architecture_instances(doc, definition)? {
        let proposed = &request
            .architecture_inputs
            .get(&record)
            .unwrap_or(&value.inputs)
            .parameters;
        conflicts.extend(
            representation_role_changes(&old, replacement, &value.inputs.parameters, proposed)?
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
    // Enforce semantic conflicts here too: callers may prepare an update
    // directly without first requesting a comparison report.
    for (record, value) in &previous {
        if let Some((_, reason)) = representation_role_changes(
            &old,
            &request.replacement,
            &value.overrides,
            &request.instance_overrides[record],
        )?
        .into_iter()
        .next()
        {
            return Err(invalid(reason).into());
        }
    }
    for (record, value) in architecture::architecture_instances(doc, request.definition)? {
        let proposed = &request
            .architecture_inputs
            .get(&record)
            .unwrap_or(&value.inputs)
            .parameters;
        if let Some((_, reason)) = representation_role_changes(
            &old,
            &request.replacement,
            &value.inputs.parameters,
            proposed,
        )?
        .into_iter()
        .next()
        {
            return Err(invalid(reason).into());
        }
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
