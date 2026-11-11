use multicad_core::{
    commands::{Actor, Channel, Session},
    constraint_sketch::Dimension,
    document::{Document, EntityId},
    model::{
        self, BevelDimensions, BevelToothSystem, GearFamily, GearPairRequest, GearParameters,
        GearRequest, PrimitiveFrame, angle, length,
    },
    project_file::{self, FileLimits, ProjectFile},
    references::TopologyKind,
};
use multicad_kernel::{
    BooleanOperation, GeometryErrorKind, RigidTransform, Solid,
    rust_kernel::bevel_gear::clearance::ResultKind,
};
use multicad_model::{Compiler, GeometryBackend};
use std::f64::consts::PI;

fn actor() -> Actor {
    Actor::local(Channel::Ui)
}

fn count(value: u32) -> Dimension {
    Dimension::Literal {
        value: f64::from(value),
        unit: "count".into(),
    }
}

fn member(teeth: u32, mate: u32, width: f64) -> GearParameters {
    GearParameters {
        family: GearFamily::Bevel,
        teeth: count(teeth),
        face_width: length(width),
        bevel: Some(BevelDimensions {
            tooth_system: BevelToothSystem::SphericalInvoluteStraight,
            mate_teeth: count(mate),
            shaft_angle: angle(90.),
        }),
        ..Default::default()
    }
}

fn roles(solid: &Solid, owner: EntityId) -> std::collections::BTreeSet<String> {
    solid
        .topology_snapshot(owner, 0)
        .unwrap()
        .elements
        .into_iter()
        .filter(|e| e.key.kind == TopologyKind::Face)
        .flat_map(|e| e.origins.into_iter().map(|o| o.role))
        .collect()
}

#[test]
fn rust_bevel_pairs_match_native_members_and_certify_saved_edited_motion() {
    let mut session = Session::new(Document::new("Rust bevel pair").unwrap()).unwrap();
    let mut request = GearPairRequest::new(
        "Bevel pair",
        [24, 32].map(|z| GearRequest::new("Bevel", member(z, 56 - z, 6.))),
    );
    request.parameters.frame = PrimitiveFrame {
        origin: [length(10.), length(-5.), length(20.)],
        u: [0., 1., 0.],
        v: [0.8, 0., 0.6],
    };
    let draft = model::prepare_gear_pair(&session, 0, actor(), &request).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Compiler::default()
    };
    let document = draft.prepared.document();
    let rust = compiler
        .compile_with_backend(document, 1, GeometryBackend::Rust, None)
        .unwrap();
    let report = &rust.gear_pair_checks()[&request.pair];
    assert_eq!(report.cycle_drive_angles_rad.len(), 9);
    assert!(report.current_pose_checked && report.bevel.is_some());
    let witness = report.current_engagement.as_ref().unwrap();
    assert!(
        witness.distance_mm > 0. && witness.distance_mm <= 0.1 + 1e-5,
        "{witness:?}"
    );
    let [a, b] = request.outputs.instances;
    let feature = request.parameters.gears[0];
    for instance in [a, b] {
        let solid = rust.solid(instance).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 1);
        solid
            .topology_snapshot(instance, 1)
            .unwrap()
            .validate()
            .unwrap();
    }
    assert!(roles(rust.solid(a).unwrap(), a).contains("bevel/tooth/0/spherical_involute.left"));

    // A half-pitch error puts teeth into teeth; a large offset loses mesh.
    let half = PI / 32.;
    let spin = |angle: f64, solid: &Solid| {
        // Rotate the wheel about the pinion pair frame's u axis, which is
        // the wheel axis for a 90° shaft angle, around the shared apex.
        let [x, y, z] = [0., 1., 0.];
        let (s, c) = angle.sin_cos();
        let rotation = [
            [
                c + x * x * (1. - c),
                x * y * (1. - c) - z * s,
                x * z * (1. - c) + y * s,
            ],
            [
                y * x * (1. - c) + z * s,
                c + y * y * (1. - c),
                y * z * (1. - c) - x * s,
            ],
            [
                z * x * (1. - c) - y * s,
                z * y * (1. - c) + x * s,
                c + z * z * (1. - c),
            ],
        ];
        let apex = [10., -5., 20.];
        let moved: [f64; 3] = std::array::from_fn(|i| {
            apex[i] - (0..3).map(|j| rotation[i][j] * apex[j]).sum::<f64>()
        });
        solid
            .transformed(RigidTransform {
                rotation,
                translation_mm: moved,
            })
            .unwrap()
    };
    let wrong = spin(half, rust.solid(b).unwrap());
    let (result, _) = rust
        .solid(a)
        .unwrap()
        .certify_rust_bevel_pair(&wrong, compiler.policy, 0.2, None)
        .unwrap();
    assert_eq!(result, ResultKind::CommonInterior);
    let far = rust
        .solid(b)
        .unwrap()
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [1000., 0., 0.],
        })
        .unwrap();
    let separated = rust
        .solid(a)
        .unwrap()
        .certify_rust_bevel_pair(&far, compiler.policy, 0.2, None)
        .unwrap();
    assert_eq!(separated, (ResultKind::Clear, None));
    let near = rust
        .solid(b)
        .unwrap()
        .transformed(RigidTransform {
            rotation: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            translation_mm: [0.5, 0., 0.],
        })
        .unwrap();
    let error = rust
        .solid(a)
        .unwrap()
        .certify_rust_bevel_pair(&near, compiler.policy, 0.2, None)
        .unwrap_err();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);

    if multicad_kernel::OCCT_LINKED {
        let native = compiler
            .compile_with_backend(document, 1, GeometryBackend::OpenCascade, None)
            .unwrap();
        let oracle = &native.gear_pair_checks()[&request.pair];
        assert_eq!(oracle.cycle_drive_angles_rad, report.cycle_drive_angles_rad);
        for instance in [a, b] {
            let [r, n] = [&rust, &native].map(|c| c.solid(instance).unwrap().volume_mm3().unwrap());
            assert!((r - n).abs() / n < 1e-7, "{r} {n}");
            let [rb, nb] = [&rust, &native].map(|c| c.solid(instance).unwrap().bounds().unwrap());
            for i in 0..3 {
                assert!(
                    (rb.min[i] - nb.min[i]).abs() < 2e-3 && (rb.max[i] - nb.max[i]).abs() < 2e-3
                );
            }
            assert_eq!(
                roles(rust.solid(instance).unwrap(), instance),
                roles(native.solid(instance).unwrap(), instance)
            );
        }
        let wrong_native = spin(half, native.solid(b).unwrap());
        let overlap = native
            .solid(a)
            .unwrap()
            .boolean(&wrong_native, BooleanOperation::Intersect, compiler.policy)
            .unwrap()
            .unwrap()
            .volume_mm3()
            .unwrap();
        assert!(overlap > 1., "{overlap}");
    }

    session.commit(draft.prepared).unwrap();
    // Edit drive and both members' cone face width in one checked transaction.
    let mut parameters = request.parameters.clone();
    parameters.drive_angle = angle(5.);
    let gears = [24, 32].map(|z| member(z, 56 - z, 5.));
    let edited = model::prepare_gear_pair_geometry_edit(
        &session,
        session.revision(),
        actor(),
        request.pair,
        &parameters,
        &gears,
    )
    .unwrap();
    session.commit(edited.prepared).unwrap();
    let after = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let moved = &after.gear_pair_checks()[&request.pair];
    assert!((moved.current_drive_angle_rad - 5_f64.to_radians()).abs() < 1e-12);
    let narrower = after.solid(a).unwrap().volume_mm3().unwrap();
    let wider = rust.solid(a).unwrap().volume_mm3().unwrap();
    assert!(narrower < wider);
    assert_eq!(
        roles(after.solid(a).unwrap(), a),
        roles(rust.solid(a).unwrap(), a)
    );

    // Undo restores the previous recipe and its proof; redo restores the edit.
    session.undo(session.revision(), actor()).unwrap();
    let undone = compiler
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    assert!((undone.solid(a).unwrap().volume_mm3().unwrap() - wider).abs() < 1e-9);
    session.redo(session.revision(), actor()).unwrap();

    // An edit Rust cannot certify is refused before commit.
    let revision = session.revision();
    let mut zero = gears.clone();
    for gear in &mut zero {
        gear.backlash = length(0.);
    }
    let candidate = model::prepare_gear_pair_geometry_edit(
        &session,
        revision,
        actor(),
        request.pair,
        &parameters,
        &zero,
    )
    .unwrap();
    let Err(failure) = compiler.compile_with_backend(
        candidate.prepared.document(),
        revision,
        GeometryBackend::Rust,
        None,
    ) else {
        panic!("zero-backlash bevel pair must not be certified");
    };
    assert!(
        failure.to_string().contains("certified positive clearance"),
        "{failure}"
    );
    assert_eq!(session.revision(), revision);

    let mut bytes = Vec::new();
    project_file::write(
        &ProjectFile::new(session.document().clone()).unwrap(),
        &mut bytes,
        FileLimits::default(),
    )
    .unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    let readback = compiler
        .compile_with_backend(&reopened.project.document, 0, GeometryBackend::Rust, None)
        .unwrap();
    let proof = &readback.gear_pair_checks()[&request.pair];
    assert_eq!(proof.cycle_drive_angles_rad, moved.cycle_drive_angles_rad);
    assert!((readback.solid(a).unwrap().volume_mm3().unwrap() - narrower).abs() < 1e-9);
    assert_eq!(
        readback.solid(feature).is_some(),
        after.solid(feature).is_some()
    );

    // The exported Rust STEP of the placed member reads back as one solid.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bevel-member.step");
    readback.solid(a).unwrap().write_step(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let decoded = readback
        .solid(a)
        .unwrap()
        .verify_bevel_step(&text)
        .unwrap()
        .unwrap();
    assert!(
        (decoded - narrower).abs() / narrower < 1e-8,
        "{decoded} {narrower}"
    );
    if multicad_kernel::OCCT_LINKED {
        let imported = multicad_kernel::Shape::read_step(&path).unwrap();
        assert!(imported.valid().unwrap());
        assert!((imported.volume().unwrap() - narrower).abs() / narrower < 1e-7);
    }
}
