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

/// Every hardware Part of a sheet after a fastener change.
#[derive(Clone, Debug)]
pub struct FastenersPrepared {
    /// (pattern, item, feature, Part) for every hardware Part after the
    /// change; new ones are created.
    pub hardware: Vec<(u32, HardwareItem, CreatedBody)>,
    pub model: ModelPrepared,
}

/// Replace a sheet part's mounting patterns. The sheet cuts the receiving
/// holes; each purchased item of each pattern is a hardware Part. Parts whose
/// pattern and item remain keep their identities; others are removed and new
/// ones created, in one transaction.
pub fn prepare_sheet_fasteners(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    feature: EntityId,
    patterns: &[MountingPattern],
) -> Result<FastenersPrepared, CommandError> {
    revision(session, expected_revision)?;
    let document = session.document();
    let mut sheet = read_sheet(document, feature)?;
    sheet.fasteners = patterns.to_vec();
    resolve_sheet(document, session.parameters(), &sheet)?;
    let sheet_name = document
        .get(sheet.output.body)
        .map_or("Sheet".into(), |e| e.name.clone());
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
    let wanted: Vec<(u32, HardwareItem)> = patterns
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            p.preset
                .items()
                .into_iter()
                .map(move |item| (i as u32, item))
        })
        .collect();
    let existing = sheet_hardware(document, feature)?;
    let mut removed = BTreeSet::new();
    for (pattern, item, f, part) in &existing {
        if !wanted.contains(&(*pattern, *item)) {
            edits.push(Mutation::Remove(*part));
            edits.push(Mutation::Remove(*f));
            removed.extend([*part, *f]);
        }
    }
    let mut state = document.hierarchy().clone();
    for members in state.members.values_mut() {
        members.retain(|m| !removed.contains(m));
    }
    if &state != document.hierarchy() {
        edits.push(Mutation::SetHierarchy(state));
    }
    let candidate = document.edited(&edits)?;
    let parameters = crate::parameters::evaluate(&candidate)?;
    let resolved = resolve_sheet(&candidate, &parameters, &sheet)?;
    let mut hardware = vec![];
    for (pattern, item) in wanted {
        if let Some((_, _, f, part)) = existing
            .iter()
            .find(|(p, i, ..)| *p == pattern && *i == item)
        {
            hardware.push((
                pattern,
                item,
                CreatedBody {
                    feature: *f,
                    body: *part,
                },
            ));
            continue;
        }
        let output = CreatedBody {
            feature: EntityId::new(),
            body: EntityId::new(),
        };
        let definition = HardwareDefinition {
            output: BodyOutput { body: output.body },
            sheet: feature,
            pattern,
            item,
        };
        let (designation, _) = resolved.patterns[pattern as usize].preset.identity(item);
        let name = format!("{sheet_name} {designation}");
        let mut f = Entity::new(
            format!("{name} feature"),
            EntityData::Feature {
                definition: definition.definition()?,
                inputs: definition.inputs(),
                suppressed: false,
            },
        );
        f.id = output.feature;
        let mut body = Entity::new(
            name,
            EntityData::Part {
                features: vec![output.feature],
                material: None,
                tags: BTreeSet::new(),
            },
        );
        body.id = output.body;
        edits.push(Mutation::Insert(f));
        edits.push(Mutation::Insert(body));
        hardware.push((pattern, item, output));
    }
    let prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: "Set sheet fasteners".into(),
        edits,
    })?;
    let mut outputs = vec![CreatedBody {
        feature,
        body: sheet.output.body,
    }];
    outputs.extend(hardware.iter().map(|(.., o)| o.clone()));
    Ok(FastenersPrepared {
        hardware,
        model: ModelPrepared { prepared, outputs },
    })
}
