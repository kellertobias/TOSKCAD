//! Editable canonical Parameters → constrained rectangle → exact native prism.
//! The model regression imports this same fixture so operator review cannot
//! silently drift from the geometry exercised by automated checks.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{
        ConstraintKind, Dimension,
        binding::{self, DimensionInput},
        source::Source,
    },
    document::{Definition, Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{self, CreatedBody, Extent, ExtrudeParameters, ExtrudeRequest, Termination},
    parameters::BoundExpression,
    project_file::{self, ContentHash, ProjectFile},
    sketch::Datum,
};
use multicad_model::{Compiler, Model};
use std::{io::Write, path::Path};
pub type Error = Box<dyn std::error::Error>;
pub struct Fixture {
    pub session: Session,
    pub width: EntityId,
    pub height: EntityId,
    pub stock: EntityId,
    pub depth: EntityId,
    pub sketch: EntityId,
    pub width_dimension: EntityId,
    pub height_dimension: EntityId,
    pub output: CreatedBody,
    pub table: EntityId,
}
pub fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn parameter(name: &str, expression: &str, description: &str, exposed: bool) -> Entity {
    let mut e = Entity::new(
        name,
        EntityData::Parameter {
            expression: expression.into(),
            unit: "mm".into(),
            exposed,
            inputs: vec![],
        },
    );
    e.description = description.into();
    e
}
pub fn order_mutation(
    document: &Document,
    table: EntityId,
    rows: &[EntityId],
) -> Result<Mutation, Error> {
    let mut e = document
        .get(table)
        .ok_or("Missing parameter table metadata")?
        .clone();
    e.data = EntityData::Extension {
        definition: Definition {
            type_id: "core.parameter_table".into(),
            schema_version: 1,
            data: serde_json::to_value(rows)?,
        },
        inputs: rows.iter().map(|id| Link::new(*id, "row")).collect(),
    };
    Ok(Mutation::Replace(e))
}
pub fn request(session: &Session, label: &str, edits: Vec<Mutation>) -> Request {
    Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: label.into(),
        edits,
    }
}
/// Generator preflight: source preparation plus genuine native compilation,
/// then the atomic commit. This helper does not claim the UI calls it.
pub fn commit_checked(f: &mut Fixture, label: &str, edits: Vec<Mutation>) -> Result<(), Error> {
    let prepared = f.session.prepare(request(&f.session, label, edits))?;
    Compiler::default().compile(prepared.document(), f.session.revision())?;
    f.session.commit(prepared)?;
    Ok(())
}
pub fn seed() -> Result<Fixture, Error> {
    let width = parameter(
        "width",
        "120",
        "Driving panel span; changes the sketch width",
        true,
    );
    let height = parameter(
        "height",
        "80",
        "Driving panel rise; changes the sketch height",
        true,
    );
    let stock = parameter(
        "stock",
        "6",
        "Stock thickness used by the derived extrusion depth",
        true,
    );
    let depth = parameter(
        "depth",
        "=$stock + 2 mm",
        "Derived extrusion depth; retains its stock UUID binding",
        false,
    );
    let rows = [width.id, height.id, stock.id, depth.id];
    let mut table = Entity::new(
        "Parameter table",
        EntityData::Extension {
            definition: Definition {
                type_id: "core.parameter_table".into(),
                schema_version: 1,
                data: serde_json::to_value(rows)?,
            },
            inputs: rows.iter().map(|id| Link::new(*id, "row")).collect(),
        },
    );
    table.description = "Canonical row order shared with the Parameters table".into();
    let table_id = table.id;
    let (sketch, mut edits) = Source::create(Datum::Xy, "Fully constrained parameter rectangle")?;
    edits.extend(
        [width, height, stock, depth, table]
            .into_iter()
            .map(Mutation::Insert),
    );
    let document = Document::new("Parameters drive a real panel")?.edited(&edits)?;
    let mut session = Session::new(document)?;
    let mut source = Source::read(session.document(), sketch)?.0;
    let bottom = source.add_rectangle([0., 0.], [60., 40.])?;
    source.add_constraint(ConstraintKind::Fix {
        point: source.sketch.points[0].id,
    });
    let width_dimension = source.add_constraint(ConstraintKind::Length {
        line: bottom,
        dimension: model::length(60.),
    });
    let height_dimension = source.add_constraint(ConstraintKind::Length {
        line: source.sketch.curves[1].id,
        dimension: model::length(40.),
    });
    for (id, name) in [(width_dimension, "width"), (height_dimension, "height")] {
        source = binding::draft_dimension(
            session.document(),
            &source,
            id,
            &DimensionInput::Expression {
                text: format!("=${name}"),
                unit: "mm".into(),
            },
            false,
        )?;
    }
    let edit =
        multicad_solver::prepare_edit(&session, session.revision(), actor(), sketch, source)?;
    session.commit(edit.preview.ok_or("Initial sketch did not solve")?)?;
    let evaluated = multicad_solver::evaluate_document_sketch(
        session.document(),
        session.revision(),
        sketch,
        None,
    )?;
    let regions = evaluated.regions()?;
    if regions.regions.len() != 1 || !regions.diagnostics.is_empty() {
        return Err("Expected one exact solved rectangle".into());
    }
    let mut extrusion = ExtrudeRequest::new(
        sketch,
        regions.regions.iter().map(|r| r.intent()).collect(),
        ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: Dimension::Expression {
                        expression: BoundExpression::bind(
                            session.document(),
                            "=$depth",
                            "mm",
                            None,
                        )?,
                    },
                },
            },
            ..Default::default()
        },
    );
    extrusion.outputs[0].name = "Parameter-driven panel".into();
    let prepared = model::prepare_extrude(&session, session.revision(), actor(), &extrusion)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision())?;
    let output = prepared.outputs[0].clone();
    session.commit(prepared.prepared)?;
    Ok(Fixture {
        session,
        width: rows[0],
        height: rows[1],
        stock: rows[2],
        depth: rows[3],
        sketch,
        width_dimension,
        height_dimension,
        output,
        table: table_id,
    })
}
pub fn compile(f: &Fixture) -> Result<Model, Error> {
    Ok(Compiler::default().compile(f.session.document(), f.session.revision())?)
}
pub fn verify(f: &Fixture, expected: [f64; 3]) -> Result<(), Error> {
    let model = compile(f)?;
    let shape = model
        .snapshot()
        .shapes
        .get(&f.output.feature)
        .ok_or("Missing real extrusion")?;
    let native = model
        .solid(f.output.feature)
        .ok_or("Missing exact native prism")?;
    let volume = expected.into_iter().product::<f64>();
    if shape.solid_count != 1
        || native.solid_count()? != 1
        || (native.volume_mm3()? - volume).abs() > 1e-5
        || (shape.volume_mm3 - volume).abs() > 1e-5
        || shape.bounds.min.iter().any(|v| v.abs() > 1e-5)
        || shape
            .bounds
            .max
            .iter()
            .zip(expected)
            .any(|(a, b)| (a - b).abs() > 1e-5)
    {
        return Err("Native panel differs from independently expected dimensions/volume".into());
    }
    let solved = multicad_solver::evaluate_document_sketch(
        f.session.document(),
        f.session.revision(),
        f.sketch,
        None,
    )?;
    if solved.evaluation.diagnostics.state != multicad_solver::SolveState::FullyConstrained
        || solved.evaluation.diagnostics.degrees_of_freedom != 0
    {
        return Err("Rectangle is not genuinely fully constrained".into());
    }
    if (solved.regions()?.regions[0].area_mm2 - expected[0] * expected[1]).abs() > 1e-5 {
        return Err("Solved profile area disagrees with independent rectangle dimensions".into());
    }
    for (id, want) in [
        (f.width_dimension, expected[0]),
        (f.height_dimension, expected[1]),
    ] {
        let actual = solved
            .resolved_rhs
            .get(&id)
            .ok_or("Missing parameter-bound length RHS")?;
        if (actual.value - want).abs() > 1e-5 || actual.unit != "mm" {
            return Err(
                "Driving constraint did not resolve its bound parameter in millimeters".into(),
            );
        }
    }
    Ok(())
}
pub fn archive(document: &Document) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(document.clone())?,
        &mut bytes,
        Default::default(),
    )?;
    Ok(bytes)
}
pub fn reopen(bytes: &[u8]) -> Result<Session, Error> {
    let project = project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?
    .project;
    Ok(Session::new(project.document)?)
}
fn save(
    directory: &Path,
    name: &str,
    f: &Fixture,
    expected: [f64; 3],
) -> Result<serde_json::Value, Error> {
    verify(f, expected)?;
    let bytes = archive(f.session.document())?;
    let reopened = reopen(&bytes)?;
    if reopened.document() != f.session.document() {
        return Err("Project archive changed canonical parameter/sketch/solid source".into());
    }
    let model = Compiler::default().compile(reopened.document(), reopened.revision())?;
    let exact = model
        .solid(f.output.feature)
        .ok_or("Reopened native feature missing")?;
    if (exact.volume_mm3()? - expected.into_iter().product::<f64>()).abs() > 1e-5 {
        return Err("Reopened native volume mismatch".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        serde_json::json!({"file":name,"document":f.session.document().id(),"sketch":f.sketch,"part":f.output.body,"feature":f.output.feature,"width_dimension":f.width_dimension,"height_dimension":f.height_dimension,"expected_bounds_mm":[[0.,0.,0.],expected],"expected_volume_mm3":expected.into_iter().product::<f64>(),"source_hash":ContentHash::document(f.session.document())?,"archive_sha256":ContentHash::of(&bytes).to_string(),"revision":f.session.revision()}),
    )
}
pub fn generate(directory: &Path) -> Result<serde_json::Value, Error> {
    std::fs::create_dir(directory)?;
    let mut f = seed()?;
    let mut projects = vec![save(
        directory,
        "01-parameter-panel.multicad",
        &f,
        [120., 80., 8.],
    )?];
    let mut edits = vec![];
    for (id, value) in [(f.width, "150"), (f.height, "100"), (f.stock, "8")] {
        let mut e = f
            .session
            .document()
            .get(id)
            .ok_or("Missing parameter")?
            .clone();
        let EntityData::Parameter { expression, .. } = &mut e.data else {
            return Err("Not a parameter".into());
        };
        *expression = value.into();
        edits.push(Mutation::Replace(e));
    }
    commit_checked(&mut f, "Resize panel through parameters", edits)?;
    projects.push(save(
        directory,
        "02-resized-panel.multicad",
        &f,
        [150., 100., 10.],
    )?);
    let mut span = f
        .session
        .document()
        .get(f.width)
        .ok_or("Missing width")?
        .clone();
    span.name = "span".into();
    span.description = "Renamed original width; the sketch keeps its UUID binding".into();
    let EntityData::Parameter {
        expression,
        unit,
        exposed,
        ..
    } = &mut span.data
    else {
        return Err("Not a parameter".into());
    };
    *expression = "15".into();
    *unit = "cm".into();
    *exposed = false;
    let mut stock = f
        .session
        .document()
        .get(f.stock)
        .ok_or("Missing stock")?
        .clone();
    stock.name = "material_thickness".into();
    let width_decoy = parameter(
        "width",
        "999",
        "New same-name parameter; does not steal the existing sketch binding",
        false,
    );
    let stock_decoy = parameter(
        "stock",
        "999",
        "New same-name parameter; does not steal the existing depth binding",
        false,
    );
    let order = [
        f.depth,
        f.height,
        f.width,
        f.stock,
        width_decoy.id,
        stock_decoy.id,
    ];
    let edits = vec![
        Mutation::Replace(span),
        Mutation::Replace(stock),
        Mutation::Insert(width_decoy),
        Mutation::Insert(stock_decoy),
        order_mutation(f.session.document(), f.table, &order)?,
    ];
    commit_checked(
        &mut f,
        "Rename, change unit/exposure, reorder and reuse former names",
        edits,
    )?;
    projects.push(save(
        directory,
        "03-renamed-reordered-panel.multicad",
        &f,
        [150., 100., 10.],
    )?);
    let manifest = serde_json::json!({"generator":"parameter_workflow_review","parameters":{"original_width":f.width,"height":f.height,"original_stock":f.stock,"derived_depth":f.depth,"row_order_metadata":f.table},"projects":projects,"controls":["Edit width/height/stock in the first project, then Apply & Rebuild. Width and height drive solver dimensions; depth is =$stock + 2 mm.","In the third project span is the original width expressed in cm; material_thickness is the original stock. New width/stock rows are intentional decoys and must not drive existing bindings.","The canonical sketch retains authored guesses; displayed and manufactured geometry comes from the current constrained solve."],"verification":{"native_geometry":true,"fully_constrained":true,"archive_reopen":true,"physical_native_ui_acceptance":false,"windows_linux_runtime":false}});
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join("manifest.json"))?;
    Ok(manifest)
}
#[cfg(not(test))]
fn main() -> Result<(), Error> {
    let path = std::env::args_os().nth(1).ok_or(
        "Usage: cargo run -p multicad-app --example parameter_workflow_review -- NEW_DIRECTORY",
    )?;
    let path = std::path::PathBuf::from(path);
    let manifest = generate(&path)?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}
