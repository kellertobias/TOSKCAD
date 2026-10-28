use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Link, Mutation},
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, NumericTransform,
        PrimitiveParameters, PrimitiveRequest, angle, length,
    },
    parameters::BoundExpression,
    project_file::{self, FileLimits, ProjectFile},
    references::{AnalyticGeometry, Attachment, GeometryIntent, Origin, TopologyKind},
    sketch::Datum,
    sketch_regions,
};
use multicad_model::Compiler;
use std::io::Cursor;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn apply(session: &mut Session, edits: Vec<Mutation>) -> multicad_core::commands::Change {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Occurrence fixture edit".into(),
            edits,
        })
        .unwrap()
}
fn fixture() -> (Session, PrimitiveRequest, EntityId, InstanceCopyRequest) {
    let offset = Entity::new(
        "offset",
        EntityData::Parameter {
            expression: "1".into(),
            unit: "cm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let doc = Document::new("Exact shared occurrences")
        .unwrap()
        .edited(&[Mutation::Insert(offset.clone())])
        .unwrap();
    let mut session = Session::new(doc).unwrap();
    let part = PrimitiveRequest::new(
        "Source box",
        PrimitiveParameters::Box {
            size: [length(2.), length(3.), length(4.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, actor(), &part).unwrap();
    session.commit(prepared.prepared).unwrap();
    let copied = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        },
        NumericTransform {
            translation: [
                Dimension::Expression {
                    expression: BoundExpression::bind(session.document(), "=$offset", "mm", None)
                        .unwrap(),
                },
                length(0.),
                length(0.),
            ],
            angle: angle(90.),
            ..Default::default()
        },
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&copied),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    (session, part, offset.id, copied)
}
#[test]
fn native_occurrences_share_component_meshes_and_use_current_world_brep_and_exact_analytic_geometry()
 {
    let (mut session, part, offset, first) = fixture();
    let original_part = session.document().get(part.body).unwrap().clone();
    let original_feature = session.document().get(part.feature).unwrap().clone();
    let second = InstanceCopyRequest::new(
        InstanceCopySource::Instance {
            instance: first.instance,
        },
        NumericTransform {
            translation: [length(0.), length(10.), length(0.)],
            ..Default::default()
        },
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&second),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let compiler = Compiler::default();
    let initial = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(initial.snapshot().shapes.len(), 1);
    assert_eq!(initial.snapshot().occurrences.len(), 2);
    let a = &initial.snapshot().occurrences[&first.instance];
    let b = &initial.snapshot().occurrences[&second.instance];
    assert_eq!(a.producer, part.feature);
    assert_eq!(b.producer, part.feature);
    assert_eq!(a.bounds.min, [7., 0., 0.]);
    assert_eq!(a.bounds.max, [10., 2., 4.]);
    assert_eq!(b.bounds.min, [7., 10., 0.]);
    for instance in [first.instance, second.instance] {
        let solid = initial.solid(instance).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        assert!((solid.volume_mm3().unwrap() - 24.).abs() < 1e-9);
        let snapshot = &initial.snapshot().occurrences[&instance];
        assert!(snapshot.topology.elements.iter().any(|element|matches!(element.geometry,Some(AnalyticGeometry::Point {position_mm}) if position_mm==snapshot.bounds.min)));
    }
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&opened.project.document, session.document());
    let mut parameter = session.document().get(offset).unwrap().clone();
    parameter.name = "renamed_offset".into();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "2".into();
    apply(&mut session, vec![Mutation::Replace(parameter)]);
    let changed = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        changed.snapshot().shapes[&part.feature].bounds.min,
        [0., 0., 0.]
    );
    assert_eq!(
        changed.snapshot().occurrences[&first.instance].bounds.min,
        [17., 0., 0.]
    );
    assert_eq!(
        changed.snapshot().occurrences[&second.instance].bounds.min,
        [17., 10., 0.]
    );
    assert_eq!(session.document().get(part.body).unwrap(), &original_part);
    assert_eq!(
        session.document().get(part.feature).unwrap(),
        &original_feature
    );
}
#[test]
fn placed_cap_reference_evaluates_before_downstream_sketch_and_follows_pose_parameters() {
    let (mut session, part, offset, placed) = fixture();
    let compiler = Compiler::default();
    let initial = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let snapshot = &initial.snapshot().occurrences[&placed.instance].topology;
    let cap = Origin {
        source: part.feature,
        role: "cap.end".into(),
    };
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&cap))
        .unwrap();
    let mut intent = GeometryIntent::capture(snapshot, element.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: cap,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let (sketch, edits) = Source::create(Datum::Xy, "Sketch on placed occurrence").unwrap();
    apply(&mut session, edits);
    let (mut source, _, _) = Source::read(session.document(), sketch).unwrap();
    source.add_rectangle([0.2, 0.2], [1.2, 1.2]).unwrap();
    let region = sketch_regions::extract(&source).regions[0].intent();
    let source_edits = source.replace(session.document(), sketch).unwrap();
    apply(&mut session, source_edits);
    let EntityData::Sketch { plane, .. } = session.document().get(sketch).unwrap().data else {
        panic!()
    };
    let mut entity = session.document().get(plane).unwrap().clone();
    let EntityData::Plane { anchor, .. } = &mut entity.data else {
        panic!()
    };
    *anchor = Some(Link {
        target: placed.instance,
        role: "face_attachment".into(),
        geometry: Some(intent),
    });
    apply(&mut session, vec![Mutation::Replace(entity)]);
    let child = model::ExtrudeRequest::new(
        sketch,
        vec![region],
        model::ExtrudeParameters {
            extent: model::Extent::OneSided {
                end: model::Termination::Distance {
                    distance: length(2.),
                },
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_extrude(&session, session.revision(), actor(), &child).unwrap();
    session.commit(prepared.prepared).unwrap();
    let child_feature = child.outputs[0].feature;
    let current = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert!(current.snapshot().references.all_resolved());
    let bounds = current.snapshot().shapes[&child_feature].bounds;
    assert!((bounds.min[0] - 8.8).abs() < 1e-9);
    assert!((bounds.max[0] - 9.8).abs() < 1e-9);
    assert_eq!(bounds.min[2], 4.);
    assert_eq!(bounds.max[2], 6.);
    let canonical_sketch = session.document().get(sketch).unwrap().clone();
    let canonical_plane = session.document().get(plane).unwrap().clone();
    let mut changed = session.document().get(offset).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut changed.data else {
        panic!()
    };
    *expression = "2".into();
    let result = apply(&mut session, vec![Mutation::Replace(changed)]);
    assert!(result.invalidated.contains(&child_feature));
    let rebuilt = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let moved = rebuilt.snapshot().shapes[&child_feature].bounds;
    assert!((moved.min[0] - 18.8).abs() < 1e-9);
    assert!((moved.max[0] - 19.8).abs() < 1e-9);
    assert_eq!(moved.min[2], 4.);
    assert_eq!(session.document().get(sketch).unwrap(), &canonical_sketch);
    assert_eq!(session.document().get(plane).unwrap(), &canonical_plane);
}

#[test]
fn oblique_cylinder_occurrence_keeps_exact_circular_descriptors_and_world_step_geometry() {
    let mut session = Session::new(Document::new("Circular occurrence").unwrap()).unwrap();
    let part = PrimitiveRequest::new(
        "Cylinder",
        PrimitiveParameters::Cylinder {
            radius: length(2.),
            height: length(5.),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(&session, 0, actor(), &part).unwrap();
    session.commit(prepared.prepared).unwrap();
    let control = NumericTransform {
        translation: [length(10.), length(-5.), length(2.)],
        pivot: [length(2.), length(1.), length(-4.)],
        axis: [1., 2., 3.],
        angle: angle(37.5),
    };
    let pose = control.resolve(session.parameters()).unwrap();
    let request = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: part.body,
                producer: part.feature,
            },
        },
        control,
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&request),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let occurrence = &native.snapshot().occurrences[&request.instance];
    let solid = native.solid(request.instance).unwrap();
    assert!((solid.volume_mm3().unwrap() - 20. * std::f64::consts::PI).abs() < 1e-8);
    assert_eq!(solid.solid_count().unwrap(), 1);
    let circles: Vec<_> = occurrence
        .topology
        .elements
        .iter()
        .filter_map(|e| match e.geometry {
            Some(AnalyticGeometry::CircularArc {
                frame, radius_mm, ..
            }) => Some((frame, radius_mm)),
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 2);
    for (frame, radius) in circles {
        assert_eq!(radius, 2.);
        assert!([[0., 0., 0.], [0., 0., 5.]].into_iter().any(|point| {
            point
                .into_iter()
                .enumerate()
                .all(|(i, _)| (pose.point(point)[i] - frame.origin_mm[i]).abs() < 1e-8)
        }));
    }
    let path = std::env::temp_dir().join(format!("multicad-occurrence-{}.step", EntityId::new()));
    solid.write_step(&path).unwrap();
    let imported = multicad_kernel::Solid::read_step(&path, Default::default()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!((imported.volume_mm3().unwrap() - 20. * std::f64::consts::PI).abs() < 1e-8);
    let bounds = imported.bounds().unwrap();
    for (a, b) in bounds.min.into_iter().chain(bounds.max).zip(
        occurrence
            .bounds
            .min
            .into_iter()
            .chain(occurrence.bounds.max),
    ) {
        assert!((a - b).abs() < 1e-7);
    }
}
