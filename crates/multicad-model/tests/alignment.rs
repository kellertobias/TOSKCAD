use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityData, EntityId},
    model::{
        self, AlignmentAnchor, AlignmentInput, AlignmentParameters, AlignmentRequest, BodyInput,
        PrimitiveFrame, PrimitiveParameters, PrimitiveRequest, length,
    },
    references::{Attachment, GeometryIntent, Origin, TopologyKind},
    sketch,
};
use multicad_model::{Compiler, Model};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn box_(session: &mut Session, name: &str, origin: [f64; 3], size: [f64; 3]) -> PrimitiveRequest {
    let request = PrimitiveRequest::new(
        name,
        PrimitiveParameters::Box {
            size: size.map(length),
            frame: PrimitiveFrame {
                origin: origin.map(length),
                ..Default::default()
            },
        },
    );
    let prepared =
        model::prepare_primitive(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    request
}
fn cap(model: &Model, source: EntityId, role: &str) -> AlignmentInput {
    let snapshot = &model.snapshot().shapes[&source].topology;
    let origin = Origin {
        source,
        role: role.into(),
    };
    let face = snapshot
        .elements
        .iter()
        .find(|e| e.key.kind == TopologyKind::Face && e.origins.contains(&origin))
        .unwrap();
    let mut intent = GeometryIntent::capture(snapshot, face.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    AlignmentInput { source, intent }
}
fn snapshots(
    model: &Model,
) -> std::collections::BTreeMap<EntityId, multicad_core::references::TopologySnapshot> {
    model
        .snapshot()
        .shapes
        .iter()
        .map(|(id, shape)| (*id, shape.topology.clone()))
        .collect()
}
#[test]
fn native_face_alignment_uses_outward_normals_named_uv_and_upstream_geometry() {
    let mut session = Session::new(Document::new("Referenced face placement").unwrap()).unwrap();
    let moving = box_(&mut session, "Moving", [30., 0., 0.], [2., 3., 4.]);
    let target = box_(&mut session, "Target", [0.; 3], [10.; 3]);
    let compiler = Compiler::default();
    let original = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let a = cap(&original, moving.feature, "cap.start");
    let b = cap(&original, target.feature, "cap.end");
    let parameters = AlignmentParameters {
        source: AlignmentAnchor::Geometry {
            reference: a.intent.id,
        },
        target: AlignmentAnchor::Geometry {
            reference: b.intent.id,
        },
        opposed: true,
        ..Default::default()
    };
    let request = AlignmentRequest::new(
        BodyInput {
            body: moving.body,
            producer: moving.feature,
        },
        parameters,
        vec![a, b],
    );
    let before = session.document().clone();
    let prepared =
        model::prepare_alignment(&session, session.revision(), actor(), &request).unwrap();
    let native = compiler
        .compile(prepared.prepared.document(), session.revision() + 1)
        .unwrap();
    assert_eq!(session.document(), &before);
    let shape = &native.snapshot().shapes[&request.feature];
    assert_eq!(shape.bounds.min, [0., 0., 10.]);
    assert_eq!(shape.bounds.max, [2., 3., 14.]);
    assert!((shape.volume_mm3 - 24.).abs() < 1e-9);
    assert_eq!(
        native.snapshot().shapes[&target.feature].bounds.max,
        [10.; 3]
    );
    session.commit(prepared.prepared).unwrap();
    let frozen = model::read_alignment(session.document(), request.feature).unwrap();
    let updated = PrimitiveParameters::Box {
        size: [length(10.), length(10.), length(20.)],
        frame: Default::default(),
    };
    let prepared = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        target.feature,
        &updated,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let rebuilt = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        rebuilt.snapshot().shapes[&request.feature].bounds.min,
        [0., 0., 20.]
    );
    assert_eq!(
        rebuilt.snapshot().shapes[&request.feature].bounds.max,
        [2., 3., 24.]
    );
    assert_eq!(
        model::read_alignment(session.document(), request.feature).unwrap(),
        frozen
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(
        compiler
            .compile(session.document(), session.revision())
            .unwrap()
            .snapshot()
            .shapes[&request.feature]
            .bounds
            .min,
        [0., 0., 10.]
    );
}
#[test]
fn stale_ambiguous_missing_and_unframed_references_never_become_a_numeric_fallback() {
    let mut session = Session::new(Document::new("Failed alignments").unwrap()).unwrap();
    let a = box_(&mut session, "A", [0.; 3], [2.; 3]);
    let b = box_(&mut session, "B", [10., 0., 0.], [2.; 3]);
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    let from = cap(&native, a.feature, "cap.end");
    let to = cap(&native, b.feature, "cap.start");
    let parameters = AlignmentParameters {
        source: AlignmentAnchor::Geometry {
            reference: from.intent.id,
        },
        target: AlignmentAnchor::Geometry {
            reference: to.intent.id,
        },
        ..Default::default()
    };
    let refs = vec![from, to];
    let current = snapshots(&native);
    assert!(
        model::resolve_alignment(
            session.document(),
            session.revision() + 1,
            session.parameters(),
            &parameters,
            &refs,
            &current,
            Default::default()
        )
        .is_err()
    );
    let mut missing = current.clone();
    missing.remove(&b.feature);
    assert!(
        model::resolve_alignment(
            session.document(),
            session.revision(),
            session.parameters(),
            &parameters,
            &refs,
            &missing,
            Default::default()
        )
        .is_err()
    );
    let mut ambiguous = current.clone();
    let target = ambiguous.get_mut(&b.feature).unwrap();
    let original = target
        .elements
        .iter()
        .find(|e| e.origins == refs[1].intent.origins)
        .unwrap()
        .clone();
    let mut duplicate = original;
    duplicate.key.ordinal = 999;
    target.elements.push(duplicate);
    assert!(
        model::resolve_alignment(
            session.document(),
            session.revision(),
            session.parameters(),
            &parameters,
            &refs,
            &ambiguous,
            Default::default()
        )
        .unwrap_err()
        .to_string()
        .contains("ambiguous")
    );
    let mut unframed = refs;
    unframed[0].intent.attachment = None;
    assert!(
        model::resolve_alignment(
            session.document(),
            session.revision(),
            session.parameters(),
            &parameters,
            &unframed,
            &current,
            Default::default()
        )
        .unwrap_err()
        .to_string()
        .contains("authored frame")
    );
}
#[test]
fn datum_alignment_stays_associated_and_axis_up_parallel_is_explicitly_rejected() {
    let mut session = Session::new(Document::new("Datum alignment").unwrap()).unwrap();
    let moving = box_(&mut session, "Moving", [0.; 3], [2.; 3]);
    let (_, edits) = sketch::create(sketch::Datum::Xy, "Target datum").unwrap();
    let mut doc = session.document().edited(&edits).unwrap();
    let plane = edits
        .iter()
        .find_map(|e| {
            if let multicad_core::document::Mutation::Insert(entity) = e
                && matches!(entity.data, EntityData::Plane { .. })
            {
                Some(entity.id)
            } else {
                None
            }
        })
        .unwrap();
    let mut entity = doc.get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut entity.data else {
        panic!()
    };
    *origin_mm = [10., 20., 30.];
    doc = doc
        .edited(&[multicad_core::document::Mutation::Replace(entity)])
        .unwrap();
    let session = Session::new(doc).unwrap();
    let parameters = AlignmentParameters {
        target: AlignmentAnchor::Plane { plane },
        ..Default::default()
    };
    let request = AlignmentRequest::new(
        BodyInput {
            body: moving.body,
            producer: moving.feature,
        },
        parameters,
        vec![],
    );
    let prepared = model::prepare_alignment(&session, 0, actor(), &request).unwrap();
    let native = Compiler::default()
        .compile(prepared.prepared.document(), 1)
        .unwrap();
    assert_eq!(
        native.snapshot().shapes[&request.feature].bounds.min,
        [10., 20., 30.]
    );
    let bad = AlignmentParameters {
        mode: model::AlignmentMode::Axis,
        ..Default::default()
    };
    assert!(
        model::resolve_alignment(
            session.document(),
            0,
            session.parameters(),
            &bad,
            &[],
            &snapshots(&native),
            Default::default()
        )
        .unwrap_err()
        .to_string()
        .contains("up vector")
    );
}

#[test]
fn instance_face_alignment_and_copied_reference_chains_follow_target_without_mutating_source_part()
{
    let mut session = Session::new(Document::new("Instance reference placement").unwrap()).unwrap();
    let moving = box_(
        &mut session,
        "Moving definition",
        [30., 0., 0.],
        [2., 3., 4.],
    );
    let target = box_(&mut session, "Fixed target", [0.; 3], [10.; 3]);
    let original_part = session.document().get(moving.body).unwrap().clone();
    let copied = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: moving.body,
                producer: moving.feature,
            },
        },
        model::NumericTransform {
            translation: [length(100.), length(0.), length(0.)],
            angle: model::angle(90.),
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
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let from = cap(&native, moving.feature, "cap.start");
    let to = cap(&native, target.feature, "cap.end");
    let parameters = AlignmentParameters {
        source: AlignmentAnchor::Geometry {
            reference: from.intent.id,
        },
        target: AlignmentAnchor::Geometry {
            reference: to.intent.id,
        },
        opposed: true,
        ..Default::default()
    };
    let request = model::InstanceAlignmentRequest::new(copied.instance, parameters, vec![from, to]);
    let before = session.document().clone();
    let prepared = model::prepare_instance_alignments(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&request),
    )
    .unwrap();
    let preview = compiler
        .compile(prepared.prepared.document(), session.revision() + 1)
        .unwrap();
    assert_eq!(session.document(), &before);
    assert_eq!(
        preview.snapshot().occurrences[&copied.instance].bounds.min,
        [0., 0., 10.]
    );
    assert_eq!(
        preview.snapshot().occurrences[&copied.instance].bounds.max,
        [2., 3., 14.]
    );
    session.commit(prepared.prepared).unwrap();
    assert_eq!(session.document().get(moving.body).unwrap(), &original_part);
    let duplicate = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Instance {
            instance: copied.instance,
        },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&duplicate),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        native.snapshot().occurrences[&copied.instance].pose,
        native.snapshot().occurrences[&duplicate.instance].pose
    );
    let project =
        multicad_core::project_file::ProjectFile::new(session.document().clone()).unwrap();
    assert!(
        project
            .capabilities()
            .contains(model::INSTANCE_ALIGNMENT_CAPABILITY)
    );
    let mut bytes = vec![];
    multicad_core::project_file::write(&project, &mut bytes, Default::default()).unwrap();
    let opened = multicad_core::project_file::read(
        &mut std::io::Cursor::new(bytes),
        Default::default(),
        &multicad_core::project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(opened.project.document, *session.document());
    let saved = model::read_instance_alignment(session.document(), request.driver).unwrap();
    let prepared = model::prepare_primitive_edit(
        &session,
        session.revision(),
        actor(),
        target.feature,
        &PrimitiveParameters::Box {
            size: [length(10.), length(10.), length(20.)],
            frame: Default::default(),
        },
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    for id in [copied.instance, duplicate.instance] {
        assert_eq!(native.snapshot().occurrences[&id].bounds.min, [0., 0., 20.]);
        assert_eq!(native.snapshot().occurrences[&id].bounds.max, [2., 3., 24.]);
    }
    assert_eq!(
        native.snapshot().shapes[&moving.feature].bounds.min,
        [30., 0., 0.]
    );
    assert_eq!(
        model::read_instance_alignment(session.document(), request.driver).unwrap(),
        saved
    );
}
fn geometry_input(
    model: &Model,
    source: EntityId,
    select: impl Fn(multicad_core::references::AnalyticGeometry) -> bool,
) -> AlignmentInput {
    let snapshot = &model.snapshot().shapes[&source].topology;
    let element = snapshot
        .elements
        .iter()
        .find(|e| e.geometry.is_some_and(&select))
        .unwrap();
    AlignmentInput {
        source,
        intent: GeometryIntent::capture(snapshot, element.key).unwrap(),
    }
}
#[test]
fn real_named_edges_and_vertices_drive_axis_and_position_alignment() {
    use multicad_core::references::AnalyticGeometry;
    for axis in [false, true] {
        let mut session = Session::new(Document::new("Analytic anchors").unwrap()).unwrap();
        let moving = box_(&mut session, "Moving", [30., 0., 0.], [2., 3., 4.]);
        let target = box_(&mut session, "Target", [0., 0., 10.], [10.; 3]);
        let compiler = Compiler::default();
        let native = compiler
            .compile(session.document(), session.revision())
            .unwrap();
        let from = geometry_input(&native, moving.feature, |geometry| {
            if axis {
                matches!(geometry,AnalyticGeometry::Line {a_mm,b_mm} if a_mm==[30.,0.,0.] && b_mm==[32.,0.,0.])
            } else {
                matches!(geometry,AnalyticGeometry::Point {position_mm} if position_mm==[30.,0.,0.])
            }
        });
        let to = geometry_input(&native, target.feature, |geometry| {
            if axis {
                matches!(geometry,AnalyticGeometry::Line {a_mm,b_mm} if a_mm==[10.,0.,10.] && b_mm==[10.,10.,10.])
            } else {
                matches!(geometry,AnalyticGeometry::Point {position_mm} if position_mm==[0.,0.,10.])
            }
        });
        let parameters = AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: from.intent.id,
            },
            target: AlignmentAnchor::Geometry {
                reference: to.intent.id,
            },
            mode: if axis {
                model::AlignmentMode::Axis
            } else {
                model::AlignmentMode::Position
            },
            ..Default::default()
        };
        let request = AlignmentRequest::new(
            BodyInput {
                body: moving.body,
                producer: moving.feature,
            },
            parameters,
            vec![from, to],
        );
        let prepared =
            model::prepare_alignment(&session, session.revision(), actor(), &request).unwrap();
        let result = compiler
            .compile(prepared.prepared.document(), session.revision() + 1)
            .unwrap();
        let shape = &result.snapshot().shapes[&request.feature];
        assert_eq!(
            shape.bounds.min,
            if axis { [7., 0., 10.] } else { [0., 0., 10.] }
        );
        assert_eq!(
            shape.bounds.max,
            if axis { [10., 2., 14.] } else { [2., 3., 14.] }
        );
        assert!((shape.volume_mm3 - 24.).abs() < 1e-9);
    }
}

#[test]
fn current_part_target_alias_follows_appended_and_edited_head_and_rejects_indirect_cycles() {
    let mut session = Session::new(Document::new("Current-head alignment").unwrap()).unwrap();
    let moving = box_(&mut session, "Moving", [30., 0., 0.], [2., 3., 4.]);
    let target = box_(&mut session, "Target", [0.; 3], [10.; 3]);
    let compiler = Compiler::default();
    let original = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let from = cap(&original, moving.feature, "cap.start");
    let mut to = cap(&original, target.feature, "cap.end");
    to.source = target.body;
    let request = AlignmentRequest::new(
        BodyInput {
            body: moving.body,
            producer: moving.feature,
        },
        AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: from.intent.id,
            },
            target: AlignmentAnchor::Geometry {
                reference: to.intent.id,
            },
            opposed: true,
            ..Default::default()
        },
        vec![from, to],
    );
    let prepared =
        model::prepare_alignment(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let transform = model::TransformRequest::new(
        BodyInput {
            body: target.body,
            producer: target.feature,
        },
        model::NumericTransform {
            translation: [length(10.), length(0.), length(5.)],
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_transform(&session, session.revision(), actor(), &transform).unwrap();
    session.commit(prepared.prepared).unwrap();
    let moved = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        moved.snapshot().shapes[&request.feature].bounds.min,
        [10., 0., 15.]
    );
    assert_eq!(
        moved.snapshot().body_sources[&target.body],
        transform.feature
    );
    assert_eq!(
        moved.solid(target.body).unwrap().bounds().unwrap().max,
        [20., 10., 15.]
    );
    let part_ids = session.document().get(target.body).unwrap().clone();
    let old_hash = moved.snapshot().stamp.source_hash;
    let changed = model::NumericTransform {
        translation: [length(20.), length(0.), length(7.)],
        ..Default::default()
    };
    let prepared = model::prepare_transform_edit(
        &session,
        session.revision(),
        actor(),
        transform.feature,
        &changed,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    assert_eq!(session.document().get(target.body).unwrap(), &part_ids);
    let current = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_ne!(current.snapshot().stamp.source_hash, old_hash);
    assert_eq!(
        current.snapshot().shapes[&request.feature].bounds.min,
        [20., 0., 17.]
    );
    assert_eq!(current.snapshot().references.revision, session.revision());
    assert!(current.snapshot().references.entries.iter().any(|diagnostic|diagnostic.source==target.body&&matches!(diagnostic.resolution,multicad_core::references::Resolution::Resolved {frame:Some(frame),..} if frame.origin_mm==[20.,0.,17.])));
    let mut source = cap(&current, target.feature, "cap.start");
    source.source = transform.feature;
    let mut destination = cap(&current, moving.feature, "cap.end");
    destination.source = moving.body;
    let cyclic = AlignmentRequest::new(
        BodyInput {
            body: target.body,
            producer: transform.feature,
        },
        AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: source.intent.id,
            },
            target: AlignmentAnchor::Geometry {
                reference: destination.intent.id,
            },
            ..Default::default()
        },
        vec![source, destination],
    );
    let before = session.document().clone();
    let error = model::prepare_alignment(&session, session.revision(), actor(), &cyclic)
        .expect_err("Cross-Part current-head dependencies must reject an indirect cycle");
    assert!(
        error.to_string().to_lowercase().contains("cycle"),
        "{error}"
    );
    assert_eq!(session.document(), &before);
}

#[test]
fn an_instance_local_component_alias_follows_later_source_geometry_transforms() {
    let mut session = Session::new(Document::new("Component-local current head").unwrap()).unwrap();
    let moving = box_(&mut session, "Moving", [30., 0., 0.], [2., 3., 4.]);
    let target = box_(&mut session, "Target", [0.; 3], [10.; 3]);
    let copied = model::InstanceCopyRequest::new(
        model::InstanceCopySource::Part {
            source: BodyInput {
                body: moving.body,
                producer: moving.feature,
            },
        },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&copied),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let compiler = Compiler::default();
    let native = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    let mut source = cap(&native, moving.feature, "cap.start");
    source.source = moving.body;
    let mut destination = cap(&native, target.feature, "cap.end");
    destination.source = target.body;
    let request = model::InstanceAlignmentRequest::new(
        copied.instance,
        AlignmentParameters {
            source: AlignmentAnchor::Geometry {
                reference: source.intent.id,
            },
            target: AlignmentAnchor::Geometry {
                reference: destination.intent.id,
            },
            opposed: true,
            ..Default::default()
        },
        vec![source, destination],
    );
    let prepared = model::prepare_instance_alignments(
        &session,
        session.revision(),
        actor(),
        std::slice::from_ref(&request),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let first = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        first.snapshot().occurrences[&copied.instance].bounds.min,
        [0., 0., 10.]
    );
    let transform = model::TransformRequest::new(
        BodyInput {
            body: moving.body,
            producer: moving.feature,
        },
        model::NumericTransform {
            translation: [length(5.), length(6.), length(7.)],
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_transform(&session, session.revision(), actor(), &transform).unwrap();
    session.commit(prepared.prepared).unwrap();
    let moved = compiler
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        moved.snapshot().occurrences[&copied.instance].bounds.min,
        [0., 0., 10.]
    );
    assert_eq!(
        moved.snapshot().occurrences[&copied.instance].bounds.max,
        [2., 3., 14.]
    );
    assert_eq!(
        moved.snapshot().shapes[&transform.feature].bounds.min,
        [35., 6., 7.]
    );
}
