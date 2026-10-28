//! Editable rail-driven receiver examples with exact native geometry evidence.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent},
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
fn bound(s: &Session, expression: &str, unit: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(s.document(), expression, unit, None)?,
    })
}
fn native(s: &Session) -> Result<Model, Error> {
    Ok(Compiler::default().compile(s.document(), s.revision())?)
}
fn commit(s: &mut Session, p: ModelPrepared) -> Result<(), Error> {
    Compiler::default().compile(p.prepared.document(), s.revision())?;
    s.commit(p.prepared)?;
    Ok(())
}
fn top(native: &Model, source: EntityId, height: f64) -> Result<AlignmentInput, Error> {
    let topology = &native
        .snapshot()
        .shapes
        .get(&source)
        .ok_or("Missing exact producer")?
        .topology;
    let face = topology
        .elements
        .iter()
        .find(|e| match &e.geometry {
            Some(AnalyticGeometry::Plane { frame }) => {
                let n = frame.u[0] * frame.v[1] - frame.u[1] * frame.v[0];
                n > 0.999 && (frame.origin_mm[2] - height).abs() < 1e-7
            }
            _ => false,
        })
        .ok_or("Missing upward entry face")?;
    Ok(AlignmentInput {
        source,
        intent: GeometryIntent::capture(topology, face.key)?,
    })
}
struct Fixture {
    session: Session,
    transfer: TransferRequest,
    members: Vec<ComponentPlacementRequest>,
    outer: ComponentPlacementRequest,
    rail: EntityId,
}
fn fixture() -> Result<Fixture, Error> {
    let mut doc = Document::new("Rail-driven wood, printed and sheet receivers")?;
    for (name, value, unit) in [
        ("hole_pitch", "32", "mm"),
        ("hole_count", "2", "count"),
        ("rail_offset", "10", "mm"),
    ] {
        doc.insert(Entity::new(
            name,
            EntityData::Parameter {
                expression: value.into(),
                unit: unit.into(),
                exposed: true,
                inputs: vec![],
            },
        ))?;
    }
    let mut s = Session::new(doc)?;
    let rail = PrimitiveRequest::new(
        "Independent drawer rail",
        PrimitiveParameters::Box {
            size: [130., 20., 4.].map(length),
            frame: Default::default(),
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &rail)?;
    commit(&mut s, p)?;
    let cutter = PrimitiveRequest::new(
        "Rail hole pattern source",
        PrimitiveParameters::Cylinder {
            radius: length(3.),
            height: length(6.),
            frame: PrimitiveFrame {
                origin: [20., 10., -1.].map(length),
                ..Default::default()
            },
        },
    );
    let p = prepare_primitive(&s, s.revision(), actor(), &cutter)?;
    commit(&mut s, p)?;
    let pattern = AdvancedRequest::on_body(
        BodyInput {
            body: cutter.body,
            producer: cutter.feature,
        },
        AdvancedParameters::LinearPattern(LinearPatternParameters {
            count: bound(&s, "=$hole_count", "count")?,
            step: [bound(&s, "=$hole_pitch", "mm")?, length(0.), length(0.)],
            merge: false,
        }),
        vec![],
    );
    let p = prepare_advanced(&s, s.revision(), actor(), &pattern)?;
    commit(&mut s, p)?;
    let cut = BooleanRequest::new(
        Operation::Cut,
        BodyInput {
            body: rail.body,
            producer: rail.feature,
        },
        vec![BodyInput {
            body: cutter.body,
            producer: pattern.feature,
        }],
    );
    let p = prepare_boolean(&s, s.revision(), actor(), &cut)?;
    commit(&mut s, p)?;
    let mut parts = vec![];
    for (name, height, density, color) in [
        ("Wood receiver", 18., 700., [177, 133, 83, 255]),
        ("Printed receiver", 8., 1240., [75, 132, 172, 255]),
        ("Sheet receiver", 2., 7850., [169, 180, 194, 255]),
    ] {
        let p = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: [200., 60., height].map(length),
                frame: Default::default(),
            },
        );
        let ready = prepare_primitive(&s, s.revision(), actor(), &p)?;
        commit(&mut s, ready)?;
        let material = Entity::new(
            format!("{name} material"),
            EntityData::Material {
                density_kg_m3: Some(density),
                color_rgba: color,
                section_hatch: None,
            },
        );
        let mut part = s.document().get(p.body).ok_or("Missing receiver")?.clone();
        if let EntityData::Part {
            material: assigned,
            tags,
            ..
        } = &mut part.data
        {
            *assigned = Some(material.id);
            tags.insert("rail-receivers".into());
        }
        let edit = s.prepare(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Receiver material and export tag".into(),
            edits: vec![Mutation::Insert(material), Mutation::Replace(part)],
        })?;
        s.commit(edit)?;
        parts.push(p);
    }
    let mut members = vec![];
    for (body, translation) in [
        (rail.body, [10., 0., 80.]),
        (parts[0].body, [0., 0., 0.]),
        (parts[1].body, [0., 0., 40.]),
        (parts[2].body, [0., 0., 60.]),
    ] {
        let mut p = ComponentPlacementRequest::new(
            body,
            NumericTransform {
                translation: translation.map(length),
                ..Default::default()
            },
        );
        if body == rail.body {
            p.parameters.translation[0] = bound(&s, "=$rail_offset", "mm")?;
        }
        let ready = prepare_place_component(&s, s.revision(), actor(), &p)?;
        s.commit(ready.prepared)?;
        members.push(p);
    }
    let mut templates = parts.iter().map(|p| p.body).collect::<Vec<_>>();
    templates.push(rail.body);
    let assembly = AssemblyRequest::new(
        "Reusable rail and receiver fixture",
        members.iter().map(|i| i.instance).collect(),
        templates,
    );
    let p = prepare_assembly(&s, s.revision(), actor(), &assembly)?;
    s.commit(p.prepared)?;
    let outer = ComponentPlacementRequest::new(
        assembly.assembly,
        NumericTransform {
            translation: [300., 500., 100.].map(length),
            angle: angle(30.),
            ..Default::default()
        },
    );
    let p = prepare_place_component(&s, s.revision(), actor(), &outer)?;
    s.commit(p.prepared)?;
    let native = native(&s)?;
    let source = top(&native, cut.feature, 4.)?;
    let path = |index: usize| OccurrencePath {
        root: outer.instance,
        members: vec![members[index].instance],
    };
    let mut transfer = TransferRequest::new(
        "Rail mounting pattern",
        TransferSource {
            component: rail.body,
            path: path(0),
            selection: TransferSelection::FaceHoles {
                face: source.intent.id,
            },
        },
    );
    transfer.references.push(AlignmentInput {
        source: rail.body,
        intent: source.intent,
    });
    for (i, part) in parts.iter().enumerate() {
        let height = [18., 8., 2.][i];
        let face = top(&native, part.feature, height)?;
        transfer.targets.push(TransferTarget::new(
            BodyInput {
                body: part.body,
                producer: part.feature,
            },
            path(i + 1),
            face.intent.id,
            TransferTargetParameters {
                diameter: [Some(length(5.)), None, Some(length(6.5))][i].clone(),
                clearance: length(if i == 1 { 0.2 } else { 0. }),
                depth: if i == 0 {
                    TransferDepth::Blind { depth: length(12.) }
                } else {
                    TransferDepth::Through
                },
                reverse: true,
            },
        ));
        transfer.references.push(face);
    }
    Ok(Fixture {
        session: s,
        transfer,
        members,
        outer,
        rail: rail.body,
    })
}
fn save(directory: &Path, name: &str, s: &Session, stage: Value) -> Result<Value, Error> {
    let n = native(s)?;
    if !n.snapshot().references.all_resolved() {
        return Err("Review fixture has unresolved references".into());
    }
    let leaves=n.snapshot().leaf_occurrences.values().map(|l|{let solid=n.leaf_solid(&l.path).ok_or("Missing exact leaf")?;Ok(json!({"path":l.path,"component":l.component,"producer":l.producer,"pose":l.pose,"volume_mm3":solid.volume_mm3()?,"bounds":{"min":l.bounds.min,"max":l.bounds.max}}))}).collect::<Result<Vec<_>,Error>>()?;
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    let opened = project_file::read(
        &mut Cursor::new(&bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if opened.project.document != *s.document() {
        return Err("Source changed through file container".into());
    }
    let restored = Compiler::default().compile(&opened.project.document, 0)?;
    if restored.assembly_evaluation().leaves != n.assembly_evaluation().leaves {
        return Err("Reopened physical leaf poses changed".into());
    }
    let mut output = tempfile::NamedTempFile::new_in(directory)?;
    output.write_all(&bytes)?;
    output.as_file().sync_all()?;
    output.persist_noclobber(directory.join(name))?;
    Ok(
        json!({"file":name,"source_hash":n.snapshot().stamp.source_hash,"stage":stage,"transfer_counts":n.snapshot().transfer_counts,"leaves":leaves}),
    )
}
fn parameter(s: &mut Session, name: &str, value: &str) -> Result<(), Error> {
    let mut entity = s
        .document()
        .entities()
        .find(|e| e.name == name)
        .ok_or("Missing example parameter")?
        .clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        return Err("Example parameter has wrong type".into());
    };
    *expression = value.into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: format!("Edit {name}"),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(())
}
fn generate(directory: &Path) -> Result<(), Error> {
    std::fs::create_dir(directory)?;
    let mut f = fixture()?;
    let mut projects = vec![];
    projects.push(save(
        directory,
        "01-rail-receivers-input.multicad",
        &f.session,
        json!({"transfer":f.transfer,"members":f.members,"outer":f.outer,"rail":f.rail}),
    )?);
    let p = prepare_transfer(&f.session, f.session.revision(), actor(), &f.transfer)?;
    commit(&mut f.session, p)?;
    check_volumes(&f, 2.)?;
    projects.push(save(directory,"02-transferred.multicad",&f.session,json!({"hole_pitch_mm":32,"hole_count":2,"receiver_local_x_mm":[30,62],"receiver_local_y_mm":10}))?);
    parameter(&mut f.session, "hole_pitch", "40")?;
    check_volumes(&f, 2.)?;
    projects.push(save(
        directory,
        "03-pitch-edited.multicad",
        &f.session,
        json!({"hole_pitch_mm":40,"hole_count":2,"receiver_local_x_mm":[30,70]}),
    )?);
    parameter(&mut f.session, "hole_count", "3")?;
    check_volumes(&f, 3.)?;
    projects.push(save(
        directory,
        "04-pattern-edited.multicad",
        &f.session,
        json!({"hole_pitch_mm":40,"hole_count":3,"receiver_local_x_mm":[30,70,110]}),
    )?);
    parameter(&mut f.session, "rail_offset", "15")?;
    check_volumes(&f, 3.)?;
    projects.push(save(
        directory,
        "05-placement-edited.multicad",
        &f.session,
        json!({"rail_offset_mm":15,"receiver_local_x_mm":[35,75,115]}),
    )?);
    let holes_document = f.session.document().clone();
    let mut cutout = TransferRequest::existing(f.session.document(), f.transfer.group)?;
    let TransferSelection::FaceHoles { face } = cutout.source.selection else {
        return Err("Expected pattern face source".into());
    };
    cutout.source.selection = TransferSelection::FaceBoundary { face };
    for target in &mut cutout.targets {
        target.parameters.diameter = None;
        target.parameters.clearance = length(0.);
    }
    let p = prepare_transfer_edit(&f.session, f.session.revision(), actor(), &cutout)?;
    commit(&mut f.session, p)?;
    projects.push(save(directory,"06-boundary-cutout.multicad",&f.session,json!({"mode":"FaceBoundary","policy":"Only the explicit captured face outer wire is transferred; its inner holes are excluded."}))?);
    f.session = Session::new(holes_document)?;
    let mut explicit = TransferRequest::existing(f.session.document(), f.transfer.group)?;
    let n = native(&f.session)?;
    let selected = rim(&n, f.rail, 100.)?;
    let capture = selected.intent.id;
    explicit.source.selection = TransferSelection::Circles {
        references: vec![capture],
    };
    explicit.references.retain(|r| r.source != f.rail);
    explicit.references.push(selected);
    let p = prepare_transfer_edit(&f.session, f.session.revision(), actor(), &explicit)?;
    commit(&mut f.session, p)?;
    projects.push(save(
        directory,
        "07-explicit-third-hole.multicad",
        &f.session,
        json!({"capture":capture,"source_local_x_mm":100,"receiver_local_x_mm":115}),
    )?);
    parameter(&mut f.session, "hole_count", "1")?;
    projects.push(save_broken(
        directory,
        "08-missing-explicit-hole.multicad",
        &f.session,
        capture,
    )?);
    let source = Compiler::default().compile_sources(
        f.session.document(),
        f.session.revision(),
        &[f.rail],
    )?;
    let mut replacement = rim(&source, f.rail, 20.)?;
    replacement.intent.id = capture;
    let mut repair = TransferRequest::existing(f.session.document(), f.transfer.group)?;
    let slot = repair
        .references
        .iter_mut()
        .find(|r| r.intent.id == capture)
        .ok_or("Missing repair slot")?;
    *slot = replacement;
    let p = prepare_transfer_edit(&f.session, f.session.revision(), actor(), &repair)?;
    commit(&mut f.session, p)?;
    projects.push(save(directory,"09-repaired-explicit-hole.multicad",&f.session,json!({"capture":capture,"source_local_x_mm":20,"receiver_local_x_mm":35,"repair":"Explicit source-only capture replacement; same canonical capture identity."}))?);
    let mut output = tempfile::NamedTempFile::new_in(directory)?;
    output.write_all(&serde_json::to_vec_pretty(&json!({"projects":projects,"limits":"Parallel analytic source projection; shared Part definition edits affect all copies. These files demonstrate native behavior, not manufacturing approval."}))?)?;
    output.persist_noclobber(directory.join("manifest.json"))?;
    Ok(())
}
fn rim(model: &Model, part: EntityId, x: f64) -> Result<AlignmentInput, Error> {
    let producer = *model
        .snapshot()
        .body_sources
        .get(&part)
        .ok_or("Missing hardware head")?;
    let snapshot = &model
        .snapshot()
        .shapes
        .get(&producer)
        .ok_or("Missing hardware topology")?
        .topology;
    let e=snapshot.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::CircularArc{frame,radius_mm,sweep_rad,..})if (frame.origin_mm[0]-x).abs()<1e-7 && (frame.origin_mm[1]-10.).abs()<1e-7 && (frame.origin_mm[2]-4.).abs()<1e-7 && (radius_mm-3.).abs()<1e-7 && (sweep_rad.abs()-std::f64::consts::TAU).abs()<1e-9)).ok_or("Missing explicit top hole rim")?;
    Ok(AlignmentInput {
        source: part,
        intent: GeometryIntent::capture(snapshot, e.key)?,
    })
}
fn save_broken(
    directory: &Path,
    name: &str,
    s: &Session,
    capture: EntityId,
) -> Result<Value, Error> {
    let error = native(s)
        .err()
        .ok_or("Removed explicit hole unexpectedly compiled without repair")?;
    let message = error.to_string();
    if !message.contains("repair") && !message.contains("Missing") {
        return Err(format!("Unexpected missing-hole failure: {message}").into());
    }
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    let opened = project_file::read(
        &mut Cursor::new(&bytes),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if opened.project.document != *s.document() {
        return Err("Missing reference state changed through save/reopen".into());
    }
    if Compiler::default()
        .compile(&opened.project.document, 0)
        .is_ok()
    {
        return Err("Missing reference silently repaired through save/reopen".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(directory.join(name))?;
    Ok(
        json!({"file":name,"capture":capture,"expected_native_failure":message,"repair":"Open the saved receiving transfer and explicitly reselect a current source hole."}),
    )
}
fn check_volumes(f: &Fixture, count: f64) -> Result<(), Error> {
    let model = native(&f.session)?;
    for (id, expected) in std::iter::once((
        f.rail,
        130. * 20. * 4. - count * std::f64::consts::PI * 9. * 4.,
    ))
    .chain(f.transfer.targets.iter().enumerate().map(|(i, target)| {
        let (height, radius, depth) = [(18., 2.5, 12.), (8., 3.2, 8.), (2., 3.25, 2.)][i];
        (
            target.input.body,
            200. * 60. * height - count * std::f64::consts::PI * radius * radius * depth,
        )
    })) {
        let actual = model
            .solid(id)
            .ok_or("Missing exact review source")?
            .volume_mm3()?;
        if (actual - expected).abs() > 1e-4 {
            return Err(
                format!("Exact volume {actual} differs from expected {expected} for {id}").into(),
            );
        }
    }
    Ok(())
}
fn main() -> Result<(), Error> {
    let directory = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .ok_or("Pass a new output directory")?;
    generate(&directory)
}
