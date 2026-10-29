//! Transactions creating and editing panels, carcasses and stacks. Each is
//! source-only: a native evaluator must still build every panel solid before
//! the transaction is offered for Apply.
use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::Mutation,
    hierarchy,
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
/// Feature, Part and manufacturing properties of one new panel. The Part
/// takes the stock's material and records the stock assignment.
fn panel_entities(
    document: &Document,
    output: &CreatedBody,
    name: &str,
    panel: &PanelDefinition,
    stock: EntityId,
) -> Result<Vec<Mutation>, CommandError> {
    for id in [output.feature, output.body] {
        if id.is_nil() || document.get(id).is_some() || output.feature == output.body {
            return Err(error("Panel identities must be new, distinct and non-nil"));
        }
    }
    let spec = read_panel_stock(document, stock)?;
    let mut feature = Entity::new(
        format!("{name} feature"),
        EntityData::Feature {
            definition: panel.definition()?,
            inputs: panel.inputs(),
            suppressed: false,
        },
    );
    feature.id = output.feature;
    let mut body = Entity::new(
        name,
        EntityData::Part {
            features: vec![output.feature],
            material: Some(spec.material),
            tags: BTreeSet::new(),
        },
    );
    body.id = output.body;
    let properties = LibraryDefinition::PartProperties(PartProperties {
        part: output.body,
        material: Some(LibraryRef::capture(document, spec.material)?),
        stock: Some(LibraryRef::capture(document, stock)?),
        appearance: None,
        hatch: None,
    })
    .entity(EntityId::new(), format!("{name} manufacturing properties"))?;
    Ok(vec![
        Mutation::Insert(feature),
        Mutation::Insert(body),
        Mutation::Insert(properties),
    ])
}
/// Point an existing panel Part at new stock: its material and stock
/// assignment follow, other properties are kept.
fn restock(
    document: &Document,
    part: EntityId,
    stock: EntityId,
) -> Result<Vec<Mutation>, CommandError> {
    let spec = read_panel_stock(document, stock)?;
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
        edits.push(Mutation::Replace(
            LibraryDefinition::PartProperties(properties).entity(id, {
                document
                    .get(id)
                    .map_or("Part manufacturing properties".into(), |e| e.name.clone())
            })?,
        ));
    }
    Ok(edits)
}
fn remove_panel(
    document: &Document,
    feature: EntityId,
    body: EntityId,
) -> Result<Vec<Mutation>, CommandError> {
    if let Some(joint) = panel_joints_of(document, body)
        .first()
        .or(machinings_of(document, body).first())
    {
        return Err(error(format!(
            "{} is joined by {}; delete it before removing the panel",
            document
                .get(body)
                .map_or("A panel".into(), |e| e.name.clone()),
            document
                .get(*joint)
                .map_or("a joint".into(), |e| e.name.clone()),
        )));
    }
    let mut edits = vec![Mutation::Remove(body), Mutation::Remove(feature)];
    if let Some((id, _)) = materials::part_properties(document, body)? {
        edits.push(Mutation::Remove(id));
    }
    Ok(edits)
}
/// The container new model entries go into: the active component or the root.
fn insertion_parent(document: &Document) -> EntityId {
    document
        .hierarchy()
        .active_component
        .unwrap_or(document.id())
}

#[derive(Clone, Debug, PartialEq)]
pub struct CarcassRequest {
    pub carcass: EntityId,
    pub name: String,
    pub definition: CarcassDefinition,
    /// Hold the panels in a new model group named after the carcass.
    pub group: bool,
}
impl CarcassRequest {
    pub fn new(name: impl Into<String>, definition: CarcassDefinition) -> Self {
        Self {
            carcass: EntityId::new(),
            name: name.into(),
            definition,
            group: true,
        }
    }
}
#[derive(Clone, Debug)]
pub struct CarcassPrepared {
    pub carcass: EntityId,
    /// Every carcass panel by role after the change; new ones are created.
    pub panels: Vec<(PanelRole, CreatedBody)>,
    pub model: ModelPrepared,
}

/// Create a carcass and one named panel Part per role, as one transaction.
pub fn prepare_carcass(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &CarcassRequest,
) -> Result<CarcassPrepared, CommandError> {
    revision(session, expected_revision)?;
    check_name(&request.name, "Carcass")?;
    let document = session.document();
    if request.carcass.is_nil() || document.get(request.carcass).is_some() {
        return Err(error("Carcass identity must be new"));
    }
    let mut definition = request.definition.clone();
    let group = request.group.then(EntityId::new);
    definition.group = group;
    let mut carcass = Entity::new(request.name.clone(), definition.entity_data()?);
    carcass.id = request.carcass;
    let mut edits = vec![Mutation::Insert(carcass)];
    let candidate = document.edited(&edits)?;
    let parameters = crate::parameters::evaluate(&candidate)?;
    resolve_carcass(&candidate, &parameters, &definition)?;
    let mut panels = vec![];
    for role in definition.roles() {
        let output = CreatedBody {
            feature: EntityId::new(),
            body: EntityId::new(),
        };
        let panel = carcass_panel(request.carcass, role, output.body);
        let stock = definition
            .stock_for(role)
            .ok_or_else(|| error("Carcass back has no stock"))?;
        edits.extend(panel_entities(
            &candidate,
            &output,
            &format!("{} {}", request.name, role.label()),
            &panel,
            stock,
        )?);
        panels.push((role, output));
    }
    let prepared = if let Some(group) = group {
        let mut entity = Entity::new(request.name.clone(), EntityData::Group {});
        entity.id = group;
        edits.push(Mutation::Insert(entity));
        let mut state = document.hierarchy().clone();
        state
            .members
            .entry(insertion_parent(document))
            .or_default()
            .push(group);
        state.members.insert(
            group,
            panels
                .iter()
                .flat_map(|(_, o)| [o.body, o.feature])
                .collect(),
        );
        edits.push(Mutation::SetHierarchy(state));
        session.prepare(Request {
            expected_revision,
            actor,
            label: "Create carcass".into(),
            edits,
        })?
    } else {
        session.prepare_scoped(Request {
            expected_revision,
            actor,
            label: "Create carcass".into(),
            edits,
        })?
    };
    Ok(CarcassPrepared {
        carcass: request.carcass,
        model: ModelPrepared {
            prepared,
            outputs: panels.iter().map(|(_, o)| o.clone()).collect(),
        },
        panels,
    })
}
fn carcass_panel(carcass: EntityId, role: PanelRole, body: EntityId) -> PanelDefinition {
    PanelDefinition {
        output: BodyOutput { body },
        source: PanelSource::Carcass { carcass, role },
        grain: GrainAxis::U,
        edges: PanelEdges::default(),
        oversize: None,
    }
}

/// Edit a carcass. Panels whose role remains keep their Parts and identities;
/// shelves or a back that are no longer needed are removed and new ones are
/// created. Panels follow stock changes in their material and assignment.
pub fn prepare_carcass_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    id: EntityId,
    definition: &CarcassDefinition,
) -> Result<CarcassPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let previous = read_carcass(document, id)?;
    let name = document
        .get(id)
        .map(|e| e.name.clone())
        .ok_or_else(|| error("Carcass is missing"))?;
    let mut definition = definition.clone();
    definition.group = previous.group;
    let mut entity = document
        .get(id)
        .cloned()
        .ok_or_else(|| error("Carcass is missing"))?;
    entity.data = definition.entity_data()?;
    let mut edits = vec![Mutation::Replace(entity)];
    let existing = carcass_panels(document, id)?;
    let roles = definition.roles();
    let mut removed = BTreeSet::new();
    for (role, feature, body) in &existing {
        if roles.contains(role) {
            let stock = definition
                .stock_for(*role)
                .ok_or_else(|| error("Carcass back has no stock"))?;
            if previous.stock_for(*role) != Some(stock) {
                edits.extend(restock(document, *body, stock)?);
            }
        } else {
            edits.extend(remove_panel(document, *feature, *body)?);
            removed.extend([*feature, *body]);
        }
    }
    // Detach removed panels in the same transaction, before anything else.
    let mut state = document.hierarchy().clone();
    for members in state.members.values_mut() {
        members.retain(|m| !removed.contains(m));
    }
    if &state != document.hierarchy() {
        edits.push(Mutation::SetHierarchy(state.clone()));
    }
    let candidate = document.edited(&edits)?;
    let parameters = crate::parameters::evaluate(&candidate)?;
    resolve_carcass(&candidate, &parameters, &definition)?;
    let mut panels = vec![];
    let mut created = vec![];
    for role in roles {
        if let Some((_, feature, body)) = existing.iter().find(|(r, ..)| *r == role) {
            panels.push((
                role,
                CreatedBody {
                    feature: *feature,
                    body: *body,
                },
            ));
            continue;
        }
        let output = CreatedBody {
            feature: EntityId::new(),
            body: EntityId::new(),
        };
        let stock = definition
            .stock_for(role)
            .ok_or_else(|| error("Carcass back has no stock"))?;
        edits.extend(panel_entities(
            &candidate,
            &output,
            &format!("{name} {}", role.label()),
            &carcass_panel(id, role, output.body),
            stock,
        )?);
        panels.push((role, output.clone()));
        created.push(output);
    }
    let parent = definition
        .group
        .filter(|g| document.get(*g).is_some())
        .unwrap_or_else(|| insertion_parent(document));
    if !created.is_empty() && (parent != document.id() || state.members.contains_key(&parent)) {
        state
            .members
            .entry(parent)
            .or_default()
            .extend(created.iter().flat_map(|o| [o.body, o.feature]));
        edits.push(Mutation::SetHierarchy(state));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit carcass".into(),
        edits,
    })?;
    Ok(CarcassPrepared {
        carcass: id,
        model: ModelPrepared {
            prepared,
            outputs: panels.iter().map(|(_, o)| o.clone()).collect(),
        },
        panels,
    })
}

pub struct PanelRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub panel: PanelDefinition,
}
impl PanelRequest {
    pub fn new(name: impl Into<String>, mut panel: PanelDefinition) -> Self {
        let body = EntityId::new();
        panel.output.body = body;
        Self {
            feature: EntityId::new(),
            body,
            name: name.into(),
            panel,
        }
    }
}
fn panel_stock(panel: &PanelDefinition) -> Result<EntityId, CommandError> {
    match &panel.source {
        PanelSource::Rectangle { stock, .. } | PanelSource::Profile { stock, .. } => Ok(*stock),
        PanelSource::Carcass { .. } => Err(error(
            "Carcass panels are created and changed through their carcass",
        )),
    }
}
/// Create a rectangle or sketch-region panel Part from panel stock.
pub fn prepare_panel(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &PanelRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    check_name(&request.name, "Panel")?;
    if request.panel.output.body != request.body {
        return Err(error("Panel output must name the requested Part"));
    }
    let stock = panel_stock(&request.panel)?;
    let document = session.document();
    resolve_panel(document, session.parameters(), &request.panel)?;
    let output = CreatedBody {
        feature: request.feature,
        body: request.body,
    };
    let edits = panel_entities(document, &output, &request.name, &request.panel, stock)?;
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: "Create panel".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![output],
    })
}
/// Edit a rectangle or sketch-region panel; a stock change moves the Part's
/// material and stock assignment with it.
pub fn prepare_panel_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    panel: &PanelDefinition,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let previous = read_panel(document, feature)?;
    if previous.output != panel.output {
        return Err(error("A panel edit keeps its output Part"));
    }
    panel_stock(&previous)?;
    let stock = panel_stock(panel)?;
    resolve_panel(document, session.parameters(), panel)?;
    let mut entity = document
        .get(feature)
        .cloned()
        .ok_or_else(|| error("Panel is missing"))?;
    entity.data = EntityData::Feature {
        definition: panel.definition()?,
        inputs: panel.inputs(),
        suppressed: false,
    };
    let mut edits = vec![Mutation::Replace(entity)];
    if panel_stock(&previous)? != stock {
        edits.extend(restock(document, panel.output.body, stock)?);
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit panel".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature,
            body: panel.output.body,
        }],
    })
}

/// Record face-to-face panel layers as a stack. With `group`, the layers move
/// into a new model group named after the stack; they must share a container.
/// Contact between the layers is checked by the native evaluator.
pub fn prepare_stack(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    name: &str,
    layers: &[EntityId],
    adhesive: Option<String>,
    group: bool,
) -> Result<(EntityId, Prepared), CommandError> {
    revision(session, expected_revision)?;
    check_name(name, "Stack")?;
    let document = session.document();
    let definition = StackDefinition {
        layers: layers.to_vec(),
        adhesive,
        group: group.then(EntityId::new),
    };
    definition.validate()?;
    for layer in layers {
        let feature = panel_feature(document, *layer).ok_or_else(|| {
            error(format!(
                "{} is not a panel; stack panel Parts only",
                document
                    .get(*layer)
                    .map_or("A layer".into(), |e| e.name.clone())
            ))
        })?;
        read_panel(document, feature)?;
    }
    for other in stacks(document) {
        let s = read_stack(document, other.id)?;
        if let Some(part) = s.layers.iter().find(|p| layers.contains(p)) {
            return Err(error(format!(
                "{} is already a layer of {}",
                document
                    .get(*part)
                    .map_or("A layer".into(), |e| e.name.clone()),
                other.name
            )));
        }
    }
    let mut entity = Entity::new(name, definition.entity_data()?);
    let id = entity.id;
    let mut edits = vec![];
    if let Some(group_id) = definition.group {
        let index = hierarchy::Index::new(document).map_err(|e| error(e.to_string()))?;
        let mut parents = BTreeSet::new();
        for layer in layers {
            parents.insert(index.parent(*layer).map_err(|e| error(e.to_string()))?);
        }
        if parents.len() != 1 {
            return Err(error("Group stack layers that share one container"));
        }
        let parent = parents
            .into_iter()
            .next()
            .flatten()
            .unwrap_or(document.id());
        let mut state = document.hierarchy().clone();
        for members in state.members.values_mut() {
            members.retain(|m| !layers.contains(m));
        }
        state.members.entry(parent).or_default().push(group_id);
        state.members.insert(group_id, layers.to_vec());
        let mut group_entity = Entity::new(name, EntityData::Group {});
        group_entity.id = group_id;
        edits.push(Mutation::Insert(group_entity));
        edits.push(Mutation::SetHierarchy(state));
    }
    entity.id = id;
    edits.insert(0, Mutation::Insert(entity));
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Create panel stack".into(),
        edits,
    })?;
    Ok((id, prepared))
}
