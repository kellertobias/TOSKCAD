//! Generic editable context geometry, with an independently authored linked plan.
use super::*;
use crate::{
    commands::{Actor, Channel, CommandError, Request, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{Mutation, Placement},
    model::{self, AssemblyRequest, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest},
    parameters::BoundExpression,
    sketch::Datum,
};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(session: &mut Session, edits: Vec<Mutation>) -> Result<(), CommandError> {
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Author generic plant".into(),
        edits,
    })?;
    Ok(())
}
fn dimension(document: &Document, text: &str) -> Result<Dimension, DocumentError> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(document, text, "mm", None)
            .map_err(|e| invalid(e.to_string()))?,
    })
}
/// Creates a self-contained symbol source. No botanical or supplier dimensions
/// are inferred: these are explicitly generic planned-size defaults.
pub fn generic_definition(kind: PlantKind) -> Result<symbols::SymbolDefinition, CommandError> {
    let name = match kind {
        PlantKind::Tree => "Generic tree",
        PlantKind::Shrub => "Generic shrub",
    };
    let id = |role: u32| {
        EntityId::parse(&format!(
            "cad20200-0000-4000-8000-{:012x}",
            role + if kind == PlantKind::Tree {
                0x1000
            } else {
                0x2000
            }
        ))
        .expect("fixed plant identity")
    };
    let mut wire =
        serde_json::to_value(Document::new(name)?).map_err(|e| invalid(e.to_string()))?;
    wire["id"] = serde_json::to_value(id(1)).expect("identity serialization");
    let document = serde_json::from_value(wire).map_err(|e| invalid(e.to_string()))?;
    let mut session = Session::new(document)?;
    let mut parameter = |name: &str, value: &str| -> Result<EntityId, CommandError> {
        let mut entity = Entity::new(
            name,
            EntityData::Parameter {
                expression: value.into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        entity.id = id(if name == "height" { 2 } else { 3 });
        let id = entity.id;
        apply(&mut session, vec![Mutation::Insert(entity)])?;
        Ok(id)
    };
    let height = parameter(
        "height",
        if kind == PlantKind::Tree {
            "6000"
        } else {
            "800"
        },
    )?;
    let spread = parameter(
        "spread",
        if kind == PlantKind::Tree {
            "4000"
        } else {
            "1000"
        },
    )?;
    let mut members = vec![];
    let mut parts = vec![];
    let cylinders: &[(&str, &str, &str, &str)] = match kind {
        PlantKind::Tree => &[
            (
                "Trunk envelope",
                "=$spread * 0.035",
                "=$height * 0.45",
                "=0 mm",
            ),
            (
                "Crown envelope",
                "=$spread / 2",
                "=$height * 0.65",
                "=$height * 0.35",
            ),
        ],
        PlantKind::Shrub => &[("Shrub envelope", "=$spread / 2", "=$height", "=0 mm")],
    };
    for (index, &(label, radius, depth, elevation)) in cylinders.iter().enumerate() {
        let d = |text| dimension(session.document(), text);
        let mut request = PrimitiveRequest::new(
            label,
            PrimitiveParameters::Cylinder {
                radius: d(radius)?,
                height: d(depth)?,
                frame: PrimitiveFrame {
                    origin: [model::length(0.), model::length(0.), d(elevation)?],
                    ..Default::default()
                },
            },
        );
        request.feature = id(10 + index as u32 * 3);
        request.body = id(11 + index as u32 * 3);
        session.commit(
            model::prepare_primitive(&session, session.revision(), actor(), &request)?.prepared,
        )?;
        let mut instance = Entity::new(
            label,
            EntityData::Instance {
                component: request.body,
                placement: Placement::default(),
                placement_source: None,
            },
        );
        instance.id = id(12 + index as u32 * 3);
        members.push(instance.id);
        parts.push(request.body);
        apply(&mut session, vec![Mutation::Insert(instance)])?;
    }
    let mut assembly = AssemblyRequest::new(name, members, parts);
    assembly.assembly = id(20);
    assembly.metadata = id(21);
    session.commit(
        model::prepare_assembly(&session, session.revision(), actor(), &assembly)?.prepared,
    )?;
    let (original_plan, mut edits) = Source::create(Datum::Xy, "Canopy footprint")?;
    let plan = id(30);
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit {
            if entity.id == original_plan {
                entity.id = plan;
                if let EntityData::Sketch { plane, .. } = &mut entity.data {
                    *plane = id(31);
                }
            } else {
                entity.id = id(31);
            }
        }
    }
    apply(&mut session, edits)?;
    let mut source = Source::read(session.document(), plan)?.0;
    let center = id(32);
    source.sketch.points.push(crate::constraint_sketch::Point {
        id: center,
        position_mm: [0., 0.],
    });
    source.add_constraint(ConstraintKind::Fix { point: center });
    source.sketch.constraints.last_mut().expect("fix").id = id(33);
    // Both types share the true outer spread. Trees have an inner ring so the
    // monochrome plan remains distinguishable from a shrub of the same size.
    let ratios: &[f64] = if kind == PlantKind::Tree {
        &[1., 0.8]
    } else {
        &[1.]
    };
    for (index, &ratio) in ratios.iter().enumerate() {
        let curve = id(40 + index as u32 * 2);
        source.sketch.curves.push(Curve {
            id: curve,
            construction: false,
            kind: CurveKind::Circle {
                center,
                radius_mm: if kind == PlantKind::Tree { 2000. } else { 500. } * ratio,
            },
        });
        source.add_constraint(ConstraintKind::Diameter {
            curve,
            dimension: dimension(session.document(), &format!("=$spread * {ratio}"))?,
        });
        source.sketch.constraints.last_mut().expect("diameter").id = id(41 + index as u32 * 2);
    }
    let edits = source.replace(session.document(), plan)?;
    apply(&mut session, edits)?;
    let template = PlantTemplate {
        component: assembly.assembly,
        kind,
        height_parameter: height,
        spread_parameter: spread,
        plan,
    };
    apply(
        &mut session,
        vec![Mutation::Insert(template.entity(id(50))?)],
    )?;
    let mut request = symbols::SymbolAuthorRequest::new(
        name,
        assembly.assembly,
        symbols::SymbolKind::Dynamic,
        vec![height, spread],
        Some(symbols::SymbolPlan { sketch: plan }),
    );
    request.definition = id(60);
    request.symbol = id(61);
    session.commit(
        symbols::prepare_author(&session, session.revision(), actor(), &request)?.prepared,
    )?;
    symbols::read_definition(session.document(), request.definition).map_err(Into::into)
}
