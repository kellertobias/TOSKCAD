use multicad_core::{
    commands::*, constraint_sketch::source::Source, document::*, references::*, sketch,
};
use multicad_kernel::*;
use std::collections::BTreeMap;
fn origin(source: EntityId, role: &str) -> Origin {
    Origin {
        source,
        role: role.into(),
    }
}
fn p() -> GeometryPolicy {
    GeometryPolicy::default()
}
fn feature(name: &str, inputs: Vec<Link>) -> Entity {
    Entity::new(
        name,
        EntityData::Feature {
            definition: Definition {
                type_id: "test.native-solid".into(),
                schema_version: 1,
                data: serde_json::json!({}),
            },
            inputs,
            suppressed: false,
        },
    )
}
fn cap(snapshot: &TopologySnapshot, feature: EntityId) -> TopologyKey {
    snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face && e.origins.contains(&origin(feature, "cap.end"))
        })
        .unwrap()
        .key
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn translated(solid: &Solid, t: [f64; 3]) -> Solid {
    solid
        .transformed(RigidTransform {
            translation_mm: t,
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn named_prism_faces_edges_vertices_have_unique_capturable_provenance() {
    let feature = EntityId::new();
    let solid = Solid::box_named([100., 60., 18.], feature, p()).unwrap();
    let snapshot = solid.topology_snapshot(feature, 0).unwrap();
    for (kind, count) in [
        (TopologyKind::Face, 6),
        (TopologyKind::Edge, 12),
        (TopologyKind::Vertex, 8),
    ] {
        assert_eq!(
            snapshot
                .elements
                .iter()
                .filter(|e| e.key.kind == kind)
                .count(),
            count
        );
    }
    for element in &snapshot.elements {
        let intent = GeometryIntent::capture(&snapshot, element.key).unwrap();
        assert!(
            matches!(resolve(&intent,&snapshot,0),Resolution::Resolved { key, .. } if key==element.key)
        );
    }
    let loaded: TopologySnapshot =
        serde_json::from_slice(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(snapshot, loaded);
}
#[test]
fn real_face_sketch_and_hole_follow_dimension_changes_and_refuse_unresolved_fallback() {
    let panel = feature("Panel", vec![]);
    let initial = Solid::box_named([100., 60., 18.], panel.id, p())
        .unwrap()
        .topology_snapshot(panel.id, 0)
        .unwrap();
    let mut intent = GeometryIntent::capture(&initial, cap(&initial, panel.id)).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin(panel.id, "cap.end"),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let mut anchor = Link::new(panel.id, "face");
    anchor.geometry = Some(intent.clone());
    let mut doc = Document::new("Face sketch").unwrap();
    doc.insert(panel.clone()).unwrap();
    let plane = Entity::new(
        "Top face",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(anchor),
        },
    );
    doc.insert(plane.clone()).unwrap();
    let mut source = Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    };
    let center = source.add_point([20., 25.]);
    source
        .sketch
        .curves
        .push(multicad_core::constraint_sketch::Curve {
            id: EntityId::new(),
            construction: false,
            kind: multicad_core::constraint_sketch::CurveKind::Circle {
                center,
                radius_mm: 3.,
            },
        });
    let sketch = Entity::new(
        "Drill circle",
        EntityData::Sketch {
            plane: plane.id,
            definition: source.definition().unwrap(),
            inputs: vec![],
        },
    );
    doc.insert(sketch.clone()).unwrap();
    let hole = feature("Hole", vec![Link::new(sketch.id, "profile")]);
    doc.insert(hole.clone()).unwrap();
    let mut session = Session::new(doc).unwrap();
    for (revision, width, height) in [(0, 100., 18.), (1, 120., 24.), (2, 80., 8.)] {
        if revision > 0 {
            let mut next = panel.clone();
            let EntityData::Feature { definition, .. } = &mut next.data else {
                unreachable!()
            };
            definition.data = serde_json::json!({"width":width,"height":height});
            session
                .apply(Request {
                    expected_revision: revision - 1,
                    actor: Actor::local(Channel::Ui),
                    label: "Dimensions".into(),
                    edits: vec![Mutation::Replace(next)],
                })
                .unwrap();
        }
        let solid = Solid::box_named([width, 60., height], panel.id, p()).unwrap();
        let snapshot = solid.topology_snapshot(panel.id, revision).unwrap();
        let report = inspect(
            session.document(),
            revision,
            &BTreeMap::from([(panel.id, snapshot)]),
        );
        assert!(report.all_resolved());
        let (_, frame, _) =
            Source::read_evaluated(session.document(), sketch.id, revision, &report).unwrap();
        assert_eq!(frame.point([20., 25.]), [20., 25., height]);
        assert!(Source::read(session.document(), sketch.id).is_err());
        let (_, facade, _) =
            sketch::Sketch::read_evaluated(session.document(), sketch.id, revision, &report)
                .unwrap();
        assert_eq!(facade, frame);
        let circle = Profile {
            frame: PlaneFrame {
                origin: frame.point([0., 0.]),
                u: frame.u,
                v: frame.v,
            },
            outer: vec![ProfileEdge::Circle {
                center: [20., 25.],
                radius_mm: 3.,
            }],
            holes: vec![],
        };
        let tool = Solid::extrude_profile_named(
            &circle,
            -height,
            hole.id,
            &[origin(sketch.id, "circle")],
            p(),
        )
        .unwrap();
        let drilled = solid
            .boolean(&tool, BooleanOperation::Subtract, p())
            .unwrap()
            .unwrap();
        near(
            drilled.volume_mm3().unwrap(),
            width * 60. * height - 9. * std::f64::consts::PI * height,
        );
        let after_drill = drilled.topology_snapshot(panel.id, revision).unwrap();
        assert!(
            matches!(resolve(&intent,&after_drill,revision),Resolution::Resolved { frame: Some(f),.. } if f.origin_mm[2]==height)
        );
        assert!(
            Source::read_evaluated(session.document(), sketch.id, revision + 1, &report).is_err()
        );
        let unresolved = inspect(session.document(), revision, &BTreeMap::new());
        assert!(
            Source::read_evaluated(session.document(), sketch.id, revision, &unresolved).is_err()
        );
    }
}
#[test]
fn real_cut_split_reports_ambiguity_and_explicit_fragment_repair_survives_rebuild() {
    let panel = feature("Panel", vec![]);
    let cutter = EntityId::new();
    let result_id = feature("Slot", vec![Link::new(panel.id, "base")]);
    let base = Solid::box_named([100., 60., 18.], panel.id, p()).unwrap();
    let initial = base.topology_snapshot(result_id.id, 0).unwrap();
    let intent = GeometryIntent::capture(&initial, cap(&initial, panel.id)).unwrap();
    let mut link = Link::new(result_id.id, "face");
    link.geometry = Some(intent.clone());
    let plane = Entity::new(
        "Attached",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: Some(link),
        },
    );
    let dependent = feature("Dependent hole", vec![Link::new(plane.id, "plane")]);
    let address = ReferenceAddress {
        owner: plane.id,
        reference: intent.id,
    };
    let mut doc = Document::new("Slot repair").unwrap();
    for e in [panel.clone(), result_id.clone(), plane, dependent.clone()] {
        doc.insert(e).unwrap();
    }
    let mut session = Session::new(doc).unwrap();
    let cut = translated(
        &Solid::box_named([10., 80., 30.], cutter, p()).unwrap(),
        [45., -10., -5.],
    );
    let split = base
        .boolean(&cut, BooleanOperation::Subtract, p())
        .unwrap()
        .unwrap();
    assert_eq!(split.solid_count().unwrap(), 2);
    let snapshot = split.topology_snapshot(result_id.id, 0).unwrap();
    let Resolution::Ambiguous { candidates } = resolve(&intent, &snapshot, 0) else {
        panic!("split top must be ambiguous")
    };
    assert_eq!(candidates.len(), 2);
    let report = inspect(
        session.document(),
        0,
        &BTreeMap::from([(result_id.id, snapshot.clone())]),
    );
    assert!(
        report
            .get(address)
            .unwrap()
            .affected
            .contains(&dependent.id)
    );
    let mut gate = ReferenceGate::default();
    let valid_report = inspect(
        session.document(),
        0,
        &BTreeMap::from([(result_id.id, initial)]),
    );
    gate.consider(0, base.mesh(MeshOptions::default()).unwrap(), valid_report)
        .unwrap();
    let prior = gate.visible().unwrap().clone();
    assert!(
        !gate
            .consider(0, split.mesh(MeshOptions::default()).unwrap(), report)
            .unwrap()
    );
    assert_eq!(gate.visible(), Some(&prior));
    let preview = prepare_repair(
        &session,
        0,
        Actor::local(Channel::Mcp),
        address,
        &snapshot,
        candidates[0],
        None,
    )
    .unwrap();
    session.commit(preview.prepared).unwrap();
    let repaired = session.document().get(address.owner).unwrap().references()[0]
        .geometry
        .clone()
        .unwrap();
    let new_base = Solid::box_named([120., 60., 24.], panel.id, p()).unwrap();
    let rebuilt = new_base
        .boolean(&cut, BooleanOperation::Subtract, p())
        .unwrap()
        .unwrap()
        .topology_snapshot(result_id.id, 1)
        .unwrap();
    assert!(matches!(
        resolve(&repaired, &rebuilt, 1),
        Resolution::Resolved { .. }
    ));
    session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(
        session.document().get(address.owner).unwrap().references()[0].geometry,
        Some(intent)
    );
}
#[test]
fn deleted_face_does_not_rebind_to_a_new_parallel_face_and_transform_preserves_source_frame() {
    let panel = EntityId::new();
    let base = Solid::box_named([100., 60., 18.], panel, p()).unwrap();
    let snapshot = base.topology_snapshot(panel, 0).unwrap();
    let mut intent = GeometryIntent::capture(&snapshot, cap(&snapshot, panel)).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin(panel, "cap.end"),
        offset_mm: [5., 7., 0.],
        angle_rad: 0.,
    });
    let tool = translated(
        &Solid::box_named([120., 80., 10.], EntityId::new(), p()).unwrap(),
        [-10., -10., 12.],
    );
    let cut = base
        .boolean(&tool, BooleanOperation::Subtract, p())
        .unwrap()
        .unwrap();
    assert_eq!(
        resolve(&intent, &cut.topology_snapshot(panel, 1).unwrap(), 1),
        Resolution::Missing
    );
    let moved = base
        .transformed(RigidTransform {
            rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
            translation_mm: [10., 20., 30.],
        })
        .unwrap();
    let Resolution::Resolved {
        frame: Some(frame), ..
    } = resolve(&intent, &moved.topology_snapshot(panel, 1).unwrap(), 1)
    else {
        panic!("transformed frame missing")
    };
    assert_eq!(frame.origin_mm, [3., 25., 48.]);
    assert_eq!(frame.u, [0., 1., 0.]);
}
#[test]
fn wire_winding_and_hole_sides_keep_their_authored_curve_sources() {
    let feature = EntityId::new();
    let sketch = EntityId::new();
    let mut profile = Profile::rectangle(40., 30.);
    profile.holes.push(vec![ProfileEdge::Circle {
        center: [20., 15.],
        radius_mm: 3.,
    }]);
    let curves: Vec<_> = (0..5)
        .map(|i| origin(sketch, &format!("curve-{i}")))
        .collect();
    let first = Solid::extrude_profile_named(&profile, 5., feature, &curves, p())
        .unwrap()
        .topology_snapshot(feature, 0)
        .unwrap();
    profile.outer.reverse();
    for edge in &mut profile.outer {
        if let ProfileEdge::Line { a, b } = edge {
            std::mem::swap(a, b);
        }
    }
    let mut reversed = curves.clone();
    reversed[..4].reverse();
    let next = Solid::extrude_profile_named(&profile, 8., feature, &reversed, p())
        .unwrap()
        .topology_snapshot(feature, 1)
        .unwrap();
    for face in first
        .elements
        .iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
    {
        let intent = GeometryIntent::capture(&first, face.key).unwrap();
        assert!(matches!(
            resolve(&intent, &next, 1),
            Resolution::Resolved { .. }
        ));
    }
    let bad = vec![curves[0].clone(); 5];
    assert!(Solid::extrude_profile_named(&profile, 5., feature, &bad, p()).is_err());
    let unnamed = Solid::box_solid([1.; 3], p())
        .unwrap()
        .topology_snapshot(feature, 0)
        .unwrap();
    assert!(GeometryIntent::capture(&unnamed, unnamed.elements[0].key).is_err());
    for value in [-10., 0., f64::NAN, f64::INFINITY, 1e8] {
        assert!(Solid::box_named([value, 10., 10.], feature, p()).is_err());
    }
}
#[test]
fn worker_receipts_publish_owned_reference_data_with_revision_guards() {
    let feature = EntityId::new();
    let worker = GeometryWorker::new().unwrap();
    let program = |revision, height| GeometryProgram {
        revision,
        policy: p(),
        operations: vec![GeometryOperation::NamedBox {
            size_mm: [100., 60., height],
            feature,
        }],
        outputs: vec![0],
        mesh: Default::default(),
    };
    let first = worker.submit(program(0, 18.)).unwrap().wait().unwrap();
    let first = first.solids[0].as_ref().unwrap().topology_snapshot(feature);
    let mut intent = GeometryIntent::capture(&first, cap(&first, feature)).unwrap();
    intent.attachment = Some(Attachment {
        anchor: origin(feature, "cap.end"),
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let second = worker.submit(program(1, 24.)).unwrap().wait().unwrap();
    let second = second.solids[0]
        .as_ref()
        .unwrap()
        .topology_snapshot(feature);
    assert_eq!(
        resolve(&intent, &first, 1),
        Resolution::Stale {
            snapshot_revision: 0
        }
    );
    assert!(
        matches!(resolve(&intent,&second,1),Resolution::Resolved {frame:Some(frame),..} if frame.origin_mm[2]==24.)
    );
}
