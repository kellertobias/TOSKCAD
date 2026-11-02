use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;
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
                outline: [[0., 0.], [6000., 0.], [6000., 4000.], [0., 4000.]]
                    .map(|p| p.map(length))
                    .to_vec(),
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
        let prepared = prepared(&session, &request);
        session.commit(prepared.prepared).unwrap();
        let native = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        near(
            native
                .solid(wall.target.body)
                .unwrap()
                .volume_mm3()
                .unwrap(),
            4000. * 200. * 2500. - if walls { 1000. * 50. * 2500. } else { 0. },
        );
        for floor in floors {
            near(
                native
                    .solid(floor.target.body)
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
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
    let result = Compiler::default().compile_sources(
        prepared.prepared.document(),
        session.revision() + 1,
        &[request.definition.receivers[0].feature],
    );
    let Err(error) = result else {
        panic!("Every receiving solid must be proved")
    };
    assert!(error.to_string().contains("split"));
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
        let native = Compiler::default()
            .compile(session.document(), session.revision())
            .unwrap();
        let ViewEvaluation::Ready { vectors, .. } = native
            .drawing_view(
                session.document(),
                session.revision(),
                view,
                &Default::default(),
            )
            .unwrap()
        else {
            panic!("Linked native drawing must resolve")
        };
        vectors
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
        let Err(error) =
            Compiler::default().compile(change.prepared.document(), session.revision() + 1)
        else {
            panic!("Invalid cut must fail native evaluation")
        };
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(session.document(), &before);
    }
}
