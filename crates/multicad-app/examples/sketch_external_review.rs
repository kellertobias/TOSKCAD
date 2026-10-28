//! Emits genuine production-compiled reference fixtures for native CAD-63 review.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Axis, ConstraintKind, CurveKind, source::Source},
    document::{Document, EntityData, EntityId, Mutation},
    model::{self, ExtrudeRequest, PrimitiveParameters, PrimitiveRequest},
    project_file::{self, ProjectFile},
    references::{AnalyticGeometry, GeometryIntent, TopologySnapshot},
    sketch::Datum,
    sketch_external::{Behavior, CaptureInput, CaptureRequest, Method, OutputIds},
};
use multicad_model::Compiler;
use std::{collections::BTreeMap, io::Write, path::Path};
type Error = Box<dyn std::error::Error>;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn parameters(width: f64) -> PrimitiveParameters {
    PrimitiveParameters::Box {
        size: [model::length(width), model::length(20.), model::length(10.)],
        frame: Default::default(),
    }
}
fn sketch(session: &mut Session, name: &str, z: f64) -> Result<EntityId, Error> {
    let (id, mut edits) = Source::create(Datum::Xy, name)?;
    for edit in &mut edits {
        if let Mutation::Insert(entity) = edit
            && let EntityData::Plane { origin_mm, .. } = &mut entity.data
        {
            origin_mm[2] = z;
        }
    }
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: name.into(),
        edits,
    })?;
    Ok(id)
}
fn source(session: &Session, id: EntityId) -> Result<Source, Error> {
    let EntityData::Sketch { definition, .. } =
        &session.document().get(id).ok_or("Missing sketch")?.data
    else {
        return Err("Not a sketch".into());
    };
    Ok(Source::from_definition(definition)?)
}
fn topologies(session: &Session) -> Result<BTreeMap<EntityId, TopologySnapshot>, Error> {
    let model = Compiler::default().compile(session.document(), session.revision())?;
    Ok(model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, shape)| (*id, shape.topology.clone()))
        .collect())
}
fn capture(
    session: &mut Session,
    sketch: EntityId,
    producer: EntityId,
    method: Method,
) -> Result<CaptureRequest, Error> {
    let maps = topologies(session)?;
    let snapshot = &maps[&producer];
    let key = snapshot
        .elements
        .iter()
        .find(|element| match (&element.geometry, method) {
            (Some(AnalyticGeometry::Line { a_mm: a, b_mm: b }), Method::Coplanar) => {
                a[1].abs() < 1e-7
                    && b[1].abs() < 1e-7
                    && (a[2] - 10.).abs() < 1e-7
                    && (b[2] - 10.).abs() < 1e-7
                    && (a[0] - b[0]).abs() > 29.
            }
            (Some(AnalyticGeometry::Line { a_mm: a, b_mm: b }), Method::Intersect) => {
                (a[0] - 30.).abs() < 1e-7
                    && (b[0] - 30.).abs() < 1e-7
                    && a[1].abs() < 1e-7
                    && b[1].abs() < 1e-7
                    && (a[2] - b[2]).abs() > 9.
            }
            _ => false,
        })
        .ok_or("Expected exact native box edge missing")?
        .key;
    let request = CaptureRequest::new(
        session.document(),
        session.revision(),
        CaptureInput {
            sketch,
            source: producer,
            intent: GeometryIntent::capture(snapshot, key)?,
            method,
            behavior: if method == Method::Coplanar {
                Behavior::Derived
            } else {
                Behavior::ReferenceOnly
            },
        },
        &maps,
    )?;
    let edit = multicad_solver::prepare_external_reference(
        session,
        session.revision(),
        actor(),
        &request,
        &maps,
    )?;
    session.commit(
        edit.edit
            .preview
            .ok_or("Reference has no checked preview")?,
    )?;
    Ok(request)
}
fn finish_profile(
    session: &mut Session,
    sketch: EntityId,
    own: Source,
    depth: f64,
) -> Result<EntityId, Error> {
    let map = topologies(session)?;
    let report = multicad_core::references::inspect(session.document(), session.revision(), &map);
    let edit = multicad_solver::prepare_canonical_edit(
        session,
        session.revision(),
        actor(),
        sketch,
        own,
        Some(&report),
    )?;
    let intent = edit
        .semantic
        .as_ref()
        .ok_or("No semantic sketch")?
        .regions()?
        .regions
        .first()
        .ok_or("No closed exact profile")?
        .intent();
    session.commit(edit.preview.ok_or("Profile has no checked preview")?)?;
    let request = ExtrudeRequest::new(
        sketch,
        vec![intent],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: model::length(depth),
                },
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_extrude(session, session.revision(), actor(), &request)?;
    session.commit(prepared.prepared)?;
    Ok(request.outputs[0].feature)
}
fn projected(
    session: &mut Session,
    sketch: EntityId,
    producer: EntityId,
) -> Result<EntityId, Error> {
    let request = capture(session, sketch, producer, Method::Coplanar)?;
    let OutputIds::Line { start, end, .. } = request.output else {
        return Err("Expected line".into());
    };
    let mut own = source(session, sketch)?;
    let a = own
        .sketch
        .points
        .iter()
        .find(|p| p.id == start)
        .ok_or("Missing start")?
        .position_mm;
    let b = own
        .sketch
        .points
        .iter()
        .find(|p| p.id == end)
        .ok_or("Missing end")?
        .position_mm;
    let ta = own.add_point([a[0], a[1] + 10.]);
    let tb = own.add_point([b[0], b[1] + 10.]);
    let right = own.add_line(end, tb);
    let top = own.add_line(tb, ta);
    let left = own.add_line(ta, start);
    for kind in [
        ConstraintKind::Vertical { line: right },
        ConstraintKind::Horizontal { line: top },
        ConstraintKind::Vertical { line: left },
        ConstraintKind::Distance {
            a: start,
            b: ta,
            axis: Axis::Y,
            dimension: model::length(10.),
        },
    ] {
        own.add_constraint(kind);
    }
    finish_profile(session, sketch, own, 5.)
}
fn intersected(
    session: &mut Session,
    sketch: EntityId,
    producer: EntityId,
) -> Result<EntityId, Error> {
    let request = capture(session, sketch, producer, Method::Intersect)?;
    let OutputIds::Point { point, .. } = request.output else {
        return Err("Expected point".into());
    };
    let mut own = source(session, sketch)?;
    let bottom = own.add_rectangle([30., 0.], [35., 7.])?;
    let CurveKind::Line { start: corner, .. } = own
        .sketch
        .curves
        .iter()
        .find(|c| c.id == bottom)
        .ok_or("No rectangle")?
        .kind
    else {
        return Err("Expected line".into());
    };
    let side = own.sketch.curves[1].id;
    for kind in [
        ConstraintKind::Coincident {
            a: point,
            b: corner,
        },
        ConstraintKind::Length {
            line: bottom,
            dimension: model::length(5.),
        },
        ConstraintKind::Length {
            line: side,
            dimension: model::length(7.),
        },
    ] {
        own.add_constraint(kind);
    }
    finish_profile(session, sketch, own, 2.)
}
fn save(session: &Session, path: &Path) -> Result<serde_json::Value, Error> {
    let native = Compiler::default().compile(session.document(), session.revision())?;
    let project = ProjectFile::new(session.document().clone())?;
    let mut bytes = vec![];
    project_file::write(&project, &mut bytes, Default::default())?;
    let loaded = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if loaded.project.document != *session.document() {
        return Err("Native project roundtrip changed canonical source".into());
    }
    let reopened = Compiler::default().compile(&loaded.project.document, 0)?;
    if reopened.snapshot().bodies != native.snapshot().bodies {
        return Err("Reopen changed body producers".into());
    }
    let parent = path.parent().ok_or("No parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    let bodies=native.snapshot().bodies.iter().map(|(body,producer)| {
        let shape=&native.snapshot().shapes[producer];
        serde_json::json!({"body":body,"producer":producer,"name":session.document().get(*body).map(|e|&e.name),"volume_mm3":shape.volume_mm3,"bounds_min":shape.bounds.min,"bounds_max":shape.bounds.max})
    }).collect::<Vec<_>>();
    Ok(
        serde_json::json!({"file":path.file_name().unwrap().to_string_lossy(),"source_revision":session.revision(),"bodies":bodies,"source_hash":native.snapshot().stamp.source_hash}),
    )
}
fn generate(directory: &Path) -> Result<Vec<serde_json::Value>, Error> {
    std::fs::create_dir_all(directory)?;
    let mut session = Session::new(Document::new("Exact linked reference review")?)?;
    let base = PrimitiveRequest::new("Adjacent cabinet source", parameters(30.));
    let prepared = model::prepare_primitive(&session, 0, actor(), &base)?;
    session.commit(prepared.prepared)?;
    let top = sketch(&mut session, "Top projection at Z10", 10.)?;
    let cross = sketch(&mut session, "Wall intersection at Z5", 5.)?;
    let mut reports = vec![save(&session, &directory.join("reference-start.multicad"))?];
    let original = session.document().clone();
    let projected_feature = projected(&mut session, top, base.feature)?;
    reports.push(save(
        &session,
        &directory.join("projection-linked.multicad"),
    )?);
    let canonical = source(&session, top)?;
    let prepared = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &parameters(40.),
    )?;
    session.commit(prepared.prepared)?;
    let native = Compiler::default().compile(session.document(), session.revision())?;
    if (native.snapshot().shapes[&projected_feature].volume_mm3 - 2000.).abs() > 1e-6
        || source(&session, top)? != canonical
    {
        return Err("Projection did not regenerate without baking authored source".into());
    }
    reports.push(save(
        &session,
        &directory.join("projection-width40.multicad"),
    )?);
    let mut session = Session::new(original)?;
    let intersection_feature = intersected(&mut session, cross, base.feature)?;
    reports.push(save(
        &session,
        &directory.join("intersection-linked.multicad"),
    )?);
    let canonical = source(&session, cross)?;
    let prepared = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &parameters(40.),
    )?;
    session.commit(prepared.prepared)?;
    let native = Compiler::default().compile(session.document(), session.revision())?;
    let shape = &native.snapshot().shapes[&intersection_feature];
    if (shape.volume_mm3 - 70.).abs() > 1e-6
        || (shape.bounds.min[0] - 40.).abs() > 1e-6
        || source(&session, cross)? != canonical
    {
        return Err("Intersection did not regenerate without baking authored source".into());
    }
    reports.push(save(
        &session,
        &directory.join("intersection-width40.multicad"),
    )?);
    let replacement = PrimitiveRequest::new("Repair replacement wall", parameters(50.));
    let edit = model::prepare_primitive(&session, session.revision(), actor(), &replacement)?;
    session.commit(edit.prepared)?;
    let edit = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        base.feature,
        &PrimitiveParameters::Box {
            size: [model::length(40.), model::length(20.), model::length(2.)],
            frame: Default::default(),
        },
    )?;
    session.commit(edit.prepared)?;
    let failure = Compiler::default()
        .compile(session.document(), session.revision())
        .err()
        .ok_or("Broken finite intersection unexpectedly rebuilt")?
        .to_string();
    let EntityData::Sketch { plane, .. } =
        session.document().get(cross).ok_or("Sketch missing")?.data
    else {
        return Err("Not a sketch".into());
    };
    let partial = Compiler::default().compile_sources(
        session.document(),
        session.revision(),
        &[replacement.feature, plane],
    )?;
    let path = directory.join("repair-broken.multicad");
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone())?,
        &mut bytes,
        Default::default(),
    )?;
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )?;
    if reopened.project.document != *session.document() {
        return Err("Broken source container changed on reopen".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(&path)?;
    reports.push(serde_json::json!({"file":"repair-broken.multicad","expected_full_native_error":failure,"source_revision":session.revision(),"source_hash":partial.snapshot().stamp.source_hash,"replacement":replacement.feature,"consumer":cross,"scope":"Structurally valid canonical project with explicitly broken consumed reference; only fresh source-only inspection succeeds"}));
    let topo = &partial.snapshot().shapes[&replacement.feature].topology;
    let selected=topo.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm:a,b_mm:b})if(a[0]-50.).abs()<1e-7&&(b[0]-50.).abs()<1e-7&&a[1].abs()<1e-7&&b[1].abs()<1e-7&&(a[2]-b[2]).abs()>9.)).ok_or("Replacement native edge missing")?.key;
    let maps = partial
        .snapshot()
        .shapes
        .iter()
        .map(|(id, s)| (*id, s.topology.clone()))
        .collect();
    let binding = multicad_core::sketch_external::bindings(session.document(), cross)?
        .into_iter()
        .next()
        .ok_or("No external reference")?;
    let edit = multicad_solver::prepare_external_repair(
        &session,
        session.revision(),
        actor(),
        &multicad_solver::ExternalRepairRequest {
            sketch: cross,
            reference: binding.reference,
            source: replacement.feature,
            selected,
        },
        &maps,
    )?;
    let prepared = edit.preview.ok_or("No checked repair preview")?;
    let full = Compiler::default().compile(prepared.document(), session.revision())?;
    if (full.snapshot().shapes[&intersection_feature].bounds.min[0] - 50.).abs() > 1e-6 {
        return Err("Repair did not regenerate downstream solid at X50".into());
    }
    session.commit(prepared)?;
    reports.push(save(&session, &directory.join("repair-repaired.multicad"))?);
    let report = serde_json::json!({"scope":"Genuine production OCCT compiler; exact checked source references; byte-roundtripped native containers; explicit broken recovery fixture; no native GUI acceptance implied","source":base,"projection_sketch":top,"intersection_sketch":cross,"fixtures":reports});
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join("fixture-report.json"))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    Ok(reports)
}
fn main() -> Result<(), Error> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [path] = args.as_slice() else {
        return Err("Usage: sketch_external_review NEW_OUTPUT_DIRECTORY".into());
    };
    let reports = generate(Path::new(path))?;
    println!(
        "{} production-compiled native projects written to {}",
        reports.len(),
        Path::new(path).display()
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn generated_projects_compile_and_reopen_with_exact_live_dependencies() {
        let directory = tempfile::tempdir().unwrap();
        let reports = super::generate(directory.path()).unwrap();
        assert_eq!(reports.len(), 7);
        assert!(
            super::generate(directory.path()).is_err(),
            "Never overwrite acceptance assets"
        );
    }
}
