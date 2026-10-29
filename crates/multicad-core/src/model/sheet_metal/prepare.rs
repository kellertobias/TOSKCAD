//! Transactions creating and editing sheet parts. Each is source-only: a
//! native evaluator must still build the folded solid and its flat pattern
//! before the transaction is offered for Apply.
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
use std::collections::BTreeSet;

pub struct SheetRequest {
    pub feature: EntityId,
    pub body: EntityId,
    pub name: String,
    pub sheet: SheetDefinition,
}
impl SheetRequest {
    pub fn new(name: impl Into<String>, mut sheet: SheetDefinition) -> Self {
        let body = EntityId::new();
        sheet.output.body = body;
        Self {
            feature: EntityId::new(),
            body,
            name: name.into(),
            sheet,
        }
    }
}

/// Create a sheet Part. The Part takes the stock's material and records the
/// stock assignment in its manufacturing properties.
pub fn prepare_sheet(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &SheetRequest,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    if request.name.trim().is_empty() || request.name.len() > 1000 {
        return Err(error("Sheet part name must contain 1–1000 bytes"));
    }
    if request.sheet.output.body != request.body {
        return Err(error("Sheet output must name the requested Part"));
    }
    let document = session.document();
    for id in [request.feature, request.body] {
        if id.is_nil() || document.get(id).is_some() || request.feature == request.body {
            return Err(error("Sheet identities must be new, distinct and non-nil"));
        }
    }
    resolve_sheet(document, session.parameters(), &request.sheet)?;
    let spec = read_sheet_stock(document, request.sheet.stock)?;
    let mut feature = Entity::new(
        format!("{} feature", request.name),
        EntityData::Feature {
            definition: request.sheet.definition()?,
            inputs: request.sheet.inputs(),
            suppressed: false,
        },
    );
    feature.id = request.feature;
    let mut body = Entity::new(
        request.name.clone(),
        EntityData::Part {
            features: vec![request.feature],
            material: Some(spec.material),
            tags: BTreeSet::new(),
        },
    );
    body.id = request.body;
    let properties = LibraryDefinition::PartProperties(PartProperties {
        part: request.body,
        material: Some(LibraryRef::capture(document, spec.material)?),
        stock: Some(LibraryRef::capture(document, request.sheet.stock)?),
        appearance: None,
        hatch: None,
    })
    .entity(
        EntityId::new(),
        format!("{} manufacturing properties", request.name),
    )?;
    let prepared = session.prepare_scoped(Request {
        expected_revision,
        actor,
        label: "Create sheet part".into(),
        edits: vec![
            Mutation::Insert(feature),
            Mutation::Insert(body),
            Mutation::Insert(properties),
        ],
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature: request.feature,
            body: request.body,
        }],
    })
}

/// Edit a sheet part in place: the feature and Part identities are kept. A
/// stock change moves the Part's material and stock assignment with it.
pub fn prepare_sheet_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    sheet: &SheetDefinition,
) -> Result<ModelPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let previous = read_sheet(document, feature)?;
    if previous.output != sheet.output {
        return Err(error("A sheet edit keeps its output Part"));
    }
    resolve_sheet(document, session.parameters(), sheet)?;
    let mut entity = document
        .get(feature)
        .cloned()
        .ok_or_else(|| error("Sheet part is missing"))?;
    entity.data = EntityData::Feature {
        definition: sheet.definition()?,
        inputs: sheet.inputs(),
        suppressed: false,
    };
    let mut edits = vec![Mutation::Replace(entity)];
    if previous.stock != sheet.stock {
        let part = sheet.output.body;
        let spec = read_sheet_stock(document, sheet.stock)?;
        if let Some(mut entity) = document.get(part).cloned()
            && let EntityData::Part { material, .. } = &mut entity.data
            && *material != Some(spec.material)
        {
            *material = Some(spec.material);
            edits.push(Mutation::Replace(entity));
        }
        if let Some((id, mut properties)) = materials::part_properties(document, part)? {
            properties.stock = Some(LibraryRef::capture(document, sheet.stock)?);
            properties.material = Some(LibraryRef::capture(document, spec.material)?);
            let name = document
                .get(id)
                .map_or("Part manufacturing properties".into(), |e| e.name.clone());
            edits.push(Mutation::Replace(
                LibraryDefinition::PartProperties(properties).entity(id, name)?,
            ));
        }
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Edit sheet part".into(),
        edits,
    })?;
    Ok(ModelPrepared {
        prepared,
        outputs: vec![CreatedBody {
            feature,
            body: sheet.output.body,
        }],
    })
}
