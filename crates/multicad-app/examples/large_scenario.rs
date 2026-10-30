//! Generated large scenario for rebuild measurements (CAD-150).
//!
//! A field of `rows × columns` drilled plates, each built through the typed
//! command API as a parameter-driven Box, a Cylinder tool and a Cut Boolean
//! (three features per final Part). Every plate's width follows its row
//! parameter `row_<r>_width`; plate depth, plate height and the hole radius
//! are shared parameters. Editing one row parameter therefore changes only
//! that row's plates, while editing `hole_radius` changes every plate.
//!
//! `cargo run --release -p multicad-app --example large_scenario [ROWS COLUMNS]`
//! builds the field and prints cold, cached-warm and cached-edit compile
//! times. The release-budget harness measures it as `large_plate_field`.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        BodyInput, BooleanRequest, Operation, PrimitiveFrame, PrimitiveParameters,
        PrimitiveRequest, length, prepare_boolean, prepare_primitive,
    },
    parameters::BoundExpression,
};

type Error = Box<dyn std::error::Error>;

pub const DEFAULT_ROWS: usize = 10;
pub const DEFAULT_COLUMNS: usize = 20;
const PITCH_X: f64 = 150.;
const PITCH_Y: f64 = 100.;

pub struct Scenario {
    pub session: Session,
    pub rows: usize,
    pub columns: usize,
    /// Final Part per plate, row-major.
    pub plates: Vec<EntityId>,
}

pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn parameter(name: &str, expression: &str) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn bound(session: &Session, text: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, "mm", None)?,
    })
}
pub fn row_parameter(row: usize) -> String {
    format!("row_{row}_width")
}

pub fn build(rows: usize, columns: usize) -> Result<Scenario, Error> {
    let mut parameters = vec![
        parameter("plate_depth", "60"),
        parameter("plate_height", "12"),
        parameter("hole_radius", "8"),
    ];
    for row in 0..rows {
        parameters.push(parameter(&row_parameter(row), &format!("{}", 90 + row)));
    }
    let edits: Vec<_> = parameters.into_iter().map(Mutation::Insert).collect();
    let mut session = Session::new(Document::new("Large plate field")?.edited(&edits)?)?;
    let mut plates = Vec::with_capacity(rows * columns);
    for row in 0..rows {
        for column in 0..columns {
            let (x, y) = (column as f64 * PITCH_X, row as f64 * PITCH_Y);
            let plate = PrimitiveRequest::new(
                format!("Plate {row}.{column}"),
                PrimitiveParameters::Box {
                    size: [
                        bound(&session, &format!("=${}", row_parameter(row)))?,
                        bound(&session, "=$plate_depth")?,
                        bound(&session, "=$plate_height")?,
                    ],
                    frame: PrimitiveFrame {
                        origin: [length(x), length(y), length(0.)],
                        ..Default::default()
                    },
                },
            );
            let prepared = prepare_primitive(&session, session.revision(), actor(), &plate)?;
            session.commit(prepared.prepared)?;
            let hole = PrimitiveRequest::new(
                format!("Hole {row}.{column}"),
                PrimitiveParameters::Cylinder {
                    radius: bound(&session, "=$hole_radius")?,
                    height: length(40.),
                    frame: PrimitiveFrame {
                        origin: [length(x + 30.), length(y + 30.), length(-10.)],
                        ..Default::default()
                    },
                },
            );
            let prepared = prepare_primitive(&session, session.revision(), actor(), &hole)?;
            session.commit(prepared.prepared)?;
            let cut = BooleanRequest::new(
                Operation::Cut,
                BodyInput {
                    body: plate.body,
                    producer: plate.feature,
                },
                vec![BodyInput {
                    body: hole.body,
                    producer: hole.feature,
                }],
            );
            let prepared = prepare_boolean(&session, session.revision(), actor(), &cut)?;
            session.commit(prepared.prepared)?;
            plates.push(plate.body);
        }
    }
    Ok(Scenario {
        session,
        rows,
        columns,
        plates,
    })
}

pub fn set_parameter(session: &mut Session, name: &str, expression: &str) -> Result<(), Error> {
    let mut entity = session
        .document()
        .entities()
        .find(|e| e.name == name && matches!(e.data, EntityData::Parameter { .. }))
        .ok_or_else(|| format!("Missing parameter {name}"))?
        .clone();
    let EntityData::Parameter { expression: e, .. } = &mut entity.data else {
        unreachable!()
    };
    *e = expression.into();
    let prepared = session.prepare(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Edit parameter".into(),
        edits: vec![Mutation::Replace(entity)],
    })?;
    session.commit(prepared)?;
    Ok(())
}

#[allow(dead_code)]
fn main() -> Result<(), Error> {
    use multicad_model::{Compiler, EvaluationCache};
    use std::time::Instant;
    let mut args = std::env::args().skip(1);
    let rows = args.next().map_or(Ok(DEFAULT_ROWS), |a| a.parse())?;
    let columns = args.next().map_or(Ok(DEFAULT_COLUMNS), |a| a.parse())?;
    let start = Instant::now();
    let mut s = build(rows, columns)?;
    println!("built {} plates in {:?}", s.plates.len(), start.elapsed());
    let compiler = Compiler::default();
    let start = Instant::now();
    let model = compiler.compile(s.session.document(), s.session.revision())?;
    println!(
        "cold compile: {:?} ({} shapes)",
        start.elapsed(),
        model.snapshot().shapes.len()
    );
    let mut cache = EvaluationCache::default();
    compiler.compile_cached(&mut cache, s.session.document(), s.session.revision(), None)?;
    let start = Instant::now();
    compiler.compile_cached(&mut cache, s.session.document(), s.session.revision(), None)?;
    println!("cached warm compile: {:?}", start.elapsed());
    set_parameter(&mut s.session, &row_parameter(0), "95")?;
    let start = Instant::now();
    compiler.compile_cached(&mut cache, s.session.document(), s.session.revision(), None)?;
    let stats = cache.last_compile();
    println!(
        "cached row edit: {:?} (evaluated {}, reused {})",
        start.elapsed(),
        stats.evaluated.len(),
        stats.reused.len()
    );
    Ok(())
}
