//! Creates fresh editable native projects for interactive advanced-model review.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, *},
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, TopologyKind},
    sketch::Datum,
    sketch_regions,
};
use multicad_model::{Compiler, Model};
use serde_json::{Value, json};
use std::{
    io::{Cursor, Write},
    path::Path,
};
type Error = Box<dyn std::error::Error>;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn base() -> Result<Document, Error> {
    let mut doc = Document::new("Advanced modelling review")?;
    for (name, value, unit) in [
        ("width", 40., "mm"),
        ("height", 20., "mm"),
        ("wall", 2., "mm"),
        ("draft", 5., "deg"),
        ("copies", 3., "count"),
        ("spacing", 45., "mm"),
        ("mirror_z", 0., "mm"),
        ("step_angle", 90., "deg"),
        ("bevel", 2., "mm"),
        ("second", 3., "mm"),
        ("bevel_angle", 45., "deg"),
        ("section_radius", 4., "mm"),
        ("path_length", 50., "mm"),
    ] {
        doc.insert(Entity::new(
            name,
            EntityData::Parameter {
                expression: value.to_string(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        ))?;
    }
    Ok(doc)
}
fn bound(doc: &Document, name: &str, unit: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(doc, &format!("=${name}"), unit, None)?,
    })
}
fn compile(session: &Session) -> Result<Model, Error> {
    Ok(Compiler::default().compile(session.document(), session.revision())?)
}
fn stock() -> Result<(Session, BodyInput), Error> {
    let mut session = Session::new(base()?)?;
    let request = PrimitiveRequest::new(
        "Stock 40 × 30 × 20 mm",
        PrimitiveParameters::Box {
            size: [
                bound(session.document(), "width", "mm")?,
                length(30.),
                bound(session.document(), "height", "mm")?,
            ],
            frame: Default::default(),
        },
    );
    session.commit(model::prepare_primitive(&session, 0, actor(), &request)?.prepared)?;
    Ok((
        session,
        BodyInput {
            body: request.body,
            producer: request.feature,
        },
    ))
}
fn selected_face(
    native: &Model,
    input: &BodyInput,
    normal: [f64; 3],
) -> Result<GeometryIntent, Error> {
    let snapshot = &native.snapshot().shapes[&input.producer].topology;
    let face=snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal().into_iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).ok_or("Expected source face is missing")?;
    Ok(GeometryIntent::capture(snapshot, face.key)?)
}
fn save_new(
    directory: &Path,
    name: &str,
    session: &Session,
    selection: Value,
    expect: Option<(EntityId, f64, u32)>,
) -> Result<Value, Error> {
    let native = compile(session)?;
    if !native.snapshot().references.all_resolved() {
        return Err("Generated project has unresolved references".into());
    }
    if let Some((body, volume, count)) = expect {
        let solid = native.solid(body).ok_or("Expected output is absent")?;
        if (solid.volume_mm3()? - volume).abs() > 1e-4 || solid.solid_count()? != count {
            return Err(
                format!("{name}: native metrics differ from expected analytic result").into(),
            );
        }
    }
    let bodies=native.snapshot().bodies.iter().map(|(body,producer)|{
        let solid=native.solid(*body).ok_or("Physical body is absent")?;let bounds=solid.bounds()?;
        Ok(json!({"part":body,"producer":producer,"volume_mm3":solid.volume_mm3()?,"solid_count":solid.solid_count()?,"bounds_mm":{"min":bounds.min,"max":bounds.max}}))
    }).collect::<Result<Vec<_>,Error>>()?;
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    let reopened = project_file::read(
        &mut Cursor::new(&bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if reopened.project.document != *session.document() {
        return Err("Review file roundtrip changed source".into());
    }
    Compiler::default().compile(&reopened.project.document, 0)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        json!({"file":name,"document":session.document().id(),"source_hash":native.snapshot().stamp.source_hash,"selection":selection,"bodies":bodies,"expected":expect.map(|(part,volume,solids)|json!({"part":part,"volume_mm3":volume,"solid_count":solids}))}),
    )
}
fn apply(session: &mut Session, request: &AdvancedRequest) -> Result<(), Error> {
    let prepared = model::prepare_advanced(session, session.revision(), actor(), request)?;
    Compiler::default().compile(prepared.prepared.document(), session.revision())?;
    session.commit(prepared.prepared)?;
    Ok(())
}
fn section(
    doc: &mut Document,
    name: &str,
    z: f64,
    radius: f64,
    driven: bool,
) -> Result<(ProfileSource, EntityId), Error> {
    let (sketch, edits) = Source::create(Datum::Xy, name)?;
    *doc = doc.edited(&edits)?;
    let EntityData::Sketch { plane, .. } = doc.get(sketch).ok_or("Missing sketch")?.data else {
        return Err("Not a sketch".into());
    };
    let mut plane = doc.get(plane).ok_or("Missing plane")?.clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        return Err("Not a plane".into());
    };
    origin_mm[2] = z;
    *doc = doc.edited(&[Mutation::Replace(plane)])?;
    let mut source = Source::read(doc, sketch)?.0;
    let center = source.add_point([0., 0.]);
    let seam = EntityId::new();
    source.sketch.curves.push(Curve {
        id: seam,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: radius,
        },
    });
    let region = sketch_regions::extract(&source)
        .regions
        .first()
        .ok_or("Circle region is missing")?
        .intent();
    source.add_constraint(ConstraintKind::Fix { point: center });
    if driven {
        source.add_constraint(ConstraintKind::Radius {
            curve: seam,
            dimension: bound(doc, "section_radius", "mm")?,
        });
    }
    *doc = doc.edited(&source.replace(doc, sketch)?)?;
    Ok((ProfileSource { sketch, region }, seam))
}
fn sweep_path(doc: &mut Document) -> Result<SketchPathSource, Error> {
    let (sketch, edits) = Source::create(Datum::Xz, "Sweep path +Z")?;
    *doc = doc.edited(&edits)?;
    let mut source = Source::read(doc, sketch)?.0;
    let a = source.add_point([0., 0.]);
    let b = source.add_point([0., 50.]);
    let line = source.add_line(a, b);
    source.add_constraint(ConstraintKind::Fix { point: a });
    source.add_constraint(ConstraintKind::Vertical { line });
    source.add_constraint(ConstraintKind::Length {
        line,
        dimension: bound(doc, "path_length", "mm")?,
    });
    *doc = doc.edited(&source.replace(doc, sketch)?)?;
    Ok(SketchPathSource {
        sketch,
        curves: vec![line],
    })
}
fn generate(directory: &Path) -> Result<(), Error> {
    // A fresh directory prevents accidentally replacing human-edited review files.
    std::fs::create_dir(directory)?;
    let mut entries = vec![];
    for kind in [
        AdvancedKind::Shell,
        AdvancedKind::Draft,
        AdvancedKind::Mirror,
        AdvancedKind::LinearPattern,
        AdvancedKind::CircularPattern,
    ] {
        let (mut session, input) = stock()?;
        let native = compile(&session)?;
        let label = match kind {
            AdvancedKind::Shell => "shell",
            AdvancedKind::Draft => "draft",
            AdvancedKind::Mirror => "mirror",
            AdvancedKind::LinearPattern => "linear-pattern",
            AdvancedKind::CircularPattern => "circular-pattern",
            _ => unreachable!(),
        };
        entries.push(save_new(
            directory,
            &format!("{label}-input.multicad"),
            &session,
            json!({"body":input.body,"producer":input.producer}),
            Some((input.body, 24000., 1)),
        )?);
        let (mut parameters, mut captures, volume, count) =
            (AdvancedParameters::default_for(kind), vec![], 24000., 1);
        let (volume, count) = match &mut parameters {
            AdvancedParameters::Shell(p) => {
                p.thickness = bound(session.document(), "wall", "mm")?;
                captures.push(selected_face(&native, &input, [0., 0., 1.])?);
                (24000. - 36. * 26. * 18., 1)
            }
            AdvancedParameters::Draft(p) => {
                p.angle = bound(session.document(), "draft", "deg")?;
                captures.push(selected_face(&native, &input, [1., 0., 0.])?);
                (24000. - 30. * 0.5 * 400. * 5f64.to_radians().tan(), 1)
            }
            AdvancedParameters::Mirror(p) => {
                p.plane.origin[2] = bound(session.document(), "mirror_z", "mm")?;
                (volume, count)
            }
            AdvancedParameters::LinearPattern(p) => {
                p.count = bound(session.document(), "copies", "count")?;
                p.step[0] = bound(session.document(), "spacing", "mm")?;
                (72000., 3)
            }
            AdvancedParameters::CircularPattern(p) => {
                p.count = bound(session.document(), "copies", "count")?;
                p.step_angle = bound(session.document(), "step_angle", "deg")?;
                (72000., 3)
            }
            _ => unreachable!(),
        };
        let request = AdvancedRequest::on_body(input, parameters, captures);
        apply(&mut session, &request)?;
        entries.push(save_new(
            directory,
            &format!("{label}-result.multicad"),
            &session,
            serde_json::to_value(&request)?,
            Some((request.body, volume, count)),
        )?);
    }
    for sided_angle in [false, true] {
        let (mut session, input) = stock()?;
        let native = compile(&session)?;
        let input_name = if sided_angle {
            "chamfer-angle-input.multicad"
        } else {
            "chamfer-distances-input.multicad"
        };
        entries.push(save_new(
            directory,
            input_name,
            &session,
            json!({"body":input.body,"producer":input.producer}),
            Some((input.body, 24000., 1)),
        )?);
        let topology = &native.snapshot().shapes[&input.producer].topology;
        let edge=topology.elements.iter().find(|e|e.key.kind==TopologyKind::Edge&&matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm,b_mm})if a_mm[0]==40.&&b_mm[0]==40.&&a_mm[1]==30.&&b_mm[1]==30.)).ok_or("Expected stock edge is missing")?;
        let capture = GeometryIntent::capture(topology, edge.key)?;
        let side = selected_face(&native, &input, [1., 0., 0.])?;
        let mut request = EdgeTreatmentRequest::new(
            input.clone(),
            EdgeTreatmentParameters {
                profile: EdgeTreatmentProfile::Chamfer,
                size: bound(session.document(), "bevel", "mm")?,
            },
            vec![capture.clone(), side.clone()],
        );
        let sides = vec![EdgeSide {
            edge: capture.id,
            face: side.id,
        }];
        request.chamfer = Some(if sided_angle {
            ChamferSettings::DistanceAngle {
                angle: bound(session.document(), "bevel_angle", "deg")?,
                sides,
            }
        } else {
            ChamferSettings::TwoDistances {
                second: bound(session.document(), "second", "mm")?,
                sides,
            }
        });
        // The request owns edge captures; explicit side captures enter the same canonical link list.
        let prepared =
            model::prepare_edge_treatment(&session, session.revision(), actor(), &request)?;
        Compiler::default().compile(prepared.prepared.document(), session.revision())?;
        session.commit(prepared.prepared)?;
        let name = if sided_angle {
            "chamfer-angle-result.multicad"
        } else {
            "chamfer-distances-result.multicad"
        };
        entries.push(save_new(
            directory,
            name,
            &session,
            serde_json::to_value(&request)?,
            Some((input.body, if sided_angle { 23960. } else { 23940. }, 1)),
        )?);
    }
    for loft in [false, true] {
        let mut doc = base()?;
        let (first, a) = section(&mut doc, "Section at Z 0", 0., 4., true)?;
        let (mut profiles, mut seams) = (vec![first], vec![a]);
        let mut path = None;
        if loft {
            let (second, b) = section(&mut doc, "Section at Z 30", 30., 8., false)?;
            profiles.push(second);
            seams.push(b);
        } else {
            path = Some(sweep_path(&mut doc)?);
        }
        let mut session = Session::new(doc)?;
        let label = if loft { "loft" } else { "sweep" };
        entries.push(save_new(
            directory,
            &format!("{label}-input.multicad"),
            &session,
            json!({"profiles":profiles,"seams":seams,"path":path}),
            None,
        )?);
        let parameters = if loft {
            AdvancedParameters::Loft(LoftParameters { ruled: true, seams })
        } else {
            AdvancedParameters::Sweep(Default::default())
        };
        let request =
            AdvancedRequest::new_body(format!("Review {label}"), parameters, profiles, path);
        apply(&mut session, &request)?;
        let volume = if loft {
            std::f64::consts::PI * 30. / 3. * (16. + 32. + 64.)
        } else {
            std::f64::consts::PI * 16. * 50.
        };
        entries.push(save_new(
            directory,
            &format!("{label}-result.multicad"),
            &session,
            serde_json::to_value(&request)?,
            Some((request.body, volume, 1)),
        )?);
    }
    let manifest = json!({"generator":"advanced_review","assumptions":["Exact native BRep compiler and canonical parametric commands; no manufacturing geometry from meshes.","Shell requires explicit opening faces, with positive wall thickness inward.","Patterns author one compound Part; members are separate solids, without independent material or motion.","Sweep path is an open authored straight-line chain; loft uses ordered single-outer sections and explicit seams.","These files support manual review; generated/native metrics are not evidence of physical UI acceptance or Windows/Linux performance."],"projects":entries});
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join("manifest.json"))?;
    println!("{}", directory.display());
    Ok(())
}
fn main() -> Result<(), Error> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("Usage: cargo run -p multicad-app --example advanced_review -- NEW_DIRECTORY")?;
    generate(Path::new(&directory))
}
