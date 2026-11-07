//! Real native cache/pose updates, without recomposing the owning connection.
#[path = "../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::Dimension,
    document::{Entity, EntityData, EntityId, Mutation},
    model::*,
    parameters::BoundExpression,
    references::AnalyticGeometry,
};
use multicad_model::{Compiler, EvaluationCache};
fn actor() -> Actor {
    Actor::local(Channel::CommandLine)
}
fn expression(s: &Session, name: &str) -> Dimension {
    Dimension::Expression {
        expression: BoundExpression::bind(s.document(), &format!("=${name}"), "mm", None).unwrap(),
    }
}
fn scalar(s: &mut Session, name: &str, value: &str) -> EntityId {
    let e = Entity::new(
        name,
        EntityData::Parameter {
            expression: value.into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let id = e.id;
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Add live scalar".into(),
        edits: vec![Mutation::Insert(e)],
    })
    .unwrap();
    id
}
fn change(s: &mut Session, id: EntityId, value: &str) {
    let mut e = s.document().get(id).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut e.data else {
        unreachable!()
    };
    *expression = value.into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Change live scalar only".into(),
        edits: vec![Mutation::Replace(e)],
    })
    .unwrap();
}
#[test]
fn native_cached_connection_follows_washer_underhead_and_captured_far_face_without_recomposition() {
    let (document, mut request, stock) = fixture::fixture();
    let mut s = Session::new(document).unwrap();
    let height = scalar(&mut s, "receiving_height", "10");
    let washer_t = scalar(&mut s, "washer_thickness", "0.2");
    let underhead = scalar(&mut s, "screw_underhead", "24");
    let mut primitive = read_primitive(s.document(), stock).unwrap();
    let PrimitiveParameters::Box { size, .. } = &mut primitive.parameters else {
        unreachable!()
    };
    size[2] = expression(&s, "receiving_height");
    let q =
        prepare_primitive_edit(&s, s.revision(), actor(), stock, &primitive.parameters).unwrap();
    s.commit(q.prepared).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let original = compiler.compile(s.document(), s.revision()).unwrap();
    let topology = &original.snapshot().shapes[&stock].topology;
    let far = topology
        .elements
        .iter()
        .find(|e| {
            matches!(e.geometry,Some(AnalyticGeometry::Plane{frame}) if frame.normal()[2]< -0.99)
                && !e.frames.is_empty()
        })
        .unwrap();
    request.parameters.end_reference = Some(ScrewConnectionEndReference {
        receiver: request.receivers[0].source.input.clone(),
        occurrence: request.receivers[0].source.occurrence.clone(),
        face: capture_profile_face(topology, far.key, None).unwrap(),
    });
    request.parameters.grip = length(0.);
    request.receivers[0].source.entry_reference = Some(request.axis.face.clone());
    let ScrewConnectionHardwareSource::Generated { part } = &mut request.hardware[0].source else {
        unreachable!()
    };
    let MechanicalPartParameters::MachineScrew {
        length_input,
        thread,
        ..
    } = &mut part.parameters
    else {
        unreachable!()
    };
    *length_input = Some(ScrewLengthInput::Underhead {
        value: expression(&s, "screw_underhead"),
    });
    thread.length = length(23.5);
    let washer = MechanicalPartRequest::new(
        "Live head washer",
        MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: expression(&s, "washer_thickness"),
            frame: Default::default(),
        },
    );
    request.hardware.insert(
        1,
        ScrewConnectionHardwareRequest {
            role_id: EntityId::new(),
            role: ScrewConnectionHardwareRole::HeadWasher,
            name: "Live washer occurrence".into(),
            source: ScrewConnectionHardwareSource::Generated {
                part: Box::new(washer),
            },
            instance: EntityId::new(),
            driver: EntityId::new(),
        },
    );
    drop(original);
    let q = prepare_screw_connection(&s, s.revision(), actor(), &request).unwrap();
    s.commit(q.prepared).unwrap();
    for row in &request.hardware {
        let driver = read_screw_hardware_placement(s.document(), row.driver).unwrap();
        assert_eq!(
            driver.connection,
            Some(ScrewHardwareConnectionBinding {
                connection: request.connection,
                role: row.role_id,
            })
        );
        assert_eq!(driver.station, length(0.));
        assert_eq!(driver.roll, angle(0.));
        assert_eq!(driver.local_mount, RigidPose::default());
    }
    let record = s.document().get(request.connection).unwrap().clone();
    let mut cache = EvaluationCache::default();
    let mut prior_hash = None;
    let mut prior_nut = None;
    for (h, w, u, edit) in [
        (10., 0.2, 24., None),
        (10., 0.3, 24., Some((washer_t, "0.3"))),
        (10., 0.3, 24.25, Some((underhead, "24.25"))),
        (18., 0.3, 24.25, Some((height, "18"))),
    ] {
        if let Some((id, value)) = edit {
            change(&mut s, id, value);
        }
        assert_eq!(
            s.document().get(request.connection).unwrap(),
            &record,
            "No connection recomposition"
        );
        eprintln!("Evaluating live native h={h}, washer={w}, U={u}");
        let native = compiler
            .compile_cached(&mut cache, s.document(), s.revision(), None)
            .unwrap();
        let check = &native.snapshot().screw_connection_checks[0];
        assert!((check.grip_mm - h).abs() < 1e-7);
        assert!(check.native_component_collision_checked);
        assert!((check.actual_engagement_mm.unwrap() - 2.4).abs() < 1e-7);
        assert_eq!(native.snapshot().leaf_occurrences.len(), 4);
        let screw = &request.hardware[0];
        let nut = &request.hardware[2];
        let path = |id| OccurrencePath {
            root: id,
            members: vec![],
        };
        let screw_pose = native.leaf_snapshot(&path(screw.instance)).unwrap().pose;
        let nut_pose = native.leaf_snapshot(&path(nut.instance)).unwrap().pose;
        // Axis starts at native top Z=h and points down. Screw neck is -w.
        assert!((screw_pose.translation_mm[2] - (h + w)).abs() < 1e-7);
        assert!(
            native
                .leaf_solid(&path(screw.instance))
                .unwrap()
                .boolean(
                    native.leaf_solid(&path(nut.instance)).unwrap(),
                    multicad_kernel::BooleanOperation::Intersect,
                    Default::default()
                )
                .unwrap()
                .is_none()
        );
        if let Some(hash) = prior_hash {
            assert_ne!(native.snapshot().stamp.source_hash, hash);
        }
        if let Some(pose) = prior_nut.filter(|_| h < 18.) {
            assert_ne!(nut_pose, pose, "Washer/U-B updates actual nut phase");
        }
        prior_hash = Some(native.snapshot().stamp.source_hash);
        prior_nut = Some(nut_pose);
        assert!(u > h + w + 2.4);
    }
    let valid = s.document().clone();
    // A candidate with stock beyond the available screw length cannot publish Ready.
    let mut invalid = Session::new(valid.clone()).unwrap();
    change(&mut invalid, height, "30");
    assert!(
        compiler
            .compile_cached(&mut cache, invalid.document(), invalid.revision(), None)
            .is_err()
    );
    assert_eq!(s.document(), &valid);
    assert!(
        compiler
            .compile_cached(&mut cache, s.document(), s.revision(), None)
            .is_ok()
    );
}
