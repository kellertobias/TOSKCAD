use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    exchange::{
        ExportReply, ExportResult, OutputSubject, RunReceipt, Scope, ScopeRoot, SetDefinition,
        prepare_set,
    },
    model::{
        self, AdvancedParameters, AdvancedRequest, AssemblyRequest, BodyInput,
        CircularPatternParameters, ComponentPlacementRequest, GearFamily, GearHand, GearHub,
        GearPairRequest, GearParameters, GearRequest, NumericTransform, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, angle, length,
    },
    references::Origin,
};
use multicad_kernel::Solid;
use multicad_model::{
    Compiler, GeometryBackend,
    export::{ExportWorker, Exporter, verify_rust_step_shape},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn bounds_match(actual: [[f64; 3]; 2], expected: [[f64; 3]; 2], tolerance: f64) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        near(actual, expected, tolerance);
    }
}

fn gear(session: &mut Session, family: GearFamily) -> GearRequest {
    let request = GearRequest::new(
        "Hubbed twisted gear",
        GearParameters {
            family,
            hand: GearHand::Right,
            helix_angle: angle(15.),
            face_width: length(4.),
            bore_diameter: length(8.),
            hub: Some(GearHub {
                diameter: length(16.),
                axial_start: length(-2.),
                axial_end: length(6.),
            }),
            frame: PrimitiveFrame {
                origin: [60., 0., 10.].map(length),
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_gear(session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    request
}

fn export_set(session: &mut Session, entities: Vec<EntityId>) -> EntityId {
    let set = EntityId::new();
    let prepared = prepare_set(
        session,
        session.revision(),
        actor(),
        set,
        "Twisted STEP",
        &SetDefinition {
            scope: Scope {
                root: ScopeRoot::Explicit { entities },
                tags: None,
            },
            ..Default::default()
        },
    )
    .unwrap();
    session.commit(prepared).unwrap();
    set
}

fn reply(worker: &ExportWorker, request: EntityId) -> ExportReply {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(reply) = worker.poll() {
            assert_eq!(reply.id, request);
            return reply;
        }
        assert!(
            Instant::now() < deadline,
            "Twisted STEP worker did not reply"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn publish(session: &Session, set: EntityId, directory: &Path) -> RunReceipt {
    let worker =
        ExportWorker::with_geometry_backend(Exporter::default(), GeometryBackend::Rust).unwrap();
    let request = worker
        .preview(session.document(), session.revision(), set, actor())
        .unwrap();
    let ExportResult::Preview(preview) = reply(&worker, request).result else {
        panic!("Expected STEP preview");
    };
    let preview = preview.unwrap();
    assert!(preview.can_run(), "{preview:?}");
    let request = worker
        .run(
            session.document(),
            session.revision(),
            &preview.key,
            directory,
            actor(),
        )
        .unwrap();
    let ExportResult::Run(run) = reply(&worker, request).result else {
        panic!("Expected verified STEP publication");
    };
    run.unwrap()
}

fn verify_compound(solid: &Solid, source: &str, count: u32) {
    let (actual_count, volume, bounds) = solid
        .verify_twisted_step_compound(source, 1e-7)
        .expect("Expected compound twisted verification")
        .unwrap();
    assert_eq!(actual_count, count);
    near(volume, solid.volume_mm3().unwrap(), 1e-5);
    let expected = solid.bounds().unwrap();
    bounds_match(bounds, [expected.min, expected.max], 1e-6);
}

#[cfg(feature = "occt")]
fn native_readback(path: &Path, expected: &Solid) {
    let native = Solid::read_step(path, Default::default()).unwrap();
    assert_eq!(
        native.solid_count().unwrap(),
        expected.solid_count().unwrap()
    );
    near(
        native.volume_mm3().unwrap(),
        expected.volume_mm3().unwrap(),
        5e-4,
    );
    let bounds = native.bounds().unwrap();
    let expected = expected.bounds().unwrap();
    bounds_match([bounds.min, bounds.max], [expected.min, expected.max], 2e-5);
}

#[test]
fn patterned_hubbed_twisted_parts_publish_one_verified_multisolid_step() {
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    for family in [GearFamily::Helical, GearFamily::Herringbone] {
        let mut session =
            Session::new(Document::new("Placed twisted gear pattern").unwrap()).unwrap();
        let request = gear(&mut session, family);
        let pattern = AdvancedRequest::on_body(
            BodyInput {
                body: request.body,
                producer: request.feature,
            },
            AdvancedParameters::CircularPattern(CircularPatternParameters {
                count: Dimension::Literal {
                    value: 2.,
                    unit: "count".into(),
                },
                step_angle: angle(90.),
                merge: false,
                ..Default::default()
            }),
            vec![],
        );
        let prepared =
            model::prepare_advanced(&session, session.revision(), actor(), &pattern).unwrap();
        session.commit(prepared.prepared).unwrap();
        let set = export_set(&mut session, vec![pattern.body]);
        let model = compiler
            .compile_with_backend(
                session.document(),
                session.revision(),
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        let solid = model.solid(pattern.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 2);
        assert!(solid.supports_step_export());
        let shape = &model.snapshot().shapes[&pattern.feature];
        for member in 0..2 {
            assert!(
                shape
                    .topology
                    .elements
                    .iter()
                    .any(|element| element.origins.contains(&Origin {
                        source: pattern.feature,
                        role: format!("member/{member}"),
                    }))
            );
        }
        let directory = tempfile::tempdir().unwrap();
        let run = publish(&session, set, directory.path());
        let [file] = run.files.as_slice() else {
            panic!("Expected one compound Part artifact");
        };
        assert_eq!(file.artifact.producer, pattern.feature);
        assert_eq!(file.artifact.solid_count, 2);
        let OutputSubject::Part(subject) = &file.artifact.output.subject else {
            panic!("Expected Part attribution");
        };
        assert_eq!(subject.part, pattern.body);
        assert!(subject.occurrence_path.is_empty());
        let path = Path::new(&run.directory).join(&file.artifact.output.filename);
        let source = std::fs::read_to_string(&path).unwrap();
        verify_compound(solid, &source, 2);
        near(
            verify_rust_step_shape(&path, solid, shape, compiler.policy.linear_tolerance_mm)
                .unwrap(),
            solid.volume_mm3().unwrap(),
            1e-5,
        );
        #[cfg(feature = "occt")]
        native_readback(&path, solid);

        // Changing the handedness preserves volume and the cardinal
        // compound bounds but changes both actual helicoid boundaries.
        let mut changed = request.parameters.clone();
        changed.hand = GearHand::Left;
        let prepared = model::prepare_gear_edit(
            &session,
            session.revision(),
            actor(),
            request.feature,
            &changed,
        )
        .unwrap();
        session.commit(prepared.prepared).unwrap();
        let other = compiler
            .compile_with_backend(
                session.document(),
                session.revision(),
                GeometryBackend::Rust,
                None,
            )
            .unwrap();
        let other_solid = other.solid(pattern.feature).unwrap();
        near(
            other_solid.volume_mm3().unwrap(),
            solid.volume_mm3().unwrap(),
            1e-8,
        );
        let before = solid.bounds().unwrap();
        let after = other_solid.bounds().unwrap();
        bounds_match([after.min, after.max], [before.min, before.max], 1e-8);
        let wrong_hand = directory.path().join("wrong-hand.step");
        other_solid.write_step(&wrong_hand).unwrap();
        let bad_source = std::fs::read_to_string(&wrong_hand).unwrap();
        assert!(
            solid
                .verify_twisted_step_compound(&bad_source, 1e-7)
                .unwrap()
                .is_err()
        );
        assert!(
            verify_rust_step_shape(
                &wrong_hand,
                solid,
                shape,
                compiler.policy.linear_tolerance_mm
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn mixed_assembly_exports_exact_placed_parts_and_verifies_its_combined_step() {
    let mut session = Session::new(Document::new("Mixed twisted assembly").unwrap()).unwrap();
    let request = gear(&mut session, GearFamily::Herringbone);
    let stock = PrimitiveRequest::new(
        "Stock",
        PrimitiveParameters::Box {
            size: [3., 5., 7.].map(length),
            frame: PrimitiveFrame {
                origin: [0., -40., 0.].map(length),
                ..Default::default()
            },
        },
    );
    let prepared = model::prepare_primitive(&session, session.revision(), actor(), &stock).unwrap();
    session.commit(prepared.prepared).unwrap();
    let members = [request.body, stock.body].map(|body| {
        let placement = ComponentPlacementRequest::new(body, NumericTransform::default());
        let prepared =
            model::prepare_place_component(&session, session.revision(), actor(), &placement)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        placement.instance
    });
    let assembly = AssemblyRequest::new(
        "Gear and stock",
        members.to_vec(),
        vec![request.body, stock.body],
    );
    let prepared =
        model::prepare_assembly(&session, session.revision(), actor(), &assembly).unwrap();
    session.commit(prepared.prepared).unwrap();
    let root = ComponentPlacementRequest::new(
        assembly.assembly,
        NumericTransform {
            translation: [-12., 8., 4.].map(length),
            axis: [1., 0., 0.],
            angle: angle(90.),
            ..Default::default()
        },
    );
    let prepared =
        model::prepare_place_component(&session, session.revision(), actor(), &root).unwrap();
    session.commit(prepared.prepared).unwrap();
    let set = export_set(&mut session, vec![root.instance]);
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let model = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let aggregate = model.solid(root.instance).unwrap();
    assert_eq!(aggregate.solid_count().unwrap(), 2);
    assert!(aggregate.supports_step_export());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mixed-assembly.step");
    aggregate.write_step(&path).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    verify_compound(aggregate, &source, 2);
    #[cfg(feature = "occt")]
    native_readback(&path, aggregate);

    let run = publish(&session, set, directory.path());
    assert_eq!(run.files.len(), 2);
    for (index, body, producer) in [
        (0, request.body, request.feature),
        (1, stock.body, stock.feature),
    ] {
        let file = run
            .files
            .iter()
            .find(|file| file.artifact.producer == producer)
            .unwrap();
        assert_eq!(file.artifact.solid, root.instance);
        assert_eq!(file.artifact.solid_count, 1);
        let OutputSubject::Part(subject) = &file.artifact.output.subject else {
            panic!("Expected placed Part attribution");
        };
        assert_eq!(subject.part, body);
        assert_eq!(subject.occurrence_path, vec![root.instance, members[index]]);
        let leaf = model
            .leaf_solid(&model::OccurrencePath {
                root: root.instance,
                members: vec![members[index]],
            })
            .unwrap();
        near(file.artifact.volume_mm3, leaf.volume_mm3().unwrap(), 1e-6);
        let bounds = leaf.bounds().unwrap();
        bounds_match(file.artifact.bounds_mm, [bounds.min, bounds.max], 1e-7);
        #[cfg(feature = "occt")]
        native_readback(
            &Path::new(&run.directory).join(&file.artifact.output.filename),
            leaf,
        );
    }
    assert_eq!(
        run.files
            .iter()
            .filter(|file| file.artifact.producer == request.feature)
            .count(),
        1
    );
}

#[test]
fn placed_twenty_four_thirty_six_pair_keeps_each_authored_boundary_in_compound_step() {
    let mut session = Session::new(Document::new("Placed hubbed pair STEP").unwrap()).unwrap();
    let gears = [(24., GearHand::Right), (36., GearHand::Left)].map(|(teeth, hand)| {
        GearRequest::new(
            format!("{teeth} tooth gear"),
            GearParameters {
                family: GearFamily::Herringbone,
                hand,
                teeth: Dimension::Literal {
                    value: teeth,
                    unit: "count".into(),
                },
                helix_angle: angle(15.),
                face_width: length(4.),
                bore_diameter: length(8.),
                hub: Some(GearHub {
                    diameter: length(16.),
                    axial_start: length(-2.),
                    axial_end: length(6.),
                }),
                ..Default::default()
            },
        )
    });
    let mut pair = GearPairRequest::new("Hubbed continuous pair", gears);
    pair.parameters.frame = PrimitiveFrame {
        origin: [12., 23., 34.].map(length),
        u: [0., 1., 0.],
        v: [0., 0., 1.],
    };
    let prepared = model::prepare_gear_pair(&session, 0, actor(), &pair).unwrap();
    session.commit(prepared.prepared).unwrap();
    let set = export_set(&mut session, vec![pair.placement.instance]);
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let model = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert!(model.gear_pair_checks()[&pair.pair].current_pose_checked);
    let solid = model.solid(pair.placement.instance).unwrap();
    assert_eq!(solid.solid_count().unwrap(), 2);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("placed-pair.step");
    solid.write_step(&path).unwrap();
    verify_compound(solid, &std::fs::read_to_string(&path).unwrap(), 2);
    #[cfg(feature = "occt")]
    native_readback(&path, solid);
    let run = publish(&session, set, directory.path());
    assert_eq!(run.files.len(), 2);
    for (gear, instance) in pair.gears.iter().zip(pair.outputs.instances) {
        let file = run
            .files
            .iter()
            .find(|file| file.artifact.producer == gear.feature)
            .unwrap();
        let OutputSubject::Part(subject) = &file.artifact.output.subject else {
            panic!("Expected pair member attribution");
        };
        assert_eq!(subject.part, gear.body);
        assert_eq!(
            subject.occurrence_path,
            vec![pair.placement.instance, instance]
        );
        near(
            file.artifact.volume_mm3,
            model.solid(instance).unwrap().volume_mm3().unwrap(),
            1e-6,
        );
    }
}
