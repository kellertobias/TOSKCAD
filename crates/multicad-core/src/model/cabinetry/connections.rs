//! Generator-owned connections are ordinary shared joint/machining definitions.
//! Reconciliation composes candidates into one source transaction and never
//! deletes a separately authored operation.
use super::*;
use crate::{
    commands::{Actor, CommandError, Request, Session},
    document::Mutation,
    model::{CreatedBody, common::error, length},
    parameters::EvaluatedParameters,
};

pub(super) fn check_owner(
    document: &Document,
    owner: EntityId,
    parts: impl IntoIterator<Item = EntityId>,
) -> Result<(), DocumentError> {
    if read_carcass(document, owner)?.cabinet.is_none() {
        return Err(invalid(
            "Generated connections need an explicit Cabinet owner",
        ));
    }
    for part in parts {
        let feature = panel_feature(document, part)
            .ok_or_else(|| invalid("A generated connection target is not a panel"))?;
        if !matches!(read_panel(document,feature)?.source,PanelSource::Carcass{carcass,..} if carcass==owner)
        {
            return Err(invalid(
                "Generated connections must target their owning Cabinet's panels",
            ));
        }
    }
    Ok(())
}

fn wanted_kind(cabinet: &CabinetOptions) -> PanelJointKind {
    match &cabinet.joinery {
        CabinetJoinery::Dowels(spec) => PanelJointKind::Dowels(spec.clone()),
        CabinetJoinery::Domino(spec) => PanelJointKind::LooseTenon(spec.clone()),
    }
}
fn owned_joints(
    document: &Document,
    id: EntityId,
) -> Result<Vec<(EntityId, PanelJointDefinition)>, DocumentError> {
    let mut out = vec![];
    for entity in panel_joints(document) {
        let joint = read_panel_joint(document, entity.id)?;
        if joint.generated_by == Some(id) {
            out.push((entity.id, joint));
        }
    }
    Ok(out)
}

fn owned_machining(
    document: &Document,
    id: EntityId,
) -> Result<Vec<(EntityId, MachiningDefinition)>, DocumentError> {
    let mut out = vec![];
    for entity in machinings(document) {
        let operation = read_machining(document, entity.id)?;
        if operation.generated_by == Some(id) {
            out.push((entity.id, operation));
        }
    }
    Ok(out)
}

fn pairs(
    document: &Document,
    id: EntityId,
    definition: &CarcassDefinition,
) -> Result<Vec<PanelJointPair>, DocumentError> {
    let Some(cabinet) = &definition.cabinet else {
        return Ok(vec![]);
    };
    let panels: std::collections::BTreeMap<_, _> = carcass_panels(document, id)?
        .into_iter()
        .map(|(role, _, body)| (role, body))
        .collect();
    let mut horizontal = vec![PanelRole::Top, PanelRole::Bottom];
    horizontal.extend(
        cabinet
            .shelves
            .iter()
            .filter(|s| s.support == ShelfSupport::Fixed)
            .map(|s| PanelRole::Shelf { index: s.key }),
    );
    let mut out = vec![];
    for role in horizontal {
        for side in [PanelRole::LeftSide, PanelRole::RightSide] {
            if let (Some(a), Some(b)) = (panels.get(&side), panels.get(&role)) {
                out.push(PanelJointPair { a: *a, b: *b });
            }
        }
    }
    Ok(out)
}

/// Remove only generated operations that cannot be retained in the new
/// definition. A user feature depending on their cuts still blocks removal.
pub(super) fn clean_before_edit(
    session: &Session,
    actor: &Actor,
    id: EntityId,
    definition: &CarcassDefinition,
) -> Result<Document, CommandError> {
    let mut candidate = session.document().clone();
    let wanted = pairs(&candidate, id, definition)?;
    let kind = definition.cabinet.as_ref().map(wanted_kind);
    for (joint_id, joint) in owned_joints(session.document(), id)? {
        let keep = kind
            .as_ref()
            .is_some_and(|k| std::mem::discriminant(k) == std::mem::discriminant(&joint.kind))
            && joint.pairs.len() == 1
            && wanted.contains(&joint.pairs[0]);
        if !keep {
            let stage = Session::new(candidate)?;
            candidate = super::joint::prepare_owned_joint_delete(
                &stage,
                stage.revision(),
                actor.clone(),
                joint_id,
            )?
            .document()
            .clone();
        }
    }
    let adjustable = definition.cabinet.as_ref().is_some_and(|c| {
        c.shelves
            .iter()
            .any(|s| matches!(s.support, ShelfSupport::System32 { .. }))
    });
    if !adjustable {
        for (operation, _) in owned_machining(session.document(), id)? {
            let stage = Session::new(candidate)?;
            candidate = super::machining::prepare_owned_machining_delete(
                &stage,
                stage.revision(),
                actor.clone(),
                operation,
            )?
            .document()
            .clone();
        }
    }
    Ok(candidate)
}

/// Resolve the generated pin operation afresh from its owning definition.
/// No evaluated position is frozen into source, so parameter-only changes
/// rebuild the drilling as well as the panel geometry.
pub(super) fn pin_definition(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
) -> Result<MachiningDefinition, DocumentError> {
    let definition = read_carcass(document, id)?;
    let cabinet = definition
        .cabinet
        .as_ref()
        .ok_or_else(|| invalid("Cabinet pin rows have no configuration"))?;
    let resolved = resolve_carcass(document, parameters, &definition)?;
    let mm = |d: &Dimension| crate::model::common::number(d, Unit::Millimeter, parameters);
    let toe = mm(&cabinet.toe_clearance)?;
    let thickness = stock_thickness(&read_panel_stock(document, definition.stock)?);
    let side = resolved
        .panels
        .iter()
        .find(|p| p.role == PanelRole::LeftSide)
        .ok_or_else(|| invalid("Cabinet has no left side"))?
        .panel;
    let side_bottom = side.origin[2] - resolved.origin[2];
    let front = mm(&cabinet.pins.front_inset)?;
    let rear = mm(&cabinet.pins.rear_inset)?;
    let diameter = mm(&cabinet.pins.diameter)?;
    let depth = mm(&cabinet.pins.depth)?;
    if depth <= 0. || depth > thickness - MIN_WALL_MM + 1e-6 {
        return Err(invalid(format!(
            "Shelf-pin receiving depth must leave at least {MIN_WALL_MM} mm in the cabinet side"
        )));
    }
    if mm(&cabinet.pins.start)? < diameter / 2. + MIN_WALL_MM
        || mm(&cabinet.pins.end_clearance)? < diameter / 2. + MIN_WALL_MM
    {
        return Err(invalid(
            "Shelf-pin start and end clearances must leave material beyond the hole radius",
        ));
    }
    let setback = mm(&definition.shelves.setback)?;
    if diameter <= 0. || front < diameter / 2. + MIN_WALL_MM || rear < diameter / 2. + MIN_WALL_MM {
        return Err(invalid(
            "Shelf-pin rows must leave material inside both shelf edges; increase row insets or reduce pin diameter",
        ));
    }
    let mut positions = vec![];
    for shelf in cabinet
        .shelves
        .iter()
        .filter(|s| matches!(s.support, ShelfSupport::System32 { .. }))
    {
        let depth = mm(&shelf.depth)?;
        if depth - front - rear < diameter + MIN_WALL_MM {
            return Err(invalid(format!(
                "Shelf {} is too shallow for two distinct support rows",
                shelf.key + 1
            )));
        }
        positions.extend([setback + front, setback + depth - rear]);
    }
    positions.sort_by(f64::total_cmp);
    positions.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let panels = carcass_panels(document, id)?;
    let targets = [PanelRole::LeftSide, PanelRole::RightSide]
        .into_iter()
        .map(|role| {
            panels
                .iter()
                .find(|(r, ..)| *r == role)
                .map(|(.., body)| *body)
                .ok_or_else(|| invalid("Cabinet side panel is missing"))
        })
        .collect::<Result<_, _>>()?;
    let operation = MachiningDefinition {
        generated_by: Some(id),
        targets,
        face: MachiningFace::Inside,
        suppressed: false,
        kind: MachiningKind::ShelfPins {
            along: PanelAxis::V,
            rows: positions
                .into_iter()
                .map(|p| ShelfPinRow {
                    from: RowEdge::Start,
                    offset: length(p),
                })
                .collect(),
            pitch: length(SYSTEM32_PITCH_MM),
            start: length(toe + thickness + mm(&cabinet.pins.start)? - side_bottom),
            end: length(
                side_bottom + side.width_mm
                    - (resolved.size_mm[1] - thickness - mm(&cabinet.pins.end_clearance)?),
            ),
            count: None,
            diameter: cabinet.pins.diameter.clone(),
            depth: cabinet.pins.depth.clone(),
        },
    };
    operation.validate()?;
    Ok(operation)
}

pub(super) fn finish(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    mut base: CarcassPrepared,
) -> Result<CarcassPrepared, CommandError> {
    let id = base.carcass;
    let mut candidate = base.model.prepared.document().clone();
    let definition = read_carcass(&candidate, id)?;
    if let Some(cabinet) = &definition.cabinet {
        let wanted = pairs(&candidate, id, &definition)?;
        for pair in wanted {
            let existing: Vec<_> = owned_joints(&candidate, id)?
                .into_iter()
                .filter(|(_, j)| j.pairs == [pair])
                .collect();
            if existing.len() > 1 {
                return Err(error(
                    "Cabinet has duplicate generated connections; repair ownership before editing",
                ));
            }
            let joint = PanelJointDefinition {
                generated_by: Some(id),
                kind: wanted_kind(cabinet),
                pairs: vec![pair],
            };
            let stage = Session::new(candidate)?;
            if let Some((joint_id, _)) = existing.first() {
                resolve_panel_joint_pair(stage.document(), stage.parameters(), &joint, 0)?;
                let mut entity = stage
                    .document()
                    .get(*joint_id)
                    .cloned()
                    .ok_or_else(|| error("Generated joint is missing"))?;
                entity.data = joint.entity_data(stage.document())?;
                candidate = stage.document().edited(&[Mutation::Replace(entity)])?;
            } else {
                let name = format!(
                    "{} · {} / {}",
                    stage
                        .document()
                        .get(id)
                        .map_or("Cabinet", |e| e.name.as_str()),
                    stage
                        .document()
                        .get(pair.a)
                        .map_or("side", |e| e.name.as_str()),
                    stage
                        .document()
                        .get(pair.b)
                        .map_or("panel", |e| e.name.as_str())
                );
                candidate =
                    prepare_panel_joint(&stage, stage.revision(), actor.clone(), &name, &joint)?
                        .model
                        .prepared
                        .document()
                        .clone();
            }
        }
        if cabinet
            .shelves
            .iter()
            .any(|s| matches!(s.support, ShelfSupport::System32 { .. }))
        {
            let existing = owned_machining(&candidate, id)?;
            if existing.len() > 1 {
                return Err(error(
                    "Cabinet has duplicate generated shelf-pin operations",
                ));
            }
            let stage = Session::new(candidate)?;
            let resolved = pin_definition(stage.document(), stage.parameters(), id)?;
            let operation = MachiningDefinition {
                kind: MachiningKind::CabinetShelfPins,
                ..resolved
            };
            if let Some((operation_id, _)) = existing.first() {
                check_machining(
                    stage.document(),
                    stage.parameters(),
                    &operation,
                    Some(*operation_id),
                )?;
                let mut entity = stage
                    .document()
                    .get(*operation_id)
                    .cloned()
                    .ok_or_else(|| error("Generated support is missing"))?;
                entity.data = operation.entity_data(stage.document())?;
                candidate = stage.document().edited(&[Mutation::Replace(entity)])?;
            } else {
                candidate = prepare_machining(
                    &stage,
                    stage.revision(),
                    actor.clone(),
                    "Cabinet System 32 supports",
                    &operation,
                )?
                .model
                .prepared
                .document()
                .clone();
            }
        }
    }
    let parameters = crate::parameters::evaluate(&candidate)?;
    check_cabinet_connections(&candidate, &parameters, id, None)?;
    let mut outputs = vec![];
    for (_, _, body) in carcass_panels(&candidate, id)? {
        outputs.push(CreatedBody {
            feature: super::chain::head(&candidate, body)?,
            body,
        });
    }
    for (joint, _) in owned_joints(&candidate, id)? {
        outputs.extend(
            joint_tenons(&candidate, joint)
                .into_iter()
                .map(|(_, feature, body)| CreatedBody { feature, body }),
        );
    }
    base.model.prepared = session.prepare(Request {
        expected_revision,
        actor,
        label: if session.document().get(id).is_some() {
            "Edit cabinet"
        } else {
            "Create cabinet"
        }
        .into(),
        edits: crate::model::extrude_boolean::delta(session.document(), &candidate)?,
    })?;
    if base.model.prepared.document() != &candidate {
        return Err(error("Cabinet preparation changed its derived candidate"));
    }
    base.model.outputs = outputs;
    Ok(base)
}
