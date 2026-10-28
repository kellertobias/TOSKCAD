//! Real OCCT geometry with a canonical component-local outline. Occurrence pose
//! is derived for editing, while face-history links stay on the producer.
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::source::Source,
    document::*,
    references::*,
    sketch::{self, CreationPlane, Datum, PlaneOptions},
};
use multicad_kernel::{MeshOptions, Profile, Solid};
use multicad_ui::{
    sketch_context,
    viewport::{Geometry, Instance, Scene, SceneStamp, Transform},
};
use std::sync::Arc;
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: [f64; 3], b: [f64; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < 1e-8, "{a} != {b}");
    }
}
fn feature(name: &str, inputs: Vec<Link>) -> Entity {
    Entity::new(
        name,
        EntityData::Feature {
            definition: Definition {
                type_id: "context.test.extrusion".into(),
                schema_version: 1,
                data: serde_json::json!({"height_mm":18.}),
            },
            inputs,
            suppressed: false,
        },
    )
}
struct Fixture {
    session: Session,
    producer: EntityId,
    part: EntityId,
    outline: EntityId,
    datum: EntityId,
    placed: EntityId,
    identity: EntityId,
    other: EntityId,
    geometry: Arc<Geometry>,
}
impl Fixture {
    fn new() -> Self {
        let mut session = Session::new(Document::new("Occurrence contexts").unwrap()).unwrap();
        let outline = sketch::prepare_new(
            &session,
            0,
            actor(),
            "Local base outline",
            CreationPlane::Datum(Datum::Xy),
            PlaneOptions::default(),
        )
        .unwrap();
        let outline_id = outline.sketch;
        let datum = outline.plane;
        session.commit(outline.prepared).unwrap();
        let (mut source, _, _) = Source::read(session.document(), outline_id).unwrap();
        source.add_rectangle([0., 0.], [100., 60.]).unwrap();
        let producer = feature(
            "Native outline extrusion",
            vec![Link::new(outline_id, "profile")],
        );
        let part = Entity::new(
            "Extruded component",
            EntityData::Part {
                features: vec![producer.id],
                material: None,
                tags: Default::default(),
            },
        );
        let identity = Entity::new(
            "Origin occurrence",
            EntityData::Instance {
                component: part.id,
                placement: Default::default(),
                placement_source: None,
            },
        );
        let placed = Entity::new(
            "Rotated occurrence",
            EntityData::Instance {
                component: part.id,
                placement: Placement {
                    translation_mm: [140., 25., 40.],
                    rotation_xyzw: [
                        0.,
                        0.,
                        std::f64::consts::FRAC_1_SQRT_2,
                        std::f64::consts::FRAC_1_SQRT_2,
                    ],
                },
                placement_source: None,
            },
        );
        let other = Entity::new(
            "Another occurrence",
            EntityData::Instance {
                component: part.id,
                placement: Placement {
                    translation_mm: [-160., 0., 0.],
                    ..Default::default()
                },
                placement_source: None,
            },
        );
        let mut edits = source.replace(session.document(), outline_id).unwrap();
        edits.extend(
            [
                producer.clone(),
                part.clone(),
                identity.clone(),
                placed.clone(),
                other.clone(),
            ]
            .into_iter()
            .map(Mutation::Insert),
        );
        session
            .apply(Request {
                expected_revision: 1,
                actor: actor(),
                label: "Extrude component with placements".into(),
                edits,
            })
            .unwrap();
        let origins = source
            .sketch
            .curves
            .iter()
            .map(|c| Origin {
                source: c.id,
                role: "authored curve".into(),
            })
            .collect::<Vec<_>>();
        let solid = Solid::extrude_profile_named(
            &Profile::rectangle(100., 60.),
            18.,
            producer.id,
            &origins,
            Default::default(),
        )
        .unwrap();
        let mesh = solid.mesh(MeshOptions::default()).unwrap();
        let geometry = Arc::new(
            Geometry::new(
                producer.id,
                mesh.vertices.as_chunks::<3>().0.to_vec(),
                mesh.indices,
                mesh.face_indices,
                solid
                    .topology_snapshot(producer.id, session.revision())
                    .unwrap()
                    .elements,
            )
            .unwrap(),
        );
        Self {
            session,
            producer: producer.id,
            part: part.id,
            outline: outline_id,
            datum,
            placed: placed.id,
            identity: identity.id,
            other: other.id,
            geometry,
        }
    }
    fn scene(&self) -> Scene {
        // Identity deliberately comes last, proving that it beats first-match
        // ordering when no occurrence has been explicitly chosen.
        let instances = [self.placed, self.other, self.identity]
            .into_iter()
            .map(|id| {
                let EntityData::Instance { placement, .. } =
                    self.session.document().get(id).unwrap().data
                else {
                    unreachable!()
                };
                Instance {
                    id,
                    geometry: self.producer,
                    transform: Transform::from_placement(placement).unwrap(),
                    color: [140, 170, 195, 255],
                }
            })
            .collect();
        Scene::new(
            SceneStamp {
                document: self.session.document().id(),
                revision: self.session.revision(),
                generation: self.session.revision() + 1,
            },
            vec![self.geometry.clone()],
            instances,
        )
        .unwrap()
    }
    fn face_sketch(&mut self, options: PlaneOptions) -> (EntityId, EntityId) {
        let scene = self.scene();
        let snapshot = scene.geometry_snapshot(self.producer).unwrap();
        let key = top(&snapshot, self.producer);
        let new = sketch::prepare_new(
            &self.session,
            self.session.revision(),
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
        let ids = (new.sketch, new.plane);
        self.session.commit(new.prepared).unwrap();
        ids
    }
}
fn top(snapshot: &TopologySnapshot, producer: EntityId) -> TopologyKey {
    snapshot
        .elements
        .iter()
        .find(|e| {
            e.key.kind == TopologyKind::Face
                && e.origins.contains(&Origin {
                    source: producer,
                    role: "cap.end".into(),
                })
        })
        .unwrap()
        .key
}
fn current_report(fixture: &Fixture, scene: &Scene) -> ReferenceReport {
    inspect(
        fixture.session.document(),
        fixture.session.revision(),
        &scene.reference_snapshots(),
    )
}
#[test]
fn new_attached_sketch_uses_producing_component_pose_before_any_feature_consumes_it() {
    let mut fixture = Fixture::new();
    let (id, plane) = fixture.face_sketch(PlaneOptions {
        normal_offset_mm: 2.,
        angle_rad: std::f64::consts::FRAC_PI_2,
    });
    let scene = fixture.scene();
    let before = fixture.session.document().clone();
    let selected = sketch_context::occurrence(&before, id, &scene, Some(fixture.placed)).unwrap();
    assert_eq!(selected.0, fixture.placed);
    let evaluated = current_report(&fixture, &scene);
    let (_, local, _) =
        Source::read_evaluated(&before, id, fixture.session.revision(), &evaluated).unwrap();
    near(local.origin, [0., 0., 20.]);
    let world = selected.1.frame(ReferenceFrame {
        origin_mm: local.origin,
        u: local.u,
        v: local.v,
    });
    near(world.origin_mm, [140., 25., 60.]);
    near(world.u, [-1., 0., 0.]);
    near(world.v, [0., -1., 0.]);
    assert_eq!(
        sketch_context::occurrence(&before, plane, &scene, Some(fixture.placed))
            .unwrap()
            .0,
        fixture.placed
    );
    let EntityData::Plane {
        anchor: Some(link), ..
    } = &before.get(plane).unwrap().data
    else {
        panic!("face anchor")
    };
    assert_eq!(link.target, fixture.producer);
    assert_ne!(link.target, fixture.placed);
    assert_eq!(fixture.session.document(), &before);
    // Appending a suppressed history operation is geometry-neutral and proves
    // that this same Part may consume the newly created sketch without a cycle.
    let mut downstream = feature(
        "Following operation",
        vec![
            Link::new(id, "profile"),
            Link::new(fixture.producer, "base"),
        ],
    );
    if let EntityData::Feature { suppressed, .. } = &mut downstream.data {
        *suppressed = true;
    }
    let mut part = before.get(fixture.part).unwrap().clone();
    if let EntityData::Part { features, .. } = &mut part.data {
        features.push(downstream.id);
    }
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: actor(),
            label: "Append same-component operation".into(),
            edits: vec![Mutation::Insert(downstream), Mutation::Replace(part)],
        })
        .unwrap();
    assert!(
        fixture
            .session
            .document()
            .reference_diagnostics()
            .is_empty()
    );
    assert_eq!(
        sketch_context::occurrence(
            fixture.session.document(),
            id,
            &fixture.scene(),
            Some(fixture.placed)
        )
        .unwrap()
        .0,
        fixture.placed
    );
}
#[test]
fn preferred_shared_occurrence_wins_otherwise_identity_is_used_and_offsets_follow_the_producer() {
    let mut fixture = Fixture::new();
    let (parent, plane) = fixture.face_sketch(PlaneOptions::default());
    let scene = fixture.scene();
    let report = current_report(&fixture, &scene);
    let new = sketch::prepare_new(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        "Offset face profile",
        CreationPlane::Existing {
            id: plane,
            report: Some(&report),
        },
        PlaneOptions {
            normal_offset_mm: 3.,
            angle_rad: 0.4,
        },
    )
    .unwrap();
    let child = new.sketch;
    let offset_plane = new.plane;
    fixture.session.commit(new.prepared).unwrap();
    let scene = fixture.scene();
    assert_eq!(scene.geometries().len(), 1);
    assert_eq!(scene.instances().len(), 3);
    assert!(
        scene
            .instances()
            .iter()
            .all(|i| i.geometry == fixture.producer)
    );
    assert!(Arc::ptr_eq(
        &fixture.geometry,
        &scene.geometries()[&fixture.producer]
    ));
    let before = fixture.session.document().clone();
    for seed in [parent, child, offset_plane] {
        assert_eq!(
            sketch_context::occurrence(&before, seed, &scene, None)
                .unwrap()
                .0,
            fixture.identity
        );
        assert_eq!(
            sketch_context::occurrence(&before, seed, &scene, Some(fixture.other))
                .unwrap()
                .0,
            fixture.other
        );
        assert_eq!(
            sketch_context::occurrence(&before, seed, &scene, Some(EntityId::new()))
                .unwrap()
                .0,
            fixture.identity
        );
        let (_, pose) =
            sketch_context::occurrence(&before, seed, &scene, Some(fixture.placed)).unwrap();
        assert_eq!(
            pose,
            scene
                .instances()
                .iter()
                .find(|i| i.id == fixture.placed)
                .unwrap()
                .transform
        );
    }
    let report = current_report(&fixture, &scene);
    let (_, local, _) =
        Source::read_evaluated(&before, child, fixture.session.revision(), &report).unwrap();
    let (_, pose) =
        sketch_context::occurrence(&before, child, &scene, Some(fixture.placed)).unwrap();
    let world = pose.frame(ReferenceFrame {
        origin_mm: local.origin,
        u: local.u,
        v: local.v,
    });
    near(world.origin_mm, [140., 25., 61.]);
    near(world.u, [-0.4_f64.sin(), 0.4_f64.cos(), 0.]);
    assert_eq!(fixture.session.document(), &before);
}
#[test]
fn independent_world_datum_stays_world_while_its_consumed_sketch_chooses_component_pose() {
    let fixture = Fixture::new();
    let scene = fixture.scene();
    let before = fixture.session.document().clone();
    assert!(
        sketch_context::occurrence(&before, fixture.datum, &scene, Some(fixture.placed)).is_none()
    );
    let (id, pose) =
        sketch_context::occurrence(&before, fixture.outline, &scene, Some(fixture.placed)).unwrap();
    assert_eq!(id, fixture.placed);
    near(pose.point([0.; 3]), [140., 25., 40.]);
    let independent = Entity::new(
        "Unused datum",
        EntityData::Plane {
            origin_mm: [10., 20., 30.],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    let document = before
        .edited(&[Mutation::Insert(independent.clone())])
        .unwrap();
    assert!(
        sketch_context::occurrence(&document, independent.id, &scene, Some(fixture.placed))
            .is_none()
    );
    let foreign = Scene::new(
        SceneStamp {
            document: EntityId::new(),
            ..scene.stamp()
        },
        vec![fixture.geometry.clone()],
        scene.instances().to_vec(),
    )
    .unwrap();
    assert!(sketch_context::occurrence(&before, fixture.outline, &foreign, None).is_none());
    assert_eq!(fixture.session.document(), &before);
}
#[test]
fn legacy_explicit_instance_anchor_is_already_world_and_never_receives_a_second_pose() {
    let mut fixture = Fixture::new();
    let scene = fixture.scene();
    let world = scene.topology_snapshot(fixture.placed).unwrap();
    let key = top(&world, fixture.producer);
    let mut intent = GeometryIntent::capture(&world, key).unwrap();
    intent.attachment = Some(Attachment {
        anchor: Origin {
            source: fixture.producer,
            role: "cap.end".into(),
        },
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let mut link = Link::new(fixture.placed, "face");
    link.geometry = Some(intent);
    let plane = Entity::new(
        "Existing world anchor",
        EntityData::Plane {
            origin_mm: [140., 25., 58.],
            normal: [0., 0., 1.],
            anchor: Some(link),
        },
    );
    let source = Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    };
    let sketch = Entity::new(
        "Existing world sketch",
        EntityData::Sketch {
            plane: plane.id,
            definition: source.definition().unwrap(),
            inputs: vec![],
        },
    );
    fixture
        .session
        .apply(Request {
            expected_revision: fixture.session.revision(),
            actor: actor(),
            label: "Restore legacy world attachment".into(),
            edits: vec![
                Mutation::Insert(plane.clone()),
                Mutation::Insert(sketch.clone()),
            ],
        })
        .unwrap();
    let scene = fixture.scene();
    let before = fixture.session.document().clone();
    let report = current_report(&fixture, &scene);
    assert!(report.all_resolved());
    let (_, frame, _) =
        Source::read_evaluated(&before, sketch.id, fixture.session.revision(), &report).unwrap();
    near(frame.origin, [140., 25., 58.]);
    near(frame.u, [0., 1., 0.]);
    for seed in [sketch.id, plane.id] {
        assert!(sketch_context::occurrence(&before, seed, &scene, Some(fixture.placed)).is_none());
        assert!(
            sketch_context::occurrence(&before, seed, &scene, Some(fixture.identity)).is_none()
        );
    }
    assert_eq!(fixture.session.document(), &before);
}
