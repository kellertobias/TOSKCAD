//! Reproducible native placement projects for manual application review.
//! Source and exact solids use production APIs; generated files are local only.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, AlignmentAnchor, AlignmentInput, AlignmentParameters, BodyInput,
        InstanceAlignmentRequest, InstanceCopyRequest, InstanceCopySource, NumericTransform,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
    },
    parameters::BoundExpression,
    project_file::{self, ProjectFile},
    references::{Attachment, GeometryIntent, Origin, TopologyKind},
};
use multicad_model::{Compiler, Model};
use std::{
    io::{Cursor, Write},
    path::{Path, PathBuf},
};
type Error = Box<dyn std::error::Error>;
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
struct Fixture {
    session: Session,
    source: PrimitiveRequest,
    target: PrimitiveRequest,
    free: InstanceCopyRequest,
    distance: EntityId,
    target_x: EntityId,
    gap: EntityId,
}
fn parameter(name: &str, value: f64) -> Entity {
    Entity::new(
        name,
        EntityData::Parameter {
            expression: value.to_string(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    )
}
fn expression(session: &Session, text: &str) -> Result<Dimension, Error> {
    Ok(Dimension::Expression {
        expression: BoundExpression::bind(session.document(), text, "mm", None)?,
    })
}
fn build() -> Result<Fixture, Error> {
    let mut session = Session::new(Document::new(
        "Placement review: source, free copy and target",
    )?)?;
    let distance = parameter("distance", 30.);
    let target_x = parameter("target_x", 160.);
    let gap = parameter("gap", 5.);
    let datum = Entity::new(
        "Fixture datum Z 60 mm",
        EntityData::Plane {
            origin_mm: [0., 0., 60.],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: "Define placement parameters and datum".into(),
        edits: vec![
            Mutation::Insert(distance.clone()),
            Mutation::Insert(target_x.clone()),
            Mutation::Insert(gap.clone()),
            Mutation::Insert(datum),
        ],
    })?;
    let source = PrimitiveRequest::new(
        "Source block · 60 x 40 x 12",
        PrimitiveParameters::Box {
            size: [length(60.), length(40.), length(12.)],
            frame: Default::default(),
        },
    );
    let target = PrimitiveRequest::new(
        "Target pedestal · 80 x 60 x 20",
        PrimitiveParameters::Box {
            size: [length(80.), length(60.), length(20.)],
            frame: PrimitiveFrame {
                origin: [expression(&session, "=$target_x")?, length(0.), length(0.)],
                ..Default::default()
            },
        },
    );
    for request in [&source, &target] {
        let prepared = model::prepare_primitive(&session, session.revision(), actor(), request)?;
        session.commit(prepared.prepared)?;
    }
    let mut free = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: source.body,
                producer: source.feature,
            },
        },
        NumericTransform {
            translation: [expression(&session, "=$distance")?, length(80.), length(0.)],
            ..Default::default()
        },
    );
    free.name = "Free source copy · X = $distance".into();
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&free),
    )?;
    session.commit(prepared.prepared)?;
    Ok(Fixture {
        session,
        source,
        target,
        free,
        distance: distance.id,
        target_x: target_x.id,
        gap: gap.id,
    })
}
fn cap(
    model: &Model,
    producer: EntityId,
    owner: EntityId,
    role: &str,
) -> Result<AlignmentInput, Error> {
    let snapshot = &model
        .snapshot()
        .shapes
        .get(&producer)
        .ok_or("Native producer is missing")?
        .topology;
    let anchor = Origin {
        source: producer,
        role: role.into(),
    };
    let matches = snapshot
        .elements
        .iter()
        .filter(|element| {
            element.key.kind == TopologyKind::Face && element.origins.contains(&anchor)
        })
        .collect::<Vec<_>>();
    let [face] = matches.as_slice() else {
        return Err("Review cap needs one uniquely named native face".into());
    };
    if face
        .frames
        .iter()
        .filter(|frame| frame.origin == anchor)
        .count()
        != 1
    {
        return Err("Review cap needs one explicit semantic frame".into());
    }
    let mut intent = GeometryIntent::capture(snapshot, face.key)?;
    intent.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    Ok(AlignmentInput {
        source: owner,
        intent,
    })
}
fn add_aligned(fixture: &mut Fixture) -> Result<InstanceAlignmentRequest, Error> {
    let mut copy = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: fixture.source.body,
                producer: fixture.source.feature,
            },
        },
        NumericTransform {
            translation: [length(0.), length(-80.), length(0.)],
            ..Default::default()
        },
    );
    copy.name = "Aligned source copy · bottom to target top".into();
    let session = &mut fixture.session;
    let prepared = model::prepare_instance_copies(
        session,
        session.revision(),
        actor(),
        std::slice::from_ref(&copy),
    )?;
    session.commit(prepared.prepared)?;
    let native = Compiler::default().compile(session.document(), session.revision())?;
    let source = cap(
        &native,
        fixture.source.feature,
        fixture.source.body,
        "cap.start",
    )?;
    let target = cap(
        &native,
        fixture.target.feature,
        fixture.target.body,
        "cap.end",
    )?;
    let request = InstanceAlignmentRequest::new(
        copy.instance,
        AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: source.intent.id,
            },
            target: AlignmentAnchor::Geometry {
                reference: target.intent.id,
            },
            offset: [length(0.), length(0.), expression(session, "=$gap")?],
            opposed: true,
            ..Default::default()
        },
        vec![source, target],
    );
    let prepared = model::prepare_instance_alignments(
        session,
        session.revision(),
        actor(),
        std::slice::from_ref(&request),
    )?;
    session.commit(prepared.prepared)?;
    Ok(request)
}
fn set_parameter(session: &mut Session, id: EntityId, value: f64) -> Result<(), Error> {
    let mut entity = session
        .document()
        .get(id)
        .ok_or("Review parameter is missing")?
        .clone();
    let EntityData::Parameter { expression, .. } = &mut entity.data else {
        return Err("Review parameter has the wrong kind".into());
    };
    *expression = value.to_string();
    session.apply(Request {
        expected_revision: session.revision(),
        actor: actor(),
        label: format!("Set {} to {value} mm", entity.name),
        edits: vec![Mutation::Replace(entity)],
    })?;
    Ok(())
}
fn bounds(model: &Model, id: EntityId) -> Result<serde_json::Value, Error> {
    let solid = model
        .solid(id)
        .ok_or("Native review solid is unavailable")?;
    let bounds = solid.bounds()?;
    Ok(serde_json::json!({"min":bounds.min,"max":bounds.max,"volume_mm3":solid.volume_mm3()?}))
}
fn report(
    fixture: &Fixture,
    aligned: Option<&InstanceAlignmentRequest>,
) -> Result<serde_json::Value, Error> {
    let session = &fixture.session;
    let model = Compiler::default().compile(session.document(), session.revision())?;
    if model.snapshot().bodies.len() != 2
        || model.snapshot().occurrences.len() != if aligned.is_some() { 2 } else { 1 }
        || !model.snapshot().references.all_resolved()
    {
        return Err("Native review has unexpected outputs or unresolved references".into());
    }
    let source = bounds(&model, fixture.source.body)?;
    let target = bounds(&model, fixture.target.body)?;
    let free = bounds(&model, fixture.free.instance)?;
    if (model
        .solid(fixture.source.body)
        .ok_or("Missing source")?
        .volume_mm3()?
        - 28800.)
        .abs()
        > 1e-6
        || (model
            .solid(fixture.target.body)
            .ok_or("Missing target")?
            .volume_mm3()?
            - 96000.)
            .abs()
            > 1e-6
    {
        return Err("Review primitive volume check failed".into());
    }
    let aligned_bounds = aligned
        .map(|request| bounds(&model, request.instance))
        .transpose()?;
    Ok(
        serde_json::json!({"document":session.document().id(),"source_hash":model.snapshot().stamp.source_hash,"parameters":{"distance":fixture.distance,"target_x":fixture.target_x,"gap":fixture.gap},"source":{"part":fixture.source.body,"producer":fixture.source.feature,"geometry":source},"target":{"part":fixture.target.body,"producer":fixture.target.feature,"geometry":target},"free_copy":{"instance":fixture.free.instance,"driver":fixture.free.driver,"geometry":free},"aligned_copy":aligned.map(|request|serde_json::json!({"instance":request.instance,"driver":request.driver,"references":request.references,"geometry":aligned_bounds}))}),
    )
}
fn save_new(path: &Path, session: &Session) -> Result<(), Error> {
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
        return Err("Review project roundtrip changed source".into());
    }
    Compiler::default().compile(&reopened.project.document, 0)?;
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Review path needs a parent")?)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    Ok(())
}
fn generate(directory: &Path) -> Result<(), Error> {
    // Refuse even an existing empty directory, so a retry cannot overwrite or
    // mix review files with a previous run or another application save.
    std::fs::create_dir(directory)?;
    let mut fixture = build()?;
    let ready = report(&fixture, None)?;
    save_new(&directory.join("01-ready.multicad"), &fixture.session)?;
    let alignment = add_aligned(&mut fixture)?;
    let aligned = report(&fixture, Some(&alignment))?;
    save_new(&directory.join("02-aligned.multicad"), &fixture.session)?;
    set_parameter(&mut fixture.session, fixture.target_x, 220.)?;
    let followed = report(&fixture, Some(&alignment))?;
    save_new(
        &directory.join("03-target-moved.multicad"),
        &fixture.session,
    )?;
    let model =
        Compiler::default().compile(fixture.session.document(), fixture.session.revision())?;
    if model.snapshot().occurrences[&alignment.instance].bounds.min != [220., 0., 25.]
        || model.snapshot().occurrences[&fixture.free.instance]
            .bounds
            .min
            != [30., 80., 0.]
    {
        return Err("Current target must move only its aligned occurrence".into());
    }
    let instructions = serde_json::json!({"scope":"Local review fixtures generated with production canonical transactions, native B-reps and native project container APIs. This is not GUI acceptance.","files":{"01-ready.multicad":ready,"02-aligned.multicad":aligned,"03-target-moved.multicad":followed},"checks":["01-ready: select Source block or Free source copy, then Model/Common Move or Placed copy. Preview and Apply; Undo restores source.","Parameters: distance starts at 30 mm and drives only the free copy X. Change to 45 mm to move that occurrence; Source block geometry stays at the origin.","02-aligned: select Aligned source copy and edit its applied placement. Source is Source block cap.start, target is Target pedestal cap.end, opposed normals, normal gap = $gap.","02-aligned: change target_x from 160 to 220 mm. The pedestal and aligned copy follow; the source Part and free copy stay unchanged. Compare 03-target-moved.","02-aligned: change gap from 5 to 8 mm. Only the aligned copy rises by 3 mm. Source Part and target geometry remain unchanged.","Fixture datum Z 60 mm is an explicit construction plane available in alignment datum choices."]});
    let mut bytes = serde_json::to_vec_pretty(&instructions)?;
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("review.json"))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    println!(
        "{}: 01-ready, 02-aligned, 03-target-moved; two exact Parts, an independent free copy, and a persistent face-aligned copy. Review IDs, measured geometry and steps: review.json",
        directory.display()
    );
    Ok(())
}
fn main() -> Result<(), Error> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let directory = match args.as_slice() {
        [] => PathBuf::from("artifacts/placement-review"),
        [path] => PathBuf::from(path),
        _ => return Err("Usage: placement_review [NEW_DIRECTORY]".into()),
    };
    generate(&directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_projects_reopen_with_shared_geometry_independent_poses_and_current_target_following()
    -> Result<(), Error> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("review");
        generate(&path)?;
        let open = |name: &str| -> Result<Document, Error> {
            Ok(project_file::read(
                &mut std::fs::File::open(path.join(name))?,
                Default::default(),
                &project_file::supported_capabilities(),
            )?
            .project
            .document)
        };
        let ready = open("01-ready.multicad")?;
        let aligned = open("02-aligned.multicad")?;
        let moved = open("03-target-moved.multicad")?;
        assert_eq!(ready.id(), aligned.id());
        assert_eq!(aligned.id(), moved.id());
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("review.json"))?)?;
        let entry = &manifest["files"]["02-aligned.multicad"];
        let id = |value: &serde_json::Value| EntityId::parse(value.as_str().unwrap()).unwrap();
        let source = id(&entry["source"]["part"]);
        let source_feature = id(&entry["source"]["producer"]);
        let target = id(&entry["target"]["part"]);
        let free = id(&entry["free_copy"]["instance"]);
        let aligned_copy = id(&entry["aligned_copy"]["instance"]);
        let driver = id(&entry["aligned_copy"]["driver"]);
        assert_ne!(free, aligned_copy);
        assert_eq!(aligned.get(source), ready.get(source));
        assert_eq!(aligned.get(source_feature), ready.get(source_feature));
        let (definition, refs) = model::read_instance_alignment(&aligned, driver)?;
        assert_eq!(definition.owner, aligned_copy);
        assert_eq!(refs[0].source, source);
        assert_eq!(refs[1].source, target);
        assert_eq!(
            refs[0].intent.attachment.as_ref().unwrap().anchor.role,
            "cap.start"
        );
        assert_eq!(
            refs[1].intent.attachment.as_ref().unwrap().anchor.role,
            "cap.end"
        );
        assert_eq!(
            model::read_instance_alignment(&moved, driver)?,
            (definition, refs)
        );
        let native = Compiler::default().compile(&aligned, 0)?;
        assert_eq!(
            native.snapshot().occurrences[&aligned_copy].bounds.min,
            [160., 0., 25.]
        );
        assert_eq!(
            native.snapshot().occurrences[&free].bounds.min,
            [30., 80., 0.]
        );
        assert_eq!(
            native.snapshot().shapes[&source_feature].bounds.min,
            [0.; 3]
        );
        let mut session = Session::new(aligned)?;
        let distance = id(&entry["parameters"]["distance"]);
        let gap = id(&entry["parameters"]["gap"]);
        set_parameter(&mut session, distance, 45.)?;
        set_parameter(&mut session, gap, 8.)?;
        let native = Compiler::default().compile(session.document(), session.revision())?;
        assert_eq!(
            native.snapshot().occurrences[&free].bounds.min,
            [45., 80., 0.]
        );
        assert_eq!(
            native.snapshot().occurrences[&aligned_copy].bounds.min,
            [160., 0., 28.]
        );
        assert_eq!(
            session.document().get(source_feature),
            ready.get(source_feature)
        );
        Ok(())
    }
    #[test]
    fn existing_review_directory_and_saved_file_are_never_overwritten() -> Result<(), Error> {
        let root = tempfile::tempdir()?;
        let path = root.path().join("review");
        generate(&path)?;
        let file = path.join("01-ready.multicad");
        let original = std::fs::read(&file)?;
        assert!(generate(&path).is_err());
        let fixture = build()?;
        assert!(save_new(&file, &fixture.session).is_err());
        assert_eq!(std::fs::read(&file)?, original);
        Ok(())
    }
}
