use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
};
use multicad_kernel::{
    BooleanOperation, GeometryPolicy, PlaneFrame, RigidTransform, Solid, rust_kernel::Rational,
};
use multicad_model::{Compiler, GeometryBackend};

const BACKENDS: [GeometryBackend; 2] = [GeometryBackend::OpenCascade, GeometryBackend::Rust];

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn box_at(center: [f64; 2], width: f64, depth: f64) -> CompoundCutoutBox {
    CompoundCutoutBox::new(PlanRectangle {
        center: center.map(length),
        width: length(width),
        depth: length(depth),
        rotation_deg: 0.,
    })
}
fn fixture() -> (Session, CompoundCutoutRequest) {
    fixture_with_outline([[0., 0.], [6000., 0.], [6000., 4000.], [0., 4000.]])
}

fn fixture_with_outline(outline: [[f64; 2]; 4]) -> (Session, CompoundCutoutRequest) {
    let mut session = Session::new(Document::new("Native compound shaft").unwrap()).unwrap();
    let mut receivers = vec![];
    for (name, z) in [("Ground", 0.), ("Upper", 3000.)] {
        let (level, prepared) = prepare_level(
            &session,
            session.revision(),
            actor(),
            name,
            &LevelDefinition {
                elevation: length(z),
                height: length(3000.),
                slab_thickness: length(200.),
            },
        )
        .unwrap();
        session.commit(prepared).unwrap();
        let prepared = prepare_slab(
            &session,
            session.revision(),
            actor(),
            name,
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level,
                kind: SlabKind::Slab,
                outline: outline.map(|p| p.map(length)).to_vec(),
                thickness: None,
                holes: vec![],
            },
            None,
        )
        .unwrap();
        receivers.push(CompoundCutoutReceiver::new(BodyInput {
            body: prepared.outputs[0].body,
            producer: prepared.outputs[0].feature,
        }));
        session.commit(prepared.prepared).unwrap();
    }
    let request = CompoundCutoutRequest::new(
        "L shaft",
        CompoundCutoutDefinition {
            boxes: vec![
                box_at([2000., 2000.], 1000., 400.),
                box_at([1700., 2300.], 400., 1000.),
            ],
            receivers,
            extent: CompoundCutoutExtent::ThroughReceivers,
            levels: None,
        },
    );
    (session, request)
}
fn prepared(
    session: &Session,
    request: &CompoundCutoutRequest,
) -> multicad_core::model::ModelPrepared {
    prepare_compound_cutout(
        session,
        session.revision(),
        actor(),
        &CompoundCutoutChange::Set(Box::new(request.clone())),
    )
    .unwrap()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}
fn add_wall(session: &mut Session, y: f64) -> CompoundCutoutReceiver {
    let ty = NewType::new(
        "Block 200",
        NewTypeKind::Wall(WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: None,
        }),
    );
    let wall = WallRequest::new(
        "Receiving wall",
        WallDefinition {
            layer_placement: None,
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type: ty.id,
            path: [[1000., y], [5000., y]].map(|p| p.map(length)).to_vec(),
            closed: false,
            level: None,
            base_z: length(0.),
            thickness: None,
            height: None,
            justification: None,
            openings: vec![],
            cutouts: vec![],
        },
    );
    let prepared = prepare_wall_change(
        session,
        session.revision(),
        actor(),
        &[ty],
        &WallChange::Create(&wall),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    CompoundCutoutReceiver::new(BodyInput {
        body: wall.body,
        producer: wall.feature,
    })
}
#[test]
fn wall_only_slab_only_and_combined_cut_exactly_the_explicit_parts() {
    for (walls, slabs) in [(true, false), (false, true), (true, true)] {
        let (mut session, mut request) = fixture();
        let wall = add_wall(&mut session, 1750.);
        let floors = request.definition.receivers.clone();
        if !slabs {
            request.definition.receivers.clear();
        }
        if walls {
            request.definition.receivers.push(wall.clone());
        }
        if walls && slabs {
            let rectangle = &mut request.definition.boxes[1].rectangle;
            std::mem::swap(&mut rectangle.width, &mut rectangle.depth);
            rectangle.rotation_deg = 90.;
        }
        let prepared = prepared(&session, &request);
        session.commit(prepared.prepared).unwrap();
        for backend in BACKENDS {
            let model = Compiler::default()
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap_or_else(|e| panic!("{backend:?}: {e}"));
            let solid = model.solid(wall.target.body).unwrap();
            assert_eq!(solid.uses_rust_geometry(), backend == GeometryBackend::Rust);
            near(
                solid.volume_mm3().unwrap(),
                4000. * 200. * 2500. - if walls { 1000. * 50. * 2500. } else { 0. },
            );
            for floor in &floors {
                let solid = model.solid(floor.target.body).unwrap();
                assert_eq!(solid.uses_rust_geometry(), backend == GeometryBackend::Rust);
                near(
                    solid.volume_mm3().unwrap(),
                    6000. * 4000. * 200.
                        - if slabs {
                            (1000. * 400. + 400. * 1000. - 400. * 400.) * 200.
                        } else {
                            0.
                        },
                );
            }
        }
    }
}
#[test]
fn explicit_floor_range_cuts_only_the_selected_upper_slab_and_refuses_out_of_range_receivers() {
    let (session, mut request) = fixture();
    let ground = request.definition.receivers.remove(0);
    let upper = &request.definition.receivers[0];
    let level = read_slab(session.document(), upper.target.producer)
        .unwrap()
        .level;
    request.definition.levels = Some(CutoutLevelRange {
        from: level,
        to: None,
    });
    let change = prepared(&session, &request);
    let native = Compiler::default()
        .compile(change.prepared.document(), session.revision() + 1)
        .unwrap();
    near(
        native
            .solid(ground.target.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        6000. * 4000. * 200.,
    );
    near(
        native
            .solid(upper.target.body)
            .unwrap()
            .volume_mm3()
            .unwrap(),
        6000. * 4000. * 200. - (1000. * 400. + 400. * 1000. - 400. * 400.) * 200.,
    );
    request.definition.receivers.push(ground);
    let before = session.document().clone();
    let error = prepare_compound_cutout(
        &session,
        session.revision(),
        actor(),
        &CompoundCutoutChange::Set(Box::new(request)),
    )
    .expect_err("Explicit floor range must reject the ground receiver");
    assert!(error.to_string().contains("outside"));
    assert_eq!(session.document(), &before);
}
#[test]
fn chained_cutouts_rebuild_downstream_when_a_constituent_changes_and_restore_history_on_removal() {
    let (mut session, mut first) = fixture();
    let change = prepared(&session, &first);
    session.commit(change.prepared).unwrap();
    let second = CompoundCutoutRequest::new(
        "Second penetration",
        CompoundCutoutDefinition {
            boxes: vec![box_at([4300., 2000.], 500., 500.)],
            receivers: first
                .definition
                .receivers
                .iter()
                .map(|receiver| {
                    CompoundCutoutReceiver::new(BodyInput {
                        body: receiver.target.body,
                        producer: receiver.feature,
                    })
                })
                .collect(),
            extent: CompoundCutoutExtent::ThroughReceivers,
            levels: None,
        },
    );
    let change = prepared(&session, &second);
    session.commit(change.prepared).unwrap();
    first.definition.boxes.pop();
    let change = prepared(&session, &first);
    session.commit(change.prepared).unwrap();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    for receiver in &second.definition.receivers {
        near(
            native
                .solid(receiver.target.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            6000. * 4000. * 200. - (1000. * 400. + 500. * 500.) * 200.,
        );
    }
    let removal = prepare_compound_cutout(
        &session,
        session.revision(),
        actor(),
        &CompoundCutoutChange::Remove(second.feature),
    )
    .unwrap();
    session.commit(removal.prepared).unwrap();
    let native = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    for receiver in &first.definition.receivers {
        near(
            native
                .solid(receiver.target.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            6000. * 4000. * 200. - 1000. * 400. * 200.,
        );
        assert_eq!(
            native.snapshot().body_sources[&receiver.target.body],
            receiver.feature
        );
    }
}
#[test]
fn even_scoped_first_slab_preview_refuses_another_receivers_wall_split() {
    let (mut session, mut request) = fixture();
    let wall = add_wall(&mut session, 2000.);
    request.definition.receivers.push(wall);
    let original = session.document().clone();
    let prepared = prepared(&session, &request);
    for backend in BACKENDS {
        let result = Compiler::default().compile_sources_with_backend(
            prepared.prepared.document(),
            session.revision() + 1,
            &[request.definition.receivers[0].feature],
            backend,
            None,
        );
        let Err(error) = result else {
            panic!("Every receiving solid must be proved");
        };
        assert!(error.to_string().contains("split"), "{backend:?}: {error}");
    }
    assert_eq!(session.document(), &original);
}
#[test]
fn saved_compound_boxes_drive_linked_plan_and_section_after_edit_and_reopen() {
    use multicad_core::{
        drawings::*,
        project_file::{self, ProjectFile},
        references::ReferenceFrame,
    };
    let (mut session, mut request) = fixture();
    let change = prepared(&session, &request);
    session.commit(change.prepared).unwrap();
    let body = request.definition.receivers[0].target.body;
    let mut view = DrawingView {
        id: EntityId::new(),
        name: "Linked plan".into(),
        source: ViewSource::Model {
            entities: vec![body],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [50.; 2],
        scale: Scale {
            numerator: 1,
            denominator: 50,
        },
        hidden_lines: false,
        tangent_edges: false,
        paper_chord_tolerance_mm: 0.01,
        guide: None,
        equipment_categories: None,
    };
    let vectors = |session: &Session, view: &DrawingView| {
        let [native, rust] = BACKENDS.map(|backend| {
            let model = Compiler::default()
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap_or_else(|e| panic!("{backend:?}: {e}"));
            let ViewEvaluation::Ready { vectors, .. } = model
                .drawing_view(
                    session.document(),
                    session.revision(),
                    view,
                    &Default::default(),
                )
                .unwrap()
            else {
                panic!("{backend:?} linked drawing must resolve");
            };
            vectors
        });
        if matches!(view.kind, ViewKind::Section { .. }) {
            let sections = |vectors: &[DrawingVector]| {
                let mut sections: Vec<_> = vectors
                    .iter()
                    .filter(|vector| vector.role == VectorRole::Section)
                    .map(|vector| {
                        let VectorCurve::Line { a, b } = vector.curve else {
                            panic!("This cut section has only straight edges");
                        };
                        let mut ends = [a, b].map(|p| p.map(|c| (c * 1_000.).round() as i64));
                        ends.sort_unstable();
                        (vector.source, vector.material_part, ends)
                    })
                    .collect();
                sections.sort_unstable();
                sections
            };
            let expected = sections(&native);
            assert!(!expected.is_empty());
            assert_eq!(sections(&rust), expected);
        }
        native
    };
    let has_corner = |vectors: &[DrawingVector], point: [f64; 2]| {
        vectors.iter().any(|vector| {
            vector
                .preview_mm
                .iter()
                .any(|p| (p[0] - point[0]).hypot(p[1] - point[1]) < 1e-3)
        })
    };
    assert!(has_corner(&vectors(&session, &view), [1900., 2800.]));
    let plan = view.clone();
    view.id = EntityId::new();
    view.name = "Linked section".into();
    view.position_mm = [170., 100.];
    view.frame = ReferenceFrame {
        origin_mm: [0., 2300., 0.],
        u: [1., 0., 0.],
        v: [0., 0., 1.],
    };
    view.kind = ViewKind::Section {
        offset_mm: 0.,
        range: SectionRange::SectionOnly,
        window: Default::default(),
    };
    let section = vectors(&session, &view);
    assert!(has_corner(&section, [1500., -200.]));
    assert!(has_corner(&section, [1900., 0.]));
    let mut page = Page::new("A-101");
    page.views = vec![plan, view.clone()];
    let (book, prepared_book) = multicad_core::drawings::prepare_create(
        &session,
        session.revision(),
        actor(),
        "Linked shaft drawings",
        Some(page),
    )
    .unwrap();
    session.commit(prepared_book).unwrap();
    request.definition.boxes.pop();
    let change = prepared(&session, &request);
    session.commit(change.prepared).unwrap();
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let reopened = Session::new(reopened.document).unwrap();
    assert_eq!(
        read_compound_cutout(reopened.document(), request.feature).unwrap(),
        request.definition
    );
    let book = multicad_core::drawings::read(reopened.document(), book).unwrap();
    assert_eq!(book.pages[0].views.len(), 2);
    assert!(!has_corner(
        &vectors(&reopened, &book.pages[0].views[0]),
        [1900., 2800.]
    ));
    assert!(!vectors(&reopened, &book.pages[0].views[1]).is_empty());
    let mut reopened = reopened;
    let level = read_slab(
        reopened.document(),
        request.definition.receivers[0].target.producer,
    )
    .unwrap()
    .level;
    let mut definition = read_level(reopened.document(), level).unwrap();
    definition.slab_thickness = length(250.);
    let edit =
        prepare_level_edit(&reopened, reopened.revision(), actor(), level, &definition).unwrap();
    reopened.commit(edit).unwrap();
    let native = Compiler::default()
        .compile(reopened.document(), reopened.revision())
        .unwrap();
    near(
        native.solid(body).unwrap().volume_mm3().unwrap(),
        (6000. * 4000. - 1000. * 400.) * 250.,
    );
    assert_eq!(
        read_compound_cutout(reopened.document(), request.feature).unwrap(),
        request.definition,
        "Receiver height edits must rebuild the same compound definition"
    );
}
#[test]
fn overlapping_boxes_make_one_l_shaft_through_two_slabs_and_independent_step_readbacks() {
    let (mut session, mut request) = fixture();
    let change = prepared(&session, &request);
    session.commit(change.prepared).unwrap();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    assert_eq!(
        model.snapshot().body_sources.len(),
        2,
        "Cutter must not become an exported Part"
    );
    let expected = 6000. * 4000. * 200. - (1000. * 400. + 400. * 1000. - 400. * 400.) * 200.;
    for receiver in &request.definition.receivers {
        let solid = model.solid(receiver.target.body).unwrap();
        near(solid.volume_mm3().unwrap(), expected);
        assert_eq!(solid.solid_count().unwrap(), 1);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("shaft.step");
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        near(readback.volume_mm3().unwrap(), expected);
        assert_eq!(readback.solid_count().unwrap(), 1);
    }
    let rust = Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let hole_probe = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [Rational::new(20, 1).unwrap(); 3],
        PlaneFrame {
            origin: [1800., 2000., -150.],
            u: [0.8, 0.6, 0.],
            v: [-0.6, 0.8, 0.],
        },
        GeometryPolicy::default(),
    )
    .unwrap();
    assert!(
        rust.solid(request.definition.receivers[0].target.body)
            .unwrap()
            .rust_planar_intersection_metrics(&hole_probe, GeometryPolicy::default())
            .unwrap()
            .is_none()
    );
    request.definition.boxes.pop();
    let change = prepared(&session, &request);
    session.commit(change.prepared).unwrap();
    let model = Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap();
    for receiver in &request.definition.receivers {
        near(
            model
                .solid(receiver.target.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            6000. * 4000. * 200. - 1000. * 400. * 200.,
        );
    }
}
#[test]
fn disconnected_off_target_and_partial_slab_cuts_fail_native_preview_without_source_commit() {
    let (session, request) = fixture();
    let mut disconnected = request.clone();
    disconnected.definition.boxes[1].rectangle.center = [length(4000.), length(3000.)];
    let mut off_target = request.clone();
    off_target.definition.boxes[1].vertical = Some(CutoutZRange {
        bottom: length(200.),
        top: length(2800.),
    });
    let mut partial = request.clone();
    partial.definition.boxes[0].vertical = Some(CutoutZRange {
        bottom: length(-100.),
        top: length(3100.),
    });
    for (request, expected) in [
        (disconnected, "connected"),
        (off_target, "Every"),
        (partial, "thickness"),
    ] {
        let before = session.document().clone();
        let change = prepared(&session, &request);
        for backend in BACKENDS {
            let Err(error) = Compiler::default().compile_with_backend(
                change.prepared.document(),
                session.revision() + 1,
                backend,
                None,
            ) else {
                panic!("Invalid cut must fail {backend:?} evaluation");
            };
            assert!(error.to_string().contains(expected), "{backend:?}: {error}");
        }
        assert_eq!(session.document(), &before);
    }
}

#[test]
fn rotated_compound_cutouts_match_native_and_rebuild_chained_receivers() {
    let (mut session, mut request) =
        fixture_with_outline([[0., 0.], [6000., 0.], [6200., 4000.], [0., 4000.]]);
    request.definition.boxes[1].rectangle.rotation_deg = 10.;
    let first = prepared(&session, &request);
    session.commit(first.prepared).unwrap();
    let compiler = Compiler::default();
    let [native, rust] = BACKENDS.map(|backend| {
        compiler
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap_or_else(|e| panic!("{backend:?}: {e}"))
    });
    let policy = compiler.policy;
    let ground = &request.definition.receivers[0];
    let base = ground.target.producer;
    let cutter = request.feature;
    let expected = native
        .solid(base)
        .unwrap()
        .boolean(
            native.solid(cutter).unwrap(),
            BooleanOperation::Intersect,
            policy,
        )
        .unwrap()
        .unwrap();
    let (volume, bounds) = rust
        .solid(base)
        .unwrap()
        .rust_planar_intersection_metrics(rust.solid(cutter).unwrap(), policy)
        .unwrap()
        .unwrap();
    near(volume, expected.volume_mm3().unwrap());
    let expected_bounds = expected.bounds().unwrap();
    for i in 0..3 {
        near(bounds.min[i], expected_bounds.min[i]);
        near(bounds.max[i], expected_bounds.max[i]);
    }
    let pose = RigidTransform {
        rotation: [[0.8, -0.6, 0.], [0.6, 0.8, 0.], [0., 0., 1.]],
        translation_mm: [20., -40., 10.],
    };
    let transformed_base = rust.solid(base).unwrap().transformed(pose).unwrap();
    let transformed_cutter = rust.solid(cutter).unwrap().transformed(pose).unwrap();
    near(
        transformed_base
            .rust_planar_intersection_volume(&transformed_cutter, policy)
            .unwrap(),
        volume,
    );
    let topology = rust
        .solid(ground.feature)
        .unwrap()
        .topology_snapshot(ground.feature, session.revision())
        .unwrap();
    for item in &request.definition.boxes {
        let role = format!("component/{}", item.id);
        assert!(topology.elements.iter().any(|face| {
            face.key.kind == multicad_core::references::TopologyKind::Face
                && face
                    .origins
                    .iter()
                    .any(|origin| origin.source == cutter && origin.role == role)
        }));
    }
    let measure = |session: &Session, receivers: &[CompoundCutoutReceiver]| {
        let [native, rust] = BACKENDS.map(|backend| {
            compiler
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap_or_else(|e| panic!("{backend:?}: {e}"))
        });
        let temp = tempfile::tempdir().unwrap();
        receivers
            .iter()
            .enumerate()
            .map(|(i, receiver)| {
                let actual = rust.solid(receiver.target.body).unwrap();
                let expected = native.solid(receiver.target.body).unwrap();
                assert!(actual.uses_rust_geometry());
                assert_eq!(actual.solid_count().unwrap(), 1);
                let volume = actual.volume_mm3().unwrap();
                near(volume, expected.volume_mm3().unwrap());
                let area = |solid: &Solid| {
                    solid
                        .faces()
                        .unwrap()
                        .iter()
                        .map(|face| face.area_mm2)
                        .sum::<f64>()
                };
                near(area(actual), area(expected));
                let path = temp.path().join(format!("rotated-{i}.step"));
                actual.write_step(&path).unwrap();
                let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                near(readback.volume_mm3().unwrap(), volume);
                volume
            })
            .collect::<Vec<_>>()
    };
    let first = measure(&session, &request.definition.receivers);
    let second = CompoundCutoutRequest::new(
        "Second penetration",
        CompoundCutoutDefinition {
            boxes: vec![box_at([4300., 2000.], 500., 500.)],
            receivers: request
                .definition
                .receivers
                .iter()
                .map(|receiver| {
                    CompoundCutoutReceiver::new(BodyInput {
                        body: receiver.target.body,
                        producer: receiver.feature,
                    })
                })
                .collect(),
            extent: CompoundCutoutExtent::ThroughReceivers,
            levels: None,
        },
    );
    let prepared = prepared(&session, &second);
    session.commit(prepared.prepared).unwrap();
    let second = measure(&session, &second.definition.receivers);
    for (before, after) in first.into_iter().zip(second) {
        near(before - after, 500. * 500. * 200.);
    }
}
