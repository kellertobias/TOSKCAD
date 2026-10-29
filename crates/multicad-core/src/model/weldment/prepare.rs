//! Transactions creating and editing weldment members. They are source-only:
//! a native evaluator must still solve sketch paths and build every member
//! solid before the transaction is offered for Apply.
use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::Mutation,
    materials::{LibraryRef, PartProperties},
    model::{
        CreatedBody, ModelPrepared,
        common::{error, revision},
    },
};

fn check_name(name: &str, what: &str) -> Result<(), CommandError> {
    if name.trim().is_empty() || name.len() > 1000 {
        return Err(error(format!("{what} name must contain 1–1000 bytes")));
    }
    Ok(())
}
/// The container new model entries go into: the active component or the root.
pub(super) fn insertion_parent(document: &Document) -> EntityId {
    document
        .hierarchy()
        .active_component
        .unwrap_or(document.id())
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemberRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub member: MemberDefinition,
}
impl MemberRequest {
    pub fn new(name: impl Into<String>, mut member: MemberDefinition) -> Self {
        let body = EntityId::new();
        member.output.body = body;
        Self {
            feature: EntityId::new(),
            body,
            name: name.into(),
            member,
        }
    }
}

/// Check what can be checked without solving: stock kind, sketch line
/// existence, and the resolved geometry of point paths.
fn check_member(
    document: &Document,
    parameters: &crate::parameters::EvaluatedParameters,
    member: &MemberDefinition,
) -> Result<(), CommandError> {
    member.validate()?;
    read_profile_stock(document, member.stock)?;
    match &member.path {
        MemberPath::SketchLine { sketch, line } => check_sketch_line(document, *sketch, *line)?,
        path => {
            if let Some(segment) = point_path(path, parameters)? {
                resolve_member(document, parameters, member, segment)?;
            }
        }
    }
    Ok(())
}
/// Feature, Part and manufacturing properties of one new member. The Part
/// takes the stock's material and records the stock assignment.
fn member_entities(
    document: &Document,
    request: &MemberRequest,
) -> Result<Vec<Mutation>, CommandError> {
    check_name(&request.name, "Member")?;
    let (feature_id, body_id) = (request.feature, request.body);
    if request.member.output.body != body_id {
        return Err(error("Member output must name the requested Part"));
    }
    for id in [feature_id, body_id] {
        if id.is_nil() || document.get(id).is_some() || feature_id == body_id {
            return Err(error("Member identities must be new, distinct and non-nil"));
        }
    }
    let spec = read_profile_stock(document, request.member.stock)?;
    let mut feature = Entity::new(
        format!("{} feature", request.name),
        EntityData::Feature {
            definition: request.member.definition()?,
            inputs: request.member.inputs(),
            suppressed: false,
        },
    );
    feature.id = feature_id;
    let mut body = Entity::new(
        request.name.clone(),
        EntityData::Part {
            features: vec![feature_id],
            material: Some(spec.material),
            tags: BTreeSet::new(),
        },
    );
    body.id = body_id;
    let properties = LibraryDefinition::PartProperties(PartProperties {
        part: body_id,
        material: Some(LibraryRef::capture(document, spec.material)?),
        stock: Some(LibraryRef::capture(document, request.member.stock)?),
        appearance: None,
        hatch: None,
    })
    .entity(
        EntityId::new(),
        format!("{} manufacturing properties", request.name),
    )?;
    Ok(vec![
        Mutation::Insert(feature),
        Mutation::Insert(body),
        Mutation::Insert(properties),
    ])
}

/// Create members as one transaction, each with its own Part. With a group
/// name, the members are held by a new model group of that name.
pub fn prepare_members(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    requests: &[MemberRequest],
    group: Option<&str>,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if requests.is_empty() || requests.len() > MAX_FRAME_MEMBERS {
        return Err(error(format!(
            "Create 1–{MAX_FRAME_MEMBERS} members at a time"
        )));
    }
    let document = session.document();
    let parameters = session.parameters();
    let mut edits = vec![];
    let mut seen = BTreeSet::new();
    for request in requests {
        if !seen.insert(request.feature) || !seen.insert(request.body) {
            return Err(error("Member identities must be distinct"));
        }
        check_member(document, parameters, &request.member)?;
        edits.extend(member_entities(document, request)?);
    }
    let outputs: Vec<CreatedBody> = requests
        .iter()
        .map(|r| CreatedBody {
            feature: r.feature,
            body: r.body,
        })
        .collect();
    // Duplicate sketch-line members are refused by document validation.
    let prepared = if let Some(name) = group {
        check_name(name, "Group")?;
        let entity = Entity::new(name, EntityData::Group {});
        let group_id = entity.id;
        edits.push(Mutation::Insert(entity));
        let mut state = document.hierarchy().clone();
        state
            .members
            .entry(insertion_parent(document))
            .or_default()
            .push(group_id);
        state.members.insert(
            group_id,
            outputs.iter().flat_map(|o| [o.body, o.feature]).collect(),
        );
        edits.push(Mutation::SetHierarchy(state));
        session.prepare(Request {
            expected_revision,
            actor,
            label: "Create weldment members".into(),
            edits,
        })?
    } else {
        session.prepare_scoped(Request {
            expected_revision,
            actor,
            label: "Create weldment members".into(),
            edits,
        })?
    };
    Ok(ModelPrepared { prepared, outputs })
}

/// One member request per solid line of a sketch, named "<prefix> N" in
/// source order. `template` supplies the stock, anchor, rotation and
/// extensions; its path is replaced.
pub fn frame_member_requests(
    document: &Document,
    sketch: EntityId,
    prefix: &str,
    template: &MemberDefinition,
) -> Result<Vec<MemberRequest>, CommandError> {
    let lines = sketch_path_lines(document, sketch)?;
    if lines.is_empty() {
        return Err(error(
            "The sketch has no solid lines to turn into members; draw the frame path first",
        ));
    }
    Ok(lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let mut member = template.clone();
            member.path = MemberPath::SketchLine { sketch, line };
            MemberRequest::new(format!("{prefix} {}", i + 1), member)
        })
        .collect())
}

/// Edit a member in place. The Part keeps its identity; a stock change moves
/// the Part's material and stock assignment with it.
pub fn prepare_member_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    member: &MemberDefinition,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let previous = read_member(document, feature)?;
    if previous.output != member.output {
        return Err(error("A member edit keeps its output Part"));
    }
    check_member(document, session.parameters(), member)?;
    let mut entity = document
        .get(feature)
        .cloned()
        .ok_or_else(|| error("Member is missing"))?;
    entity.data = EntityData::Feature {
        definition: member.definition()?,
        inputs: member.inputs(),
        suppressed: false,
    };
    let mut edits = vec![Mutation::Replace(entity)];
    if previous.stock != member.stock {
        edits.extend(restock(document, member.output.body, member.stock)?);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit weldment member".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature,
            body: member.output.body,
        }],
    })
}
fn restock(
    document: &Document,
    part: EntityId,
    stock: EntityId,
) -> Result<Vec<Mutation>, CommandError> {
    let spec = read_profile_stock(document, stock)?;
    let mut edits = vec![];
    if let Some(mut entity) = document.get(part).cloned()
        && let EntityData::Part { material, .. } = &mut entity.data
        && *material != Some(spec.material)
    {
        *material = Some(spec.material);
        edits.push(Mutation::Replace(entity));
    }
    if let Some((id, mut properties)) = materials::part_properties(document, part)?
        && properties.stock.as_ref().map(|s| s.id) != Some(stock)
    {
        properties.stock = Some(LibraryRef::capture(document, stock)?);
        properties.material = Some(LibraryRef::capture(document, spec.material)?);
        let name = document
            .get(id)
            .map_or("Part manufacturing properties".into(), |e| e.name.clone());
        edits.push(Mutation::Replace(
            LibraryDefinition::PartProperties(properties).entity(id, name)?,
        ));
    }
    Ok(edits)
}

fn check_connection(
    document: &Document,
    connection: &ConnectionDefinition,
) -> Result<(), CommandError> {
    connection.validate()?;
    for part in connection.members {
        let feature = member_feature(document, part).ok_or_else(|| {
            error(format!(
                "{} is not a weldment member; connect member Parts only",
                document
                    .get(part)
                    .map_or("A Part".into(), |e| e.name.clone())
            ))
        })?;
        read_member(document, feature)?;
    }
    Ok(())
}
/// Connect two members with a miter, butt joint or cope. The cuts are
/// derived natively from both members' geometry; this transaction is
/// source-only until both members are evaluated.
pub fn prepare_connection(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    connection: &ConnectionDefinition,
) -> Result<(EntityId, crate::commands::Prepared), CommandError> {
    revision(session, expected_revision)?;
    check_name(name, "Connection")?;
    let document = session.document();
    check_connection(document, connection)?;
    let entity = Entity::new(name, connection.entity_data()?);
    let id = entity.id;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Create {} connection", connection.kind.label()),
        edits: vec![Mutation::Insert(entity)],
    })?;
    Ok((id, prepared))
}
/// Change a connection's kind, member order or gap, keeping its identity.
pub fn prepare_connection_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    connection: &ConnectionDefinition,
) -> Result<crate::commands::Prepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    read_connection(document, id)?;
    check_connection(document, connection)?;
    let mut entity = document
        .get(id)
        .cloned()
        .ok_or_else(|| error("Connection is missing"))?;
    entity.data = connection.entity_data()?;
    session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit connection".into(),
        edits: vec![Mutation::Replace(entity)],
    })
}

fn check_weld(document: &Document, weld: &WeldDefinition) -> Result<(), CommandError> {
    weld.validate()?;
    read_connection(document, weld.connection)
        .map_err(|_| error("A weld attaches to a member connection; connect the members first"))?;
    Ok(())
}
/// Attach a weld to a member connection.
pub fn prepare_weld(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    weld: &WeldDefinition,
) -> Result<(EntityId, crate::commands::Prepared), CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    check_weld(document, weld)?;
    let entity = Entity::new(format!("Weld {}", weld.mark.trim()), weld.entity_data()?);
    let id = entity.id;
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Add weld {}", weld.mark.trim()),
        edits: vec![Mutation::Insert(entity)],
    })?;
    Ok((id, prepared))
}
/// Change a weld, keeping its identity and its place in any sequence.
pub fn prepare_weld_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    weld: &WeldDefinition,
) -> Result<crate::commands::Prepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    read_weld(document, id)?;
    check_weld(document, weld)?;
    let mut entity = document
        .get(id)
        .cloned()
        .ok_or_else(|| error("Weld is missing"))?;
    entity.name = format!("Weld {}", weld.mark.trim());
    entity.data = weld.entity_data()?;
    session.prepare(Request {
        expected_revision,
        actor,
        label: format!("Edit weld {}", weld.mark.trim()),
        edits: vec![Mutation::Replace(entity)],
    })
}
/// Create a weld sequence (`existing` none) or replace an existing
/// sequence's steps, for example to reorder them.
pub fn prepare_sequence(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    existing: Option<EntityId>,
    name: &str,
    sequence: &SequenceDefinition,
) -> Result<(EntityId, crate::commands::Prepared), CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    sequence.validate()?;
    for step in &sequence.steps {
        read_weld(document, step.weld)
            .map_err(|_| error("Sequence steps must name existing welds"))?;
    }
    let entity = match existing {
        Some(id) => {
            read_sequence(document, id)?;
            let mut entity = document
                .get(id)
                .cloned()
                .ok_or_else(|| error("Sequence is missing"))?;
            entity.data = sequence.entity_data()?;
            entity
        }
        None => {
            check_name(name, "Sequence")?;
            Entity::new(name, sequence.entity_data()?)
        }
    };
    let id = entity.id;
    let (label, edit) = if existing.is_some() {
        ("Edit weld sequence", Mutation::Replace(entity))
    } else {
        ("Create weld sequence", Mutation::Insert(entity))
    };
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits: vec![edit],
    })?;
    Ok((id, prepared))
}
