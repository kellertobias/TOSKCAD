use multicad_core::{
    commands::{Actor, Channel, ErrorCode, Request, Session},
    constraint_sketch::source::Source,
    document::*,
    references::*,
    sketch::{self, CreationPlane, Datum, Frame, PlaneOptions},
};
use std::collections::{BTreeMap, BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn session() -> Session {
    Session::new(Document::new("Sketch planes").unwrap()).unwrap()
}
fn near(a: [f64; 3], b: [f64; 3]) {
    for (actual, expected) in a.into_iter().zip(b) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }
}
fn frame_near(a: Frame, b: Frame) {
    near(a.origin, b.origin);
    near(a.u, b.u);
    near(a.v, b.v);
    near(a.n, b.n);
}
fn feature(name: &str, inputs: Vec<Link>) -> Entity {
    Entity::new(
        name,
        EntityData::Feature {
            definition: Definition {
                type_id: "test.profile_extrusion".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs,
            suppressed: false,
        },
    )
}
fn face_snapshot(owner: EntityId, revision: u64, height: f64) -> TopologySnapshot {
    let origin = Origin {
        source: owner,
        role: "cap.end".into(),
    };
    TopologySnapshot {
        owner,
        revision,
        elements: vec![TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Face,
                ordinal: 5,
            },
            origins: BTreeSet::from([origin.clone()]),
            boundary: BTreeSet::new(),
            frames: vec![AnchoredFrame {
                origin,
                frame: ReferenceFrame {
                    origin_mm: [0., 0., height],
                    u: [1., 0., 0.],
                    v: [0., 1., 0.],
                },
            }],
        }],
    }
}
fn source_session() -> (Session, EntityId, TopologySnapshot) {
    let source = feature("Extrusion", vec![]);
    let snapshot = face_snapshot(source.id, 0, 18.);
    let mut document = Document::new("Face plane").unwrap();
    document.insert(source.clone()).unwrap();
    (Session::new(document).unwrap(), source.id, snapshot)
}
fn reference_report(
    document: &Document,
    revision: u64,
    snapshot: TopologySnapshot,
) -> ReferenceReport {
    inspect(
        document,
        revision,
        &BTreeMap::from([(snapshot.owner, snapshot)]),
    )
}
#[test]
fn datum_creation_is_source_neutral_and_one_attributable_undo_redo_command() {
    for datum in Datum::ALL {
        let mut session = session();
        let before = session.document().clone();
        let preview = sketch::prepare_new(
            &session,
            0,
            actor(),
            "Outline",
            CreationPlane::Datum(datum),
            PlaneOptions::default(),
        )
        .unwrap();
        assert_eq!(session.document(), &before);
        assert!(!session.can_undo());
        assert_eq!(preview.prepared.actor(), &actor());
        frame_near(preview.frame, datum.frame());
        let (source, frame, plane) =
            Source::read(preview.prepared.document(), preview.sketch).unwrap();
        assert!(source.sketch.curves.is_empty());
        assert_eq!(plane, preview.plane);
        frame_near(frame, preview.frame);
        drop(preview);
        assert_eq!(session.document(), &before);
        let preview = sketch::prepare_new(
            &session,
            0,
            actor(),
            "Outline",
            CreationPlane::Datum(datum),
            PlaneOptions::default(),
        )
        .unwrap();
        let ids = (preview.sketch, preview.plane);
        session.commit(preview.prepared).unwrap();
        assert_eq!(session.revision(), 1);
        let after = session.document().clone();
        assert_eq!(session.activity().next().unwrap().actor, actor());
        session.undo(1, actor()).unwrap();
        assert_eq!(session.document(), &before);
        session.redo(2, actor()).unwrap();
        assert_eq!(session.document(), &after);
        assert!(session.document().get(ids.0).is_some());
        assert!(session.document().get(ids.1).is_some());
        assert_eq!(
            sketch::prepare_new(
                &session,
                0,
                actor(),
                "Stale",
                CreationPlane::Datum(datum),
                PlaneOptions::default()
            )
            .err()
            .unwrap()
            .code,
            ErrorCode::StaleRevision
        );
    }
}
#[test]
fn offsets_and_sketch_angles_use_parent_local_axes_on_xz_and_yz_and_survive_chains() {
    for datum in Datum::ALL {
        let mut session = session();
        let options = PlaneOptions {
            normal_offset_mm: 8.,
            angle_rad: std::f64::consts::FRAC_PI_2,
        };
        let first = sketch::prepare_new(
            &session,
            0,
            actor(),
            "Offset",
            CreationPlane::Datum(datum),
            options,
        )
        .unwrap();
        let id = first.sketch;
        let plane = first.plane;
        let first_frame = first.frame;
        frame_near(first_frame, options.apply(datum.frame()).unwrap());
        session.commit(first.prepared).unwrap();
        let (source, frame, _) = Source::read(session.document(), id).unwrap();
        near(source.u, [0., 1., 0.]); // Descriptor is local, including XZ and YZ.
        frame_near(frame, first_frame);
        let EntityData::Plane {
            anchor: Some(link), ..
        } = &session.document().get(plane).unwrap().data
        else {
            panic!("offset plane")
        };
        assert_eq!(link.role, sketch::OFFSET_PLANE_ROLE);
        let parent = link.target;
        let inspected = sketch::inspect_plane(session.document(), plane, 1, None).unwrap();
        near(inspected.u, datum.frame().u); // Plane itself has no sketch roll.
        let second_options = PlaneOptions {
            normal_offset_mm: 3.,
            angle_rad: 0.3,
        };
        let second = sketch::prepare_new(
            &session,
            1,
            actor(),
            "Chained",
            CreationPlane::Existing {
                id: plane,
                report: None,
            },
            second_options,
        )
        .unwrap();
        let second_id = second.sketch;
        let expected = second_options.apply(inspected).unwrap();
        frame_near(second.frame, expected);
        session.commit(second.prepared).unwrap();
        frame_near(
            Source::read(session.document(), second_id).unwrap().1,
            expected,
        );
        let mut changed = session.document().get(parent).unwrap().clone();
        let EntityData::Plane { origin_mm, .. } = &mut changed.data else {
            unreachable!()
        };
        *origin_mm = [10., 20., 30.];
        session
            .apply(Request {
                expected_revision: 2,
                actor: actor(),
                label: "Move parent datum".into(),
                edits: vec![Mutation::Replace(changed)],
            })
            .unwrap();
        let moved = Source::read(session.document(), second_id).unwrap().1;
        near(
            moved.origin,
            std::array::from_fn(|i| [10., 20., 30.][i] + datum.frame().n[i] * 11.),
        );
        near(moved.u, expected.u);
        let loaded: Document =
            serde_json::from_slice(&serde_json::to_vec(session.document()).unwrap()).unwrap();
        frame_near(Source::read(&loaded, second_id).unwrap().1, moved);
    }
}
#[test]
fn existing_plain_plane_is_reused_without_a_hidden_roll_dependency() {
    let mut session = session();
    let initial = sketch::prepare_new(
        &session,
        0,
        actor(),
        "Datum",
        CreationPlane::Datum(Datum::Xz),
        PlaneOptions::default(),
    )
    .unwrap();
    let plane = initial.plane;
    session.commit(initial.prepared).unwrap();
    let count = session.document().entities().count();
    let second = sketch::prepare_new(
        &session,
        1,
        actor(),
        "Second",
        CreationPlane::Existing {
            id: plane,
            report: None,
        },
        PlaneOptions::default(),
    )
    .unwrap();
    assert_eq!(second.plane, plane);
    assert_eq!(second.prepared.document().entities().count(), count + 1);
    frame_near(second.frame, Datum::Xz.frame());
}
#[test]
fn named_face_attachment_preserves_offsets_on_dimension_edits_and_keeps_part_history_acyclic() {
    let (mut session, producer, snapshot) = source_session();
    let key = snapshot.elements[0].key;
    let options = PlaneOptions {
        normal_offset_mm: 2.,
        angle_rad: 0.4,
    };
    let new = sketch::prepare_new(
        &session,
        0,
        actor(),
        "Face profile",
        CreationPlane::Face {
            snapshot: &snapshot,
            key,
            anchor: None,
        },
        options,
    )
    .unwrap();
    let plane = new.plane;
    let sketch = new.sketch;
    let expected = options
        .apply(Frame {
            origin: [0., 0., 18.],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
            n: [0., 0., 1.],
        })
        .unwrap();
    frame_near(new.frame, expected);
    let EntityData::Plane {
        anchor: Some(link), ..
    } = &new.prepared.document().get(plane).unwrap().data
    else {
        panic!("face plane")
    };
    assert_eq!(link.target, producer);
    let reference = link.geometry.as_ref().unwrap().clone();
    assert_eq!(
        reference.attachment.as_ref().unwrap().offset_mm,
        [0., 0., 2.]
    );
    session.commit(new.prepared).unwrap();
    assert!(Source::read(session.document(), sketch).is_err());
    let current = face_snapshot(producer, 1, 24.);
    let report = reference_report(session.document(), 1, current);
    let (mut source, frame, _) =
        Source::read_evaluated(session.document(), sketch, 1, &report).unwrap();
    near(frame.origin, [0., 0., 26.]);
    assert_eq!(report.entries[0].intent.id, reference.id);
    let a = source.add_point([0., 0.]);
    let b = source.add_point([30., 0.]);
    source.add_line(a, b);
    let downstream = feature(
        "Next extrusion",
        vec![Link::new(sketch, "profile"), Link::new(producer, "base")],
    );
    let part = Entity::new(
        "Same Part",
        EntityData::Part {
            features: vec![producer, downstream.id],
            material: None,
            tags: Default::default(),
        },
    );
    let mut edits = source.replace(session.document(), sketch).unwrap();
    edits.extend([Mutation::Insert(downstream), Mutation::Insert(part)]);
    session
        .apply(Request {
            expected_revision: 1,
            actor: actor(),
            label: "Append downstream operation".into(),
            edits,
        })
        .unwrap();
    assert!(session.document().reference_diagnostics().is_empty());
    let current = reference_report(session.document(), 2, face_snapshot(producer, 2, 32.));
    near(
        Source::read_evaluated(session.document(), sketch, 2, &current)
            .unwrap()
            .1
            .origin,
        [0., 0., 34.],
    );
    assert_eq!(current.entries[0].intent.id, reference.id);
}
#[test]
fn face_creation_rejects_untrusted_owner_kind_stale_curved_and_ambiguous_frames() {
    let (mut session, producer, mut snapshot) = source_session();
    let key = snapshot.elements[0].key;
    let before = session.document().clone();
    let create = |snapshot: &TopologySnapshot, key, anchor| {
        sketch::prepare_new(
            &session,
            0,
            actor(),
            "Face",
            CreationPlane::Face {
                snapshot,
                key,
                anchor,
            },
            PlaneOptions::default(),
        )
    };
    snapshot.revision = 1;
    assert_eq!(
        create(&snapshot, key, None).err().unwrap().code,
        ErrorCode::StaleRevision
    );
    snapshot.revision = 0;
    snapshot.elements[0].frames.clear();
    assert!(create(&snapshot, key, None).is_err());
    snapshot = face_snapshot(producer, 0, 18.);
    snapshot.elements[0].origins.clear();
    snapshot.elements[0].frames.clear();
    assert!(create(&snapshot, key, None).is_err());
    snapshot = face_snapshot(producer, 0, 18.);
    let another = Origin {
        source: producer,
        role: "second.planar-source".into(),
    };
    snapshot.elements[0].origins.insert(another.clone());
    let frame = snapshot.elements[0].frames[0].frame;
    snapshot.elements[0].frames.push(AnchoredFrame {
        origin: another.clone(),
        frame,
    });
    assert!(create(&snapshot, key, None).is_err());
    assert!(create(&snapshot, key, Some(&another)).is_ok());
    assert!(
        create(
            &snapshot,
            TopologyKey {
                kind: TopologyKind::Edge,
                ordinal: 0
            },
            None
        )
        .is_err()
    );
    assert_eq!(session.document(), &before);
    let part = Entity::new(
        "Final Part",
        EntityData::Part {
            features: vec![producer],
            material: None,
            tags: Default::default(),
        },
    );
    let instance = Entity::new(
        "Occurrence",
        EntityData::Instance {
            component: part.id,
            placement: Default::default(),
        },
    );
    session
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Part placement".into(),
            edits: vec![
                Mutation::Insert(part.clone()),
                Mutation::Insert(instance.clone()),
            ],
        })
        .unwrap();
    for owner in [part.id, instance.id] {
        let snapshot = face_snapshot(owner, 1, 18.);
        assert!(
            sketch::prepare_new(
                &session,
                1,
                actor(),
                "Wrong owner",
                CreationPlane::Face {
                    snapshot: &snapshot,
                    key,
                    anchor: None
                },
                PlaneOptions::default()
            )
            .is_err()
        );
    }
}
#[test]
fn rotated_attached_parent_and_offsets_use_current_frame_without_invalid_fallback_u() {
    let (mut session, producer, snapshot) = source_session();
    let new = sketch::prepare_new(
        &session,
        0,
        actor(),
        "Face",
        CreationPlane::Face {
            snapshot: &snapshot,
            key: snapshot.elements[0].key,
            anchor: None,
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let parent = new.plane;
    session.commit(new.prepared).unwrap();
    let mut current = face_snapshot(producer, 1, 24.);
    current.elements[0].frames[0].frame = ReferenceFrame {
        origin_mm: [5., 6., 7.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let report = reference_report(session.document(), 1, current);
    let base = sketch::inspect_plane(session.document(), parent, 1, Some(&report)).unwrap();
    near(base.n, [1., 0., 0.]);
    let reuse = sketch::prepare_new(
        &session,
        1,
        actor(),
        "Reused rotated face",
        CreationPlane::Existing {
            id: parent,
            report: Some(&report),
        },
        PlaneOptions::default(),
    )
    .unwrap();
    let (source, frame, _) =
        Source::read_evaluated(reuse.prepared.document(), reuse.sketch, 1, &report).unwrap();
    near(source.u, [1., 0., 0.]); // Perpendicular to saved +Z normal, unlike live U.
    frame_near(frame, base);
    let options = PlaneOptions {
        normal_offset_mm: 4.,
        angle_rad: std::f64::consts::FRAC_PI_2,
    };
    let offset = sketch::prepare_new(
        &session,
        1,
        actor(),
        "Offset rotated face",
        CreationPlane::Existing {
            id: parent,
            report: Some(&report),
        },
        options,
    )
    .unwrap();
    frame_near(
        Source::read_evaluated(offset.prepared.document(), offset.sketch, 1, &report)
            .unwrap()
            .1,
        options.apply(base).unwrap(),
    );
    near(offset.frame.origin, [9., 6., 7.]);
    assert!(sketch::inspect_plane(session.document(), parent, 1, None).is_err());
    let mut stale = report.clone();
    stale.revision = 0;
    assert!(sketch::inspect_plane(session.document(), parent, 1, Some(&stale)).is_err());
    let missing = inspect(session.document(), 1, &Default::default());
    assert!(sketch::inspect_plane(session.document(), parent, 1, Some(&missing)).is_err());
}
#[test]
fn invalid_offsets_and_missing_parent_never_become_world_datum_fallbacks() {
    let session = session();
    for options in [
        PlaneOptions {
            normal_offset_mm: f64::NAN,
            angle_rad: 0.,
        },
        PlaneOptions {
            normal_offset_mm: 0.,
            angle_rad: f64::INFINITY,
        },
    ] {
        assert!(
            sketch::prepare_new(
                &session,
                0,
                actor(),
                "Invalid",
                CreationPlane::Datum(Datum::Xy),
                options
            )
            .is_err()
        );
    }
    let parent = EntityId::new();
    let plane = Entity::new(
        "Missing offset parent",
        EntityData::Plane {
            origin_mm: [0., 0., 4.],
            normal: [0., 0., 1.],
            anchor: Some(Link::new(parent, sketch::OFFSET_PLANE_ROLE)),
        },
    );
    let document = session
        .document()
        .edited(&[Mutation::Insert(plane.clone())])
        .unwrap();
    assert!(sketch::inspect_plane(&document, plane.id, 0, None).is_err());
    let mut invalid = plane.clone();
    if let EntityData::Plane { normal, .. } = &mut invalid.data {
        *normal = [0., 1., 0.];
    }
    assert!(document.edited(&[Mutation::Replace(invalid)]).is_err());
}
#[test]
fn moving_offset_sketch_converts_local_basis_when_detaching_its_plane() {
    for datum in [Datum::Xz, Datum::Yz] {
        let mut session = session();
        let preview = sketch::prepare_new(
            &session,
            0,
            actor(),
            "Moved offset",
            CreationPlane::Datum(datum),
            PlaneOptions {
                normal_offset_mm: 8.,
                angle_rad: 0.7,
            },
        )
        .unwrap();
        let id = preview.sketch;
        session.commit(preview.prepared).unwrap();
        let (mut source, before, _) = Source::read(session.document(), id).unwrap();
        let a = source.add_point([5., 10.]);
        let b = source.add_point([30., 20.]);
        source.add_line(a, b);
        session
            .apply(Request {
                expected_revision: 1,
                actor: actor(),
                label: "Geometry".into(),
                edits: source.replace(session.document(), id).unwrap(),
            })
            .unwrap();
        let edits = sketch::translate(session.document(), id, [10., 20., 30.]).unwrap();
        session
            .apply(Request {
                expected_revision: 2,
                actor: actor(),
                label: "Move offset sketch".into(),
                edits,
            })
            .unwrap();
        let (after, frame, plane) = Source::read(session.document(), id).unwrap();
        near(
            frame.origin,
            std::array::from_fn(|i| before.origin[i] + [10., 20., 30.][i]),
        );
        near(frame.u, before.u);
        near(after.u, before.u);
        assert_eq!(after.sketch, source.sketch);
        assert!(matches!(
            &session.document().get(plane).unwrap().data,
            EntityData::Plane { anchor: None, .. }
        ));
    }
}
#[test]
fn explicit_face_repair_reorients_attached_sketch_and_associated_offset_without_rebasing_source() {
    let (mut session, producer, snapshot) = source_session();
    let parent_options = PlaneOptions {
        normal_offset_mm: 2.,
        angle_rad: 0.2,
    };
    let parent = sketch::prepare_new(
        &session,
        0,
        actor(),
        "Face parent",
        CreationPlane::Face {
            snapshot: &snapshot,
            key: snapshot.elements[0].key,
            anchor: None,
        },
        parent_options,
    )
    .unwrap();
    let parent_plane = parent.plane;
    let parent_sketch = parent.sketch;
    session.commit(parent.prepared).unwrap();
    let report = reference_report(session.document(), 1, face_snapshot(producer, 1, 18.));
    let reference = report.entries[0].address;
    let child_options = PlaneOptions {
        normal_offset_mm: 3.,
        angle_rad: 0.4,
    };
    let child = sketch::prepare_new(
        &session,
        1,
        actor(),
        "Child offset",
        CreationPlane::Existing {
            id: parent_plane,
            report: Some(&report),
        },
        child_options,
    )
    .unwrap();
    let child_sketch = child.sketch;
    session.commit(child.prepared).unwrap();
    let replacement = feature("Explicit replacement producer", vec![]);
    session
        .apply(Request {
            expected_revision: 2,
            actor: actor(),
            label: "Replacement producer".into(),
            edits: vec![Mutation::Insert(replacement.clone())],
        })
        .unwrap();
    let mut current = face_snapshot(replacement.id, 3, 18.);
    current.elements[0].frames[0].frame = ReferenceFrame {
        origin_mm: [10., 20., 30.],
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let repair = prepare_repair(
        &session,
        3,
        actor(),
        reference,
        &current,
        current.elements[0].key,
        None,
    )
    .unwrap();
    assert!(repair.affected.contains(&child_sketch));
    session.commit(repair.prepared).unwrap();
    current.revision = 4;
    let evaluated = reference_report(session.document(), 4, current);
    assert!(evaluated.all_resolved());
    let expected_parent = parent_options
        .apply(Frame {
            origin: [10., 20., 30.],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
            n: [1., 0., 0.],
        })
        .unwrap();
    frame_near(
        Source::read_evaluated(session.document(), parent_sketch, 4, &evaluated)
            .unwrap()
            .1,
        expected_parent,
    );
    let expected_child = child_options.apply(expected_parent).unwrap();
    frame_near(
        Source::read_evaluated(session.document(), child_sketch, 4, &evaluated)
            .unwrap()
            .1,
        expected_child,
    );
    near(expected_child.origin, [15., 20., 30.]);
    let loaded: Document =
        serde_json::from_slice(&serde_json::to_vec(session.document()).unwrap()).unwrap();
    frame_near(
        Source::read_evaluated(&loaded, child_sketch, 4, &evaluated)
            .unwrap()
            .1,
        expected_child,
    );
}
