use multicad_core::{
    commands::{Actor, Channel, Session},
    document::Document,
    model::{
        self, BodyInput, ButtressLoadFlankSide, PrimitiveParameters, PrimitiveRequest,
        ThreadFamily, ThreadHand, ThreadMode, ThreadParameters, ThreadRequest, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compile(
    s: &Session,
    backend: GeometryBackend,
) -> Result<multicad_model::Model, multicad_model::ModelError> {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(s.document(), s.revision(), backend, None)
}
fn reopen(s: &Session) -> Session {
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(s.document().clone()).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    Session::new(
        project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document,
    )
    .unwrap()
}
#[cfg(feature = "occt")]
fn native_material(solid: &multicad_kernel::Solid, point: [f64; 3]) -> bool {
    use multicad_kernel::{BooleanOperation, GeometryPolicy, RigidTransform, Solid};
    let edge = 0.00002;
    let probe = Solid::box_solid([edge; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: point.map(|p| p - edge / 2.),
            ..Default::default()
        })
        .unwrap();
    solid
        .boolean(
            &probe,
            BooleanOperation::Intersect,
            GeometryPolicy::default(),
        )
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / edge.powi(3)
        > 0.5
}
#[test]
fn sourced_lead_and_asymmetric_root_pairs_keep_geometry_phase_history_and_step() {
    let mut cases: Vec<ThreadParameters> = model::lead_screw_presets()
        .into_iter()
        .map(|row| row.parameters())
        .collect();
    let mut multi = model::lead_screw_presets().remove(1).parameters();
    multi.hand = ThreadHand::Left;
    multi.starts = 2;
    cases.push(multi);
    for side in [
        ButtressLoadFlankSide::NegativeAxisSide,
        ButtressLoadFlankSide::PositiveAxisSide,
    ] {
        for hand in [ThreadHand::Right, ThreadHand::Left] {
            let mut p = model::buttress_thread_presets().remove(0).parameters(side);
            p.hand = hand;
            cases.push(p);
        }
    }
    #[cfg(feature = "occt")]
    let mut native_step_errors = Vec::<String>::new();
    for (index, mut p) in cases.into_iter().enumerate() {
        let mut s =
            Session::new(Document::new("Rust lead and buttress geometry").unwrap()).unwrap();
        if index % 2 != 0 {
            p.frame = model::PrimitiveFrame {
                origin: [17., -8., 13.].map(length),
                u: [0.8, 0.6, 0.],
                v: [-0.36, 0.48, 0.8],
            };
        }
        let initial = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
        p.length = length(3.37 * initial.pitch_mm);
        let major = p.major_diameter.clone();
        let pitch = p.pitch.clone();
        let male = ThreadRequest::new("Sourced external", p.clone());
        s.commit(
            model::prepare_thread(&s, s.revision(), actor(), &male)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let resolved = p.resolve(s.parameters(), ThreadMode::NewExternal).unwrap();
        let blank = PrimitiveRequest::new(
            "Receiving blank",
            PrimitiveParameters::Cylinder {
                radius: length(resolved.major_radius_mm + 3.),
                height: p.length.clone(),
                frame: p.frame.clone(),
            },
        );
        s.commit(
            model::prepare_primitive(&s, s.revision(), actor(), &blank)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let female = ThreadRequest::receiving(
            "Sourced internal",
            ThreadMode::CutInternal,
            BodyInput {
                body: blank.body,
                producer: blank.feature,
            },
            p.clone(),
        )
        .unwrap();
        s.commit(
            model::prepare_thread(&s, s.revision(), actor(), &female)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let rust =
            compile(&s, GeometryBackend::Rust).unwrap_or_else(|e| panic!("case{index}: {e}"));
        #[cfg(feature = "occt")]
        let native = compile(&s, GeometryBackend::OpenCascade)
            .unwrap_or_else(|e| panic!("native model case{index}: {e}"));
        for (internal, request) in [(false, &male), (true, &female)] {
            let solid = rust.solid(request.feature).unwrap();
            assert!(solid.uses_rust_geometry());
            let r = request
                .parameters
                .resolve(s.parameters(), request.mode)
                .unwrap();
            let world = |point: [f64; 3]| {
                let vector = r.frame.vector(point);
                std::array::from_fn(|i| r.frame.origin[i] + vector[i])
            };
            let phase = r.buttress.as_ref().map_or(0., |b| b.crest_phase_axial_mm);
            for (sign, angle) in [
                (-1., r.profile.lower_flank_deg),
                (1., r.profile.upper_flank_deg),
            ] {
                for fraction in [0.2, 0.6] {
                    let drop = r.profile.depth_mm * fraction;
                    let z = 2. * r.pitch_mm
                        + phase
                        + sign * (r.profile.crest_width_mm / 2. + drop * angle.to_radians().tan());
                    for (offset, in_profile) in [(-0.001, true), (0.001, false)] {
                        let point = [r.major_radius_mm - drop + offset, 0., z];
                        #[cfg(feature = "occt")]
                        if r.buttress.is_some() && fraction == 0.6 {
                            assert_eq!(
                                native_material(
                                    native.solid(request.feature).unwrap(),
                                    world(point)
                                ),
                                if internal { !in_profile } else { in_profile },
                                "native phase case{index} internal{internal} flank{sign}"
                            );
                        }
                        assert_eq!(
                            solid.rust_thread_contains(world(point)).unwrap(),
                            if internal { !in_profile } else { in_profile },
                            "case{index} internal{internal} flank{sign} fraction{fraction}"
                        );
                    }
                }
            }
            if let Some(circle) = r.buttress.as_ref().and_then(|b| b.root_circle.as_ref()) {
                assert!(!internal);
                for angle in [-0.5_f64, 0.5] {
                    let z =
                        2. * r.pitch_mm + circle.center_axial_mm + circle.radius_mm * angle.sin();
                    let boundary = circle.center_radius_mm - circle.radius_mm * angle.cos();
                    #[cfg(feature = "occt")]
                    {
                        let native = native.solid(request.feature).unwrap();
                        assert!(native_material(native, world([boundary - 0.001, 0., z])));
                        assert!(!native_material(native, world([boundary + 0.001, 0., z])));
                    }
                    assert!(
                        solid
                            .rust_thread_contains(world([boundary - 0.001, 0., z]))
                            .unwrap()
                    );
                    assert!(
                        !solid
                            .rust_thread_contains(world([boundary + 0.001, 0., z]))
                            .unwrap()
                    );
                }
            }
            let folder = tempfile::tempdir().unwrap();
            let path = folder.path().join("thread.step");
            if r.starts != 1 {
                let error = solid.write_step(&path).unwrap_err();
                assert_eq!(error.kind, multicad_kernel::GeometryErrorKind::Unsupported);
                assert!(!path.exists());
                continue;
            }
            solid
                .write_step(&path)
                .unwrap_or_else(|e| panic!("STEP case{index} internal{internal}: {e}"));
            let verified = solid
                .verify_helical_step(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .unwrap();
            assert!((verified - solid.volume_mm3().unwrap()).abs() < 1e-3);
            #[cfg(feature = "occt")]
            {
                match multicad_kernel::Solid::read_step(&path, Default::default()) {
                    Ok(read) => assert!(
                        (read.volume_mm3().unwrap() - solid.volume_mm3().unwrap()).abs() < 1e-3
                    ),
                    Err(error) => {
                        native_step_errors.push(format!("case{index} internal{internal}: {error}"))
                    }
                }
            }
        }
        #[cfg(feature = "occt")]
        {
            for id in [male.feature, female.feature] {
                let (a, b) = (native.solid(id).unwrap(), rust.solid(id).unwrap());
                assert!(
                    (a.volume_mm3().unwrap() - b.volume_mm3().unwrap()).abs() < 1e-3,
                    "native volume case{index}"
                );
                for (a, b) in a
                    .bounds()
                    .unwrap()
                    .min
                    .into_iter()
                    .chain(a.bounds().unwrap().max)
                    .zip(
                        b.bounds()
                            .unwrap()
                            .min
                            .into_iter()
                            .chain(b.bounds().unwrap().max),
                    )
                {
                    assert!((a - b).abs() < 1e-5, "native bound case{index}");
                }
            }
        }
        let before = s.document().clone();
        let updated = ThreadParameters {
            length: length(3.75 * resolved.pitch_mm),
            hand: if p.hand == ThreadHand::Right {
                ThreadHand::Left
            } else {
                ThreadHand::Right
            },
            ..p.clone()
        };
        s.commit(
            model::prepare_thread_edit(&s, s.revision(), actor(), male.feature, &updated)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let after = s.document().clone();
        let opened = reopen(&s);
        let saved = model::read_thread(opened.document(), male.feature).unwrap();
        assert_eq!(saved.parameters, updated);
        assert_eq!(saved.parameters.major_diameter, major);
        assert_eq!(saved.parameters.pitch, pitch);
        let rebuilt = compile(&opened, GeometryBackend::Rust).unwrap();
        assert!(
            (rebuilt.solid(male.feature).unwrap().volume_mm3().unwrap()
                - rust.solid(male.feature).unwrap().volume_mm3().unwrap() * 3.75 / 3.37)
                .abs()
                < 1e-6
        );
        s.undo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &before);
        s.redo(s.revision(), actor()).unwrap();
        assert_eq!(s.document(), &after);
        let invalid = ThreadParameters {
            minimum_wall: length(10.),
            ..p
        };
        let candidate =
            model::prepare_thread_edit(&s, s.revision(), actor(), female.feature, &invalid)
                .unwrap();
        assert!(
            Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile_with_backend(
                candidate.prepared.document(),
                s.revision() + 1,
                GeometryBackend::Rust,
                None
            )
            .is_err()
        );
        assert_eq!(s.document(), &after);
    }
    #[cfg(feature = "occt")]
    assert!(
        native_step_errors.is_empty(),
        "{}",
        native_step_errors.join("\n")
    );
}

#[test]
fn rounded_and_tapered_other_families_are_not_enabled_by_straight_admission() {
    let mut cases: Vec<_> = model::whitworth_thread_presets()
        .into_iter()
        .map(|row| ThreadParameters {
            family: row.family,
            fit: if row.family == ThreadFamily::Bspp {
                model::ThreadFit::BsppClassBExternal
            } else {
                model::ThreadFit::WhitworthMediumExternal
            },
            major_diameter: length(row.diameter_mm),
            pitch: length(row.pitch_mm),
            length: length(3. * row.pitch_mm),
            ..Default::default()
        })
        .collect();
    cases.push(
        model::PipeThreadIntent {
            form: model::PipeThreadForm::R,
            size: model::PipeThreadSize::OneEighth,
            gauge: model::PipeGaugeConvention::Iso7Nominal1994,
        }
        .parameters(),
    );
    cases.push(
        model::NptThreadIntent {
            form: model::NptThreadForm::External,
            size: model::NptThreadSize::OneEighth,
            gauge: model::NptGaugeConvention::Nbs1959PlainFace,
            truncation: model::NptTruncationTarget::MidpointConeParallel,
        }
        .parameters(),
    );
    for parameters in cases {
        let s =
            Session::new(Document::new("Unsupported rounded or tapered family").unwrap()).unwrap();
        let request = ThreadRequest::new("Unsupported sourced profile", parameters);
        let candidate = model::prepare_thread(&s, 0, actor(), &request).unwrap();
        let error = Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(
            candidate.prepared.document(),
            1,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("Unsupported sourced family must not be rewritten as a straight custom profile");
        assert_eq!(error.entity, Some(request.feature));
        assert!(error.message.contains("Rust thread geometry"), "{error}");
        assert_eq!(s.revision(), 0);
    }
}
