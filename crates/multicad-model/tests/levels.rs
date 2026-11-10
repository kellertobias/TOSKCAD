//! Levels host slabs, pedestals, stairs and walls; level and stair edits
//! rebuild them; service envelopes check clearance against model solids.
use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    drawings::*,
    model::{
        self, BodyOutput, EnvelopeDefinition, Justification, LevelDefinition, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, SlabDefinition, SlabKind, StairDefinition,
        StairForm, Turn, WallDefinition, WallRequest, WallTypeDefinition, length,
    },
    project_file::{self, ProjectFile},
    references::ReferenceFrame,
};
use multicad_model::{
    Compiler, DrawingProviders, GeometryBackend, Model, export::verify_rust_step_shape,
};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn near(a: f64, b: f64, t: f64) {
    assert!((a - b).abs() <= t, "{a} != {b} (±{t})");
}
fn compile(s: &Session) -> Model {
    Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap()
}
fn pt(x: f64, y: f64) -> [Dimension; 2] {
    [length(x), length(y)]
}
fn level(s: &mut Session, name: &str, elevation: f64) -> EntityId {
    let (id, p) = model::prepare_level(
        s,
        s.revision(),
        actor(),
        name,
        &LevelDefinition {
            elevation: length(elevation),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    id
}
fn commit(s: &mut Session, prepare: impl FnOnce(&Session) -> model::ModelPrepared) -> EntityId {
    let p = prepare(s);
    Compiler::default()
        .compile(p.prepared.document(), s.revision())
        .unwrap();
    let f = p.outputs[0].feature;
    s.commit(p.prepared).unwrap();
    f
}
fn stair(level: EntityId, top: EntityId, form: StairForm) -> StairDefinition {
    StairDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level,
        to_level: Some(top),
        form,
        start: pt(500., 500.),
        direction_deg: 0.,
        width: length(1000.),
        risers: 16,
        going: length(280.),
    }
}
/// Area of one flight profile: columns of going width rising by one riser,
/// the last column `top` deep, all lifted by `lift`.
fn flight_area(n: f64, riser: f64, going: f64, lift: f64, top: f64) -> f64 {
    going * ((n - 1.) * lift + riser * (n - 1.) * n / 2.) + top * (lift + n * riser)
}

#[test]
fn rust_stairs_preserve_rotated_flights_landings_and_manufacturing_geometry() {
    let mut session = Session::new(Document::new("Rotated stairs").unwrap()).unwrap();
    let bottom = level(&mut session, "Ground", 100.);
    let top = level(&mut session, "Upper", 3100.);
    let mut features = Vec::new();
    for (form, direction_deg) in [
        (StairForm::Straight, 20.),
        (StairForm::QuarterTurn { turn: Turn::Left }, 27.),
        (StairForm::HalfTurn { turn: Turn::Right }, -31.),
    ] {
        let definition = StairDefinition {
            direction_deg,
            ..stair(bottom, top, form)
        };
        let prepared = model::prepare_stair(
            &session,
            session.revision(),
            actor(),
            "Stair",
            &definition,
            None,
        )
        .unwrap();
        features.push(prepared.outputs[0].feature);
        session.commit(prepared.prepared).unwrap();
    }
    let compiler = Compiler::default();
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_sources_with_backend(
                session.document(),
                session.revision(),
                &features,
                backend,
                None,
            )
            .unwrap_or_else(|cause| panic!("{backend:?}: {cause}"))
    });
    let scratch = tempfile::tempdir().unwrap();
    for feature in features {
        let [actual, expected] = [&rust, &native].map(|m| m.solid(feature).unwrap());
        assert!(actual.uses_rust_geometry());
        assert_eq!(actual.solid_count().unwrap(), 1);
        let volume = expected.volume_mm3().unwrap();
        near(actual.volume_mm3().unwrap(), volume, 1e-8 * volume);
        let [a, b] = [actual, expected].map(|solid| solid.bounds().unwrap());
        for i in 0..3 {
            near(a.min[i], b.min[i], 1e-7);
            near(a.max[i], b.max[i], 1e-7);
        }
        assert!(
            !actual
                .manufacturing_mesh_with_receipt(Default::default())
                .unwrap()
                .mesh
                .indices
                .is_empty()
        );
        let path = scratch.path().join(format!("stair-{feature}.step"));
        actual.write_step(&path).unwrap();
        let reopened = multicad_kernel::Solid::read_step(&path, compiler.policy).unwrap();
        near(reopened.volume_mm3().unwrap(), volume, 1e-8 * volume);
        let receipt = verify_rust_step_shape(
            &path,
            &rust.snapshot().shapes[&feature],
            compiler.policy.linear_tolerance_mm,
        )
        .unwrap();
        near(receipt, volume, 1e-8 * volume);
    }
}

#[test]
fn levels_host_slabs_pedestals_stairs_and_walls_that_follow_level_edits() {
    let mut s = Session::new(Document::new("Levels").unwrap()).unwrap();
    let ground = level(&mut s, "Ground", 0.);
    let upper = level(&mut s, "Upper", 3000.);
    let slab = commit(&mut s, |s| {
        model::prepare_slab(
            s,
            s.revision(),
            actor(),
            "Ground slab",
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level: ground,
                kind: SlabKind::Slab,
                outline: vec![pt(0., 0.), pt(6000., 0.), pt(6000., 4000.), pt(0., 4000.)],
                thickness: None,
                holes: vec![],
            },
            None,
        )
        .unwrap()
    });
    let pedestal = commit(&mut s, |s| {
        model::prepare_slab(
            s,
            s.revision(),
            actor(),
            "Pedestal",
            &SlabDefinition {
                output: BodyOutput {
                    body: EntityId::new(),
                },
                level: ground,
                kind: SlabKind::Pedestal,
                outline: vec![
                    pt(4500., 2500.),
                    pt(5500., 2500.),
                    pt(5500., 3300.),
                    pt(4500., 3300.),
                ],
                thickness: Some(length(300.)),
                holes: vec![],
            },
            None,
        )
        .unwrap()
    });
    let straight = commit(&mut s, |s| {
        model::prepare_stair(
            s,
            s.revision(),
            actor(),
            "Stair",
            &stair(ground, upper, StairForm::Straight),
            None,
        )
        .unwrap()
    });
    let quarter_def = StairDefinition {
        start: pt(500., 6000.),
        ..stair(ground, upper, StairForm::QuarterTurn { turn: Turn::Left })
    };
    let quarter = commit(&mut s, |s| {
        model::prepare_stair(s, s.revision(), actor(), "Quarter", &quarter_def, None).unwrap()
    });
    let half_def = StairDefinition {
        start: pt(500., 10000.),
        ..stair(ground, upper, StairForm::HalfTurn { turn: Turn::Right })
    };
    let half = commit(&mut s, |s| {
        model::prepare_stair(s, s.revision(), actor(), "Half", &half_def, None).unwrap()
    });
    let (wall_type, p) = model::prepare_wall_type(
        &s,
        s.revision(),
        actor(),
        "Wall",
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(2800.),
            justification: Justification::Center,
            material: None,
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let upper_wall = commit(&mut s, |s| {
        model::prepare_wall(
            s,
            s.revision(),
            actor(),
            &WallRequest::new(
                "Upper wall",
                WallDefinition {
                    layer_placement: None,
                    output: BodyOutput {
                        body: EntityId::new(),
                    },
                    wall_type,
                    path: vec![pt(0., -2000.), pt(4000., -2000.)],
                    closed: false,
                    level: Some(upper),
                    base_z: length(0.),
                    thickness: None,
                    height: None,
                    justification: None,
                    openings: vec![],
                    cutouts: vec![],
                },
            ),
        )
        .unwrap()
    });

    let m = compile(&s);
    let shape = |f: EntityId| m.snapshot().shapes[&f].clone();
    near(shape(slab).volume_mm3, 6000. * 4000. * 200., 1e-3);
    near(shape(slab).bounds.max[2], 0., 1e-9);
    near(shape(slab).bounds.min[2], -200., 1e-9);
    near(shape(pedestal).bounds.max[2], 300., 1e-9);
    // Straight: 16 risers of 187.5 mm, 280 mm goings, 1 m wide.
    let r = 3000. / 16.;
    near(
        shape(straight).volume_mm3,
        1000. * flight_area(16., r, 280., 0., 280.),
        1e-2,
    );
    near(shape(straight).bounds.min[0], 500., 1e-9);
    near(shape(straight).bounds.max[0], 500. + 16. * 280., 1e-9);
    near(shape(straight).bounds.max[2], 3000., 1e-9);
    // Turning stairs: one connected solid with a landing at 8 risers.
    let landing = 8. * r;
    let turning = flight_area(8., r, 280., 0., 1000.) + flight_area(8., r, 280., landing, 280.);
    for (f, extra) in [(quarter, 0.), (half, 1000. * 1000. * landing)] {
        assert_eq!(shape(f).solid_count, 1);
        near(shape(f).volume_mm3, 1000. * turning + extra, 1e-2);
        near(shape(f).bounds.max[2], 3000., 1e-9);
    }
    near(shape(upper_wall).bounds.min[2], 3000., 1e-9);
    let check = model::resolve_stair(
        s.document(),
        s.parameters(),
        &model::read_stair(s.document(), straight).unwrap(),
    )
    .unwrap();
    near(check.check.step_measure_mm, 2. * r + 280., 1e-9);
    assert!(check.check.notes.is_empty());

    // Raising the upper level rebuilds the stairs and moves its wall.
    let before = s.document().clone();
    let p = model::prepare_level_edit(
        &s,
        s.revision(),
        actor(),
        upper,
        &LevelDefinition {
            elevation: length(3300.),
            height: length(3000.),
            slab_thickness: length(200.),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let m = compile(&s);
    let r2 = 3300. / 16.;
    near(
        m.snapshot().shapes[&straight].volume_mm3,
        1000. * flight_area(16., r2, 280., 0., 280.),
        1e-2,
    );
    near(m.snapshot().shapes[&upper_wall].bounds.min[2], 3300., 1e-9);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);

    // Stair edits rebuild; geometric notes are reported, not enforced.
    let mut steep = model::read_stair(s.document(), straight).unwrap();
    steep.going = length(150.);
    let p = model::prepare_stair_edit(&s, s.revision(), actor(), straight, &steep).unwrap();
    let notes = model::resolve_stair(p.prepared.document(), p.prepared.parameters(), &steep)
        .unwrap()
        .check
        .notes;
    assert!(
        notes.iter().any(|n| n.contains("geometric note only")),
        "{notes:?}"
    );
    // Invalid stairs and slabs are refused with a reason.
    let mut down = model::read_stair(s.document(), straight).unwrap();
    down.level = upper;
    down.to_level = Some(ground);
    let msg = model::prepare_stair_edit(&s, s.revision(), actor(), straight, &down)
        .err()
        .unwrap()
        .to_string();
    assert!(msg.contains("above its base level"), "{msg}");
    let bow = SlabDefinition {
        output: BodyOutput {
            body: EntityId::new(),
        },
        level: ground,
        kind: SlabKind::Slab,
        outline: vec![pt(0., 0.), pt(2000., 1000.), pt(2000., 0.), pt(0., 2000.)],
        thickness: None,
        holes: vec![],
    };
    let msg = model::prepare_slab(&s, s.revision(), actor(), "Bow", &bow, None)
        .err()
        .unwrap()
        .to_string();
    assert!(msg.contains("self-intersects"), "{msg}");

    // Save/reopen keeps every level-hosted element.
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let opened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    for cap in [
        model::LEVEL_CAPABILITY,
        model::SLAB_CAPABILITY,
        model::STAIR_CAPABILITY,
    ] {
        assert!(opened.capabilities().contains(cap));
    }
    assert_eq!(&opened.document, s.document());
    let reopened = Compiler::default().compile(&opened.document, 0).unwrap();
    near(
        reopened.snapshot().shapes[&half].volume_mm3,
        compile(&s).snapshot().shapes[&half].volume_mm3,
        1e-6,
    );
}

#[test]
fn service_envelopes_report_model_clearance_and_show_in_plan() {
    for backend in [GeometryBackend::OpenCascade, GeometryBackend::Rust] {
        service_envelope_workflow(backend);
    }
}

fn service_envelope_workflow(backend: GeometryBackend) {
    let compile = |session: &Session| {
        Compiler::default()
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap()
    };
    let mut s = Session::new(Document::new("Envelopes").unwrap()).unwrap();
    let machine = PrimitiveRequest::new(
        "Machine",
        PrimitiveParameters::Box {
            size: [length(1000.), length(800.), length(1500.)],
            frame: PrimitiveFrame {
                origin: [length(2000.), length(2000.), length(0.)],
                ..Default::default()
            },
        },
    );
    s.commit(
        model::prepare_primitive(&s, 0, actor(), &machine)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let (wall_type, p) = model::prepare_wall_type(
        &s,
        s.revision(),
        actor(),
        "Wall",
        &WallTypeDefinition {
            thickness: length(200.),
            height: length(2500.),
            justification: Justification::Center,
            material: None,
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let wall = WallRequest::new(
        "Front wall",
        WallDefinition {
            layer_placement: None,
            output: BodyOutput {
                body: EntityId::new(),
            },
            wall_type,
            path: vec![pt(1000., 1500.), pt(4000., 1500.)],
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
    let wall_part = wall.body;
    s.commit(
        model::prepare_wall(&s, s.revision(), actor(), &wall)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let (envelope, p) = model::prepare_envelope(
        &s,
        s.revision(),
        actor(),
        "Machine service",
        &EnvelopeDefinition {
            target: machine.body,
            front: length(600.),
            back: length(200.),
            left: length(300.),
            right: length(300.),
            top: length(0.),
            label: "Operator access".into(),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    let m = compile(&s);
    let evaluations = m.envelopes(s.document()).unwrap();
    let env = &evaluations[0];
    assert_eq!(env.envelope, envelope);
    near(env.corners[0][0], 1700., 1e-6);
    near(env.corners[0][1], 1400., 1e-6);
    near(env.corners[6][0], 3300., 1e-6);
    near(env.corners[6][1], 3000., 1e-6);
    near(env.corners[6][2], 1500., 1e-6);
    assert_eq!(m.snapshot().envelope_boxes[&env.envelope], env.corners);
    near(env.volume_mm3, 1600. * 1600. * 1500., 1e-3);
    // The wall face occupies y 1400..1600 across the envelope's 1600 mm width.
    assert_eq!(env.conflicts.len(), 1);
    assert_eq!(env.conflicts[0].0, wall_part);
    near(env.conflicts[0].1, 1600. * 200. * 1500., 1e-3);

    // Narrowing the front clearance clears the conflict.
    let p = model::prepare_envelope_edit(
        &s,
        s.revision(),
        actor(),
        envelope,
        &EnvelopeDefinition {
            target: machine.body,
            front: length(350.),
            back: length(200.),
            left: length(300.),
            right: length(300.),
            top: length(0.),
            label: "Operator access".into(),
        },
    )
    .unwrap();
    s.commit(p).unwrap();
    assert!(
        compile(&s).envelopes(s.document()).unwrap()[0]
            .conflicts
            .is_empty()
    );

    // A plan view of the machine shows its envelope as dashed outlines.
    let mut page = Page::new("E-1");
    page.settings.paper = PaperSize::A3;
    let view = DrawingView {
        id: EntityId::new(),
        name: "Plan".into(),
        source: ViewSource::Model {
            entities: vec![machine.body],
        },
        frame: ReferenceFrame {
            origin_mm: [0.; 3],
            u: [1., 0., 0.],
            v: [0., 1., 0.],
        },
        kind: ViewKind::Projection,
        position_mm: [100., 200.],
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
    let vid = view.id;
    page.views.push(view);
    let (_, p) = prepare_create(&s, s.revision(), actor(), "Plans", Some(page)).unwrap();
    s.commit(p).unwrap();
    let evaluation = compile(&s)
        .drawings(s.document(), s.revision(), &DrawingProviders::default())
        .unwrap();
    let ViewEvaluation::Ready { vectors, .. } = &evaluation.views[&vid] else {
        panic!()
    };
    let dashed: Vec<_> = vectors
        .iter()
        .filter(|v| v.role == VectorRole::Hidden)
        .collect();
    assert_eq!(dashed.len(), 4);
    assert!(dashed.iter().any(|v| matches!(v.curve, VectorCurve::Line { a, b } if (a[1] - 1650.).abs() < 1e-6 && (b[1] - 1650.).abs() < 1e-6)));
}
