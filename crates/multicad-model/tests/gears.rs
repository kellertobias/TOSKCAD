use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, GearFamily, GearHand, GearParameters, GearRequest, PrimitiveFrame, angle, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::{GeometryPolicy, Solid};
use multicad_model::Compiler;

#[test]
fn rust_straight_rack_preserves_placement_tooth_origins_and_step_exchange() {
    let mut session = Session::new(Document::new("Placed straight rack").unwrap()).unwrap();
    let request = GearRequest::new(
        "Rack",
        GearParameters {
            family: GearFamily::Rack,
            teeth: multicad_core::constraint_sketch::Dimension::Literal {
                value: 25.,
                unit: "count".into(),
            },
            module: length(2.),
            rack: Some(model::RackDimensions {
                backing_thickness: length(4.),
            }),
            face_width: length(7.),
            bore_diameter: length(0.),
            frame: PrimitiveFrame {
                origin: [length(10.), length(20.), length(30.)],
                u: [0., 1., 0.],
                v: [0., 0., 1.],
            },
            ..Default::default()
        },
    );
    let prepared = model::prepare_gear(&session, 0, actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let compiler = Compiler::default();
    use multicad_model::GeometryBackend;
    let [rust, native] = [GeometryBackend::Rust, GeometryBackend::OpenCascade].map(|backend| {
        compiler
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap()
    });
    let feature = request.feature;
    let [actual, expected] = [&rust, &native].map(|model| model.solid(feature).unwrap());
    let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-6, "{a} != {b}");
    assert!(actual.uses_rust_geometry());
    assert_eq!(actual.solid_count().unwrap(), 1);
    near(actual.volume_mm3().unwrap(), expected.volume_mm3().unwrap());
    let [a, b] = [actual, expected].map(|solid| solid.bounds().unwrap());
    for i in 0..3 {
        near(a.min[i], b.min[i]);
        near(a.max[i], b.max[i]);
    }
    let topology = &rust.snapshot().shapes[&feature].topology;
    for tooth in 0..25 {
        let role = format!("rack/tooth/{tooth}/fillet/right");
        assert!(topology.elements.iter().any(|element| {
            element.key.kind == multicad_core::references::TopologyKind::Face
                && element.origins.iter().any(|origin| origin.role == role)
        }));
    }
    actual
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("rack.step");
    actual.write_step(&path).unwrap();
    let readback = Solid::read_step(&path, compiler.policy).unwrap();
    near(
        readback.volume_mm3().unwrap(),
        expected.volume_mm3().unwrap(),
    );
    let verified = multicad_model::export::verify_rust_step_shape(
        &path,
        rust.solid(feature).unwrap(),
        &rust.snapshot().shapes[&feature],
        compiler.policy.linear_tolerance_mm,
    )
    .unwrap();
    near(verified, expected.volume_mm3().unwrap());
}

#[test]
fn unchanged_gear_reuses_native_result_and_recipe_edits_invalidate_it() {
    let mut s = Session::new(Document::new("Cached gear").unwrap()).unwrap();
    let request = GearRequest::new(
        "Gear",
        GearParameters {
            face_width: length(4.),
            ..Default::default()
        },
    );
    let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let mut cache = multicad_model::EvaluationCache::default();
    let first = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(cache.last_compile().evaluated.contains(&request.feature));
    let again = compiler()
        .compile_cached(&mut cache, s.document(), 1, None)
        .unwrap();
    assert!(cache.last_compile().reused.contains(&request.feature));
    assert!(cache.last_compile().evaluated.is_empty());
    assert_eq!(
        first.solid(request.feature).unwrap().volume_mm3().unwrap(),
        again.solid(request.feature).unwrap().volume_mm3().unwrap()
    );
    let mut params = request.parameters.clone();
    params.face_width = length(5.);
    let p = model::prepare_gear_edit(&s, 1, actor(), request.feature, &params).unwrap();
    s.commit(p.prepared).unwrap();
    let edited = compiler()
        .compile_cached(&mut cache, s.document(), 2, None)
        .unwrap();
    assert!(cache.last_compile().evaluated.contains(&request.feature));
    assert!(!cache.last_compile().reused.contains(&request.feature));
    assert!(
        (edited.solid(request.feature).unwrap().volume_mm3().unwrap()
            - first.solid(request.feature).unwrap().volume_mm3().unwrap() * 1.25)
            .abs()
            < 0.001
    );
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
#[test]
fn native_gear_candidate_cancel_edit_history_and_reopen_preserve_one_body() {
    let mut s = Session::new(Document::new("Native gears").unwrap()).unwrap();
    let request = GearRequest::new(
        "Spur gear",
        GearParameters {
            face_width: length(4.),
            ..Default::default()
        },
    );
    let before = s.document().clone();
    let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
    let candidate = compiler().compile(p.prepared.document(), 1).unwrap();
    let volume = candidate
        .solid(request.feature)
        .unwrap()
        .volume_mm3()
        .unwrap();
    assert!(volume > 6000.);
    drop(p);
    assert_eq!(s.document(), &before);
    let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
    s.commit(p.prepared).unwrap();
    let committed = s.document().clone();
    let parameters = GearParameters {
        face_width: length(5.),
        ..request.parameters.clone()
    };
    let p = model::prepare_gear_edit(&s, 1, actor(), request.feature, &parameters).unwrap();
    let candidate = compiler().compile(p.prepared.document(), 2).unwrap();
    assert!(
        (candidate
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume * 1.25)
            .abs()
            < 0.001
    );
    assert_eq!(candidate.snapshot().bodies[&request.body], request.feature);
    s.commit(p.prepared).unwrap();
    let edited = s.document().clone();
    s.undo(2, actor()).unwrap();
    assert_eq!(s.document(), &committed);
    s.redo(3, actor()).unwrap();
    assert_eq!(s.document(), &edited);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(edited).unwrap(),
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
    let rebuilt = compiler().compile(&reopened.document, 0).unwrap();
    assert_eq!(rebuilt.snapshot().bodies[&request.body], request.feature);
    assert!(
        (rebuilt
            .solid(request.feature)
            .unwrap()
            .volume_mm3()
            .unwrap()
            - volume * 1.25)
            .abs()
            < 0.001
    );
}
#[test]
fn every_supported_family_builds_in_its_canonical_frame_and_survives_step_readback() {
    for family in GearFamily::ALL.into_iter().filter(|f| f.supported()) {
        let mut s = Session::new(Document::new("Framed gear").unwrap()).unwrap();
        let internal = family == GearFamily::InternalRing;
        let rack = family == GearFamily::Rack;
        let bevel = family == GearFamily::Bevel;
        let worm = family == GearFamily::Worm;
        let helical = matches!(
            family,
            GearFamily::Helical | GearFamily::Herringbone | GearFamily::DoubleHelical
        );
        let request = GearRequest::new(
            "Gear",
            GearParameters {
                family,
                teeth: multicad_core::constraint_sketch::Dimension::Literal {
                    value: if worm {
                        1.
                    } else if internal {
                        48.
                    } else {
                        24.
                    },
                    unit: "count".into(),
                },
                bore_diameter: length(if internal || rack || bevel || worm {
                    0.
                } else {
                    8.
                }),
                module_system: if worm {
                    model::GearModuleSystem::Axial
                } else {
                    model::GearModuleSystem::Normal
                },
                worm: worm.then(|| model::WormDimensions {
                    role: model::WormMember::Worm,
                    tooth_system: model::WormToothSystem::ZaAxialStraight,
                    mate_teeth: multicad_core::constraint_sketch::Dimension::Literal {
                        value: 32.,
                        unit: "count".into(),
                    },
                    diameter_quotient: model::ratio(10.),
                }),
                root_fillet_ratio: model::ratio(if worm { 0. } else { 0.2 }),
                bevel: bevel.then(|| model::BevelDimensions {
                    tooth_system: model::BevelToothSystem::SphericalInvoluteStraight,
                    mate_teeth: multicad_core::constraint_sketch::Dimension::Literal {
                        value: 32.,
                        unit: "count".into(),
                    },
                    shaft_angle: angle(90.),
                }),
                rack: rack.then(|| model::RackDimensions {
                    backing_thickness: length(4.),
                }),
                outer_diameter: internal.then(|| length(112.)),
                face_width: length(if worm { 32. } else { 4. }),
                helix_angle: angle(if helical { 15. } else { 0. }),
                hand: if worm {
                    GearHand::Right
                } else if helical {
                    GearHand::Left
                } else {
                    GearHand::None
                },
                center_gap: length(if family == GearFamily::DoubleHelical {
                    1.
                } else {
                    0.
                }),
                frame: PrimitiveFrame {
                    origin: [length(10.), length(20.), length(30.)],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
                ..Default::default()
            },
        );
        let p = model::prepare_gear(&s, 0, actor(), &request).unwrap();
        s.commit(p.prepared).unwrap();
        let built = compiler().compile(s.document(), 1).unwrap();
        let solid = built.solid(request.feature).unwrap();
        assert_eq!(solid.solid_count().unwrap(), 1);
        let bounds = solid.bounds().unwrap();
        if bevel {
            let geometry = request.parameters.resolve_geometry(s.parameters()).unwrap();
            let cone = geometry.bevel().unwrap();
            // Source frame maps local shaft Z to world X. Back cones, rather
            // than planar extrusion caps, set the two axial limits.
            assert!((bounds.min[0] - (10. + cone.toe_tip_axial_mm)).abs() < 1e-4);
            assert!(
                (bounds.max[0]
                    - (10. + cone.outer_cone_distance_mm / cone.pitch_cone_angle_rad.cos()))
                .abs()
                    < 1e-4
            );
            assert!((bounds.max[1] - (20. + cone.heel_tip_radius_mm)).abs() < 1e-4);
        } else if worm {
            assert!((bounds.min[0] - (10. - 16.)).abs() < 2e-6);
            assert!((bounds.max[0] - (10. + 16.)).abs() < 2e-6);
        } else {
            assert!((bounds.min[0] - 10.).abs() < 1e-6);
            assert!((bounds.max[0] - 14.).abs() < 1e-6);
        }
        if rack {
            let half_length = 24. * std::f64::consts::PI;
            assert!((bounds.min[1] - (20. - half_length)).abs() < 1e-6);
            assert!((bounds.max[1] - (20. + half_length)).abs() < 1e-6);
            assert!((bounds.min[2] - 23.5).abs() < 1e-6);
            assert!((bounds.max[2] - 32.).abs() < 1e-6);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gear.step");
        solid.write_step(&path).unwrap();
        let readback = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
        assert_eq!(readback.solid_count().unwrap(), 1);
        assert!((readback.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 0.001);
    }
}
