//! Source transactions shared by native UI and external control. Callers must
//! still native-preflight the prepared candidate before offering Apply.
use super::*;
use crate::{
    commands::{Actor, CommandError, Prepared, Request, Session},
    document::{Mutation, Placement},
    symbols::{SymbolDefinition, SymbolEditRequest, SymbolPlaceRequest, SymbolValue},
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantPlaceRequest {
    pub definition: SymbolDefinition,
    pub placement: SymbolPlaceRequest,
    pub settings_id: EntityId,
    pub settings: PlantInstance,
}
fn source_error(message: &str) -> CommandError {
    invalid(message).into()
}
fn candidate(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    label: &str,
    document: &Document,
) -> Result<Prepared, CommandError> {
    let mut edits = vec![];
    for entity in document.entities() {
        match session.document().get(entity.id) {
            None => edits.push(Mutation::Insert(entity.clone())),
            Some(old) if old != entity => edits.push(Mutation::Replace(entity.clone())),
            _ => {}
        }
    }
    // Plant transactions never delete unrelated source or replace the document.
    if session
        .document()
        .entities()
        .any(|e| document.get(e.id).is_none())
    {
        return Err(source_error(
            "Plant transaction unexpectedly removed a canonical entity",
        ));
    }
    session.prepare(Request {
        expected_revision,
        actor,
        label: label.into(),
        edits,
    })
}
impl PlantPlaceRequest {
    pub fn new(
        session: &Session,
        definition: SymbolDefinition,
        name: String,
        height_mm: f64,
        spread_mm: f64,
        placement: Placement,
    ) -> Result<Self, CommandError> {
        let template = template_for(&definition.template, definition.root)?
            .ok_or_else(|| source_error("Choose a plant definition"))?;
        let imported = symbols::prepare_import(
            session,
            session.revision(),
            Actor::local(crate::commands::Channel::Mcp),
            &definition,
        )?;
        let overrides = BTreeMap::from([
            (
                template.height_parameter,
                SymbolValue {
                    value: height_mm,
                    unit: "mm".into(),
                },
            ),
            (
                template.spread_parameter,
                SymbolValue {
                    value: spread_mm,
                    unit: "mm".into(),
                },
            ),
        ]);
        let placement = SymbolPlaceRequest::new(
            imported.prepared.document(),
            imported.definition,
            name,
            overrides,
            placement,
        )?;
        let settings = PlantInstance {
            symbol_instance: placement.record,
            size_basis: SizeBasis::PlannedMature,
            label: None,
            plan_visible: true,
        };
        Ok(Self {
            definition,
            placement,
            settings_id: EntityId::new(),
            settings,
        })
    }
}
pub fn prepare_place(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &PlantPlaceRequest,
) -> Result<Prepared, CommandError> {
    if request.settings.symbol_instance != request.placement.record {
        return Err(source_error(
            "Plant settings and placement must share one source",
        ));
    }
    if session.document().get(request.settings_id).is_some() {
        return Err(source_error("Plant settings identity must be fresh"));
    }
    let imported = symbols::prepare_import_at(
        session,
        expected_revision,
        actor.clone(),
        &request.definition,
        request.placement.definition,
    )?;
    if imported.definition != request.placement.definition {
        return Err(source_error(
            "Plant placement does not reference its imported definition",
        ));
    }
    let staged = Session::new(imported.prepared.document().clone())?;
    let placed = symbols::prepare_place(&staged, 0, actor.clone(), &request.placement)?;
    let doc = placed.prepared.document().edited(&[Mutation::Insert(
        request.settings.entity(request.settings_id)?,
    )])?;
    let evaluated = crate::parameters::evaluate(&doc)?;
    resolve(&doc, &evaluated, request.placement.record)?
        .ok_or_else(|| source_error("Placement has no linked plant source"))?;
    candidate(session, expected_revision, actor, "Place plant", &doc)
}
pub fn prepare_edit(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    record: EntityId,
    height_mm: f64,
    spread_mm: f64,
    settings: PlantInstance,
) -> Result<Prepared, CommandError> {
    let symbol = symbols::read_instance(session.document(), record)?;
    let plant = resolve(session.document(), session.parameters(), record)?
        .ok_or_else(|| source_error("Choose a plant instance"))?;
    if settings.symbol_instance != record {
        return Err(source_error(
            "Plant edit requires its canonical ownership record",
        ));
    }
    let mut overrides = symbol.overrides;
    overrides.insert(
        plant.template.height_parameter,
        SymbolValue {
            value: height_mm,
            unit: "mm".into(),
        },
    );
    overrides.insert(
        plant.template.spread_parameter,
        SymbolValue {
            value: spread_mm,
            unit: "mm".into(),
        },
    );
    let edited = symbols::prepare_edit(
        session,
        expected_revision,
        actor.clone(),
        &SymbolEditRequest { record, overrides },
    )?;
    let existing = session.document().entities().find_map(|e| match &e.data {
        EntityData::Extension { definition, .. } if definition.type_id == INSTANCE_TYPE => {
            PlantInstance::from_definition(definition)
                .ok()
                .filter(|s| s.symbol_instance == record)
                .map(|_| e.id)
        }
        _ => None,
    });
    let entity = settings.entity(existing.unwrap_or_default())?;
    let mutation = if existing.is_some() {
        Mutation::Replace(entity)
    } else {
        Mutation::Insert(entity)
    };
    let doc = edited.prepared.document().edited(&[mutation])?;
    let evaluated = crate::parameters::evaluate(&doc)?;
    resolve(&doc, &evaluated, record)?;
    candidate(session, expected_revision, actor, "Edit plant", &doc)
}
/// One source edit including optional ordinary Move/Copy placement intent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlantEditRequest {
    pub record: EntityId,
    pub height_mm: f64,
    pub spread_mm: f64,
    pub settings: PlantInstance,
    pub name: String,
    pub move_by: Option<crate::model::InstanceMoveRequest>,
}
pub fn prepare_edit_request(
    session: &Session,
    expected_revision: u64,
    actor: Actor,
    request: &PlantEditRequest,
) -> Result<Prepared, CommandError> {
    let edited = prepare_edit(
        session,
        expected_revision,
        actor.clone(),
        request.record,
        request.height_mm,
        request.spread_mm,
        request.settings.clone(),
    )?;
    let symbol = symbols::read_instance(edited.document(), request.record)?;
    let doc = if let Some(movement) = &request.move_by {
        if movement.instance != symbol.instance {
            return Err(source_error("Plant move must target the edited plant"));
        }
        let staged = Session::new(edited.document().clone())?;
        crate::model::prepare_instance_moves(
            &staged,
            0,
            actor.clone(),
            std::slice::from_ref(movement),
        )?
        .prepared
        .document()
        .clone()
    } else {
        edited.document().clone()
    };
    let mut entity = doc.get(symbol.instance).expect("checked symbol").clone();
    entity.name = request.name.clone();
    let doc = doc.edited(&[Mutation::Replace(entity)])?;
    candidate(session, expected_revision, actor, "Edit plant", &doc)
}
/// Absolute garden-plane placement expressed through the ordinary rigid move
/// API, preserving earlier placement history and rejecting a tilted source.
pub fn planar_move(
    document: &Document,
    evaluated: &EvaluatedParameters,
    record: EntityId,
    position_mm: [f64; 3],
    rotation_deg: f64,
) -> Result<Option<crate::model::InstanceMoveRequest>, DocumentError> {
    let symbol = symbols::read_instance(document, record)?;
    let pose = crate::model::resolve_instance_pose(document, symbol.instance, evaluated)?;
    if pose.rotation[2][2] < 1. - 1e-8 {
        return Err(invalid(
            "This plant is tilted; use Move / Copy to edit its pose",
        ));
    }
    let current = pose.rotation[1][0].atan2(pose.rotation[0][0]).to_degrees();
    if pose.translation_mm == position_mm && (rotation_deg - current).abs() < 1e-9 {
        return Ok(None);
    }
    let parameters = crate::model::NumericTransform {
        translation: std::array::from_fn(|i| {
            crate::model::length(position_mm[i] - pose.translation_mm[i])
        }),
        pivot: pose.translation_mm.map(crate::model::length),
        angle: crate::model::angle(rotation_deg - current),
        ..Default::default()
    };
    parameters.resolve(evaluated)?;
    Ok(Some(crate::model::InstanceMoveRequest::new(
        symbol.instance,
        parameters,
    )))
}
