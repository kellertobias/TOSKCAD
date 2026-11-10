use super::common::*;
use multicad_core::{
    constraint_sketch::Dimension,
    model::{self, *},
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologySnapshot},
};
use multicad_kernel::{Bounds, GeometryPolicy, Solid};
use multicad_model::{Compiler, GeometryBackend};

#[test]
fn merged_linear_pattern_rebuilds_touching_and_separate_stock_in_one_union() {
    let (mut session, input) = fixture();
    apply_parameter(&mut session, "copies", "3");
    let request = AdvancedRequest::on_body(
        input,
        AdvancedParameters::LinearPattern(LinearPatternParameters {
            count: bound(&session, "copies", "count"),
            step: [bound(&session, "pitch", "mm"), length(0.), length(0.)],
            merge: true,
        }),
        vec![],
    );
    let authored =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(authored.prepared).unwrap();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };

    for (pitch, expected_volume, expected_solids, max_x) in [
        ("20", 14000., 1, 70.),
        ("30", 18000., 1, 90.),
        ("40", 18000., 3, 110.),
    ] {
        apply_parameter(&mut session, "pitch", pitch);
        let compile = |backend| {
            compiler
                .compile_with_backend(session.document(), session.revision(), backend, None)
                .unwrap()
        };
        let rust = compile(GeometryBackend::Rust);
        let solid = rust.solid(request.body).unwrap();
        assert!(solid.uses_rust_geometry());
        near(solid.volume_mm3().unwrap(), expected_volume);
        assert_eq!(solid.solid_count().unwrap(), expected_solids);
        let bounds = solid.bounds().unwrap();
        assert_eq!(bounds.min, [0.; 3]);
        assert_eq!(bounds.max, [max_x, 20., 10.]);
        let shape = &rust.snapshot().shapes[&request.feature].topology;
        for member in 0..3 {
            let origin = Origin {
                source: request.feature,
                role: format!("member/{member}"),
            };
            assert!(
                shape
                    .elements
                    .iter()
                    .any(|element| element.origins.contains(&origin)),
                "member {member} lost its source identity at pitch {pitch}"
            );
        }

        if cfg!(feature = "occt") {
            let native = compile(GeometryBackend::OpenCascade);
            let solid = native.solid(request.body).unwrap();
            near(solid.volume_mm3().unwrap(), expected_volume);
            assert_eq!(solid.solid_count().unwrap(), expected_solids);
            let native_bounds = solid.bounds().unwrap();
            for (actual, expected) in bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip(native_bounds.min.into_iter().chain(native_bounds.max))
            {
                near(actual, expected);
            }
        }
    }
}

fn member_points(topology: &TopologySnapshot, member: &Origin) -> Vec<[f64; 3]> {
    topology
        .elements
        .iter()
        .filter(|element| element.origins.contains(member))
        .filter_map(|element| match element.geometry {
            Some(AnalyticGeometry::Point { position_mm }) => Some(position_mm),
            _ => None,
        })
        .collect()
}

#[test]
fn merged_rotated_pattern_rebuilds_overlapping_and_independent_members_and_reopens_step() {
    let (mut session, input) = fixture();
    let seed = PrimitiveParameters::Box {
        size: [length(10.), length(10.), length(4.)],
        frame: PrimitiveFrame::default(),
    };
    let edited =
        model::prepare_primitive_edit(&session, session.revision(), actor(), input.producer, &seed)
            .unwrap();
    session.commit(edited.prepared).unwrap();
    apply_parameter(&mut session, "pitch", "5");
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::CircularPattern(CircularPatternParameters {
            count: bound(&session, "copies", "count"),
            step_angle: angle(45.),
            origin: [bound(&session, "pitch", "mm"), length(5.), length(0.)],
            axis: [0., 0., 1.],
            merge: true,
        }),
        vec![],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let radius = 5. * std::f64::consts::SQRT_2;
    let check = |solid: &Solid, volume: f64, count, expected: Bounds| {
        near(solid.volume_mm3().unwrap(), volume);
        assert_eq!(solid.solid_count().unwrap(), count);
        let bounds = solid.bounds().unwrap();
        for i in 0..3 {
            near(bounds.min[i], expected.min[i]);
            near(bounds.max[i], expected.max[i]);
        }
    };
    for (pivot, volume, count) in [
        (5., (100. + 2. * (10. - radius).powi(2)) * 4., 1),
        (50., 800., 2),
    ] {
        apply_parameter(&mut session, "pitch", &pivot.to_string());
        let file = ProjectFile::new(session.document().clone()).unwrap();
        let mut bytes = vec![];
        project_file::write(&file, &mut bytes, Default::default()).unwrap();
        let restored = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(&restored.document, session.document());
        let compile = |backend| {
            Compiler {
                omit_display: true,
                ..Default::default()
            }
            .compile_with_backend(&restored.document, 0, backend, None)
            .unwrap()
        };
        let rust = compile(GeometryBackend::Rust);
        assert_eq!(rust.snapshot().body_sources[&request.body], request.feature);
        assert_eq!(request.body, input.body);
        near(
            rust.solid(input.producer).unwrap().volume_mm3().unwrap(),
            400.,
        );
        let offset = (5. - pivot) * std::f64::consts::FRAC_1_SQRT_2;
        let center = [pivot + offset, 5. + offset];
        let expected = Bounds {
            min: [
                (center[0] - radius).min(0.),
                (center[1] - radius).min(0.),
                0.,
            ],
            max: [
                (center[0] + radius).max(10.),
                (center[1] + radius).max(10.),
                4.,
            ],
        };
        let merged = rust.solid(request.body).unwrap();
        assert!(merged.uses_rust_geometry());
        check(merged, volume, count, expected);
        let topology = &rust.snapshot().shapes[&request.feature].topology;
        for member in 0..2 {
            let origin = Origin {
                source: request.feature,
                role: format!("member/{member}"),
            };
            assert!(
                !member_points(topology, &origin).is_empty(),
                "missing {origin:?}"
            );
        }

        let path = dir.path().join(format!("rotated-pattern-{count}.step"));
        assert!(merged.supports_step_export());
        merged.write_step(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let policy = GeometryPolicy::default();
        let reopened = Solid::import_rust_step(&text, request.feature, policy).unwrap();
        check(&reopened, volume, count, expected);
        #[cfg(feature = "occt")]
        {
            let native = compile(GeometryBackend::OpenCascade);
            check(native.solid(request.body).unwrap(), volume, count, expected);
            check(
                &Solid::read_step(&path, policy).unwrap(),
                volume,
                count,
                expected,
            );
        }
    }
}

#[test]
fn merged_circular_pattern_declines_curved_members_without_changing_the_document() {
    let (mut session, input) = fixture();
    let seed = PrimitiveParameters::Cylinder {
        radius: length(3.),
        height: length(5.),
        frame: PrimitiveFrame::default(),
    };
    let edited =
        model::prepare_primitive_edit(&session, session.revision(), actor(), input.producer, &seed)
            .unwrap();
    session.commit(edited.prepared).unwrap();
    let request = AdvancedRequest::on_body(
        input,
        AdvancedParameters::CircularPattern(CircularPatternParameters {
            count: bound(&session, "copies", "count"),
            step_angle: angle(45.),
            origin: [length(1.), length(0.), length(0.)],
            axis: [0., 0., 1.],
            merge: true,
        }),
        vec![],
    );
    let original = session.document().clone();
    let revision = session.revision();
    let prepared = model::prepare_advanced(&session, revision, actor(), &request).unwrap();
    let error = Compiler::default()
        .compile_with_backend(
            prepared.prepared.document(),
            revision,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .expect("A merged curved pattern needs a certified curved union");
    assert_eq!(error.entity, Some(request.feature));
    assert!(
        error.message.contains("certified straight-sided"),
        "{error}"
    );
    assert_eq!(session.document(), &original);
    assert_eq!(session.revision(), revision);
}

#[test]
fn saved_oblique_circular_pattern_retains_rotated_members_after_pitch_edit() {
    let (mut session, input) = fixture();
    let seed = PrimitiveParameters::Box {
        size: [length(3.), length(5.), length(8.)],
        frame: PrimitiveFrame {
            origin: [14., -7., 5.].map(length),
            u: [0.6, 0.8, 0.],
            v: [-0.48, 0.36, 0.8],
        },
    };
    let edited =
        model::prepare_primitive_edit(&session, session.revision(), actor(), input.producer, &seed)
            .unwrap();
    session.commit(edited.prepared).unwrap();
    apply_parameter(&mut session, "angle", "61");
    let request = AdvancedRequest::on_body(
        input,
        AdvancedParameters::CircularPattern(CircularPatternParameters {
            count: Dimension::Literal {
                value: 3.,
                unit: "count".into(),
            },
            step_angle: bound(&session, "angle", "deg"),
            origin: [2., 3., -1.].map(length),
            axis: [1., 2., 3.],
            merge: false,
        }),
        vec![],
    );
    let authored =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(authored.prepared).unwrap();

    let compile = |backend, session: &multicad_core::commands::Session| {
        Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
    };
    let original = compile(GeometryBackend::Rust, &session);
    let topology = &original.snapshot().shapes[&request.feature].topology;
    let last = Origin {
        source: request.feature,
        role: "member/2".into(),
    };
    let selected = topology
        .elements
        .iter()
        .find(|element| {
            element.origins.contains(&last)
                && matches!(element.geometry, Some(AnalyticGeometry::Point { .. }))
        })
        .unwrap();
    let capture = GeometryIntent::capture(topology, selected.key).unwrap();
    let original_position = selected.geometry.unwrap();

    for pitch in ["61", "-37"] {
        apply_parameter(&mut session, "angle", pitch);
        let file = ProjectFile::new(session.document().clone()).unwrap();
        let mut bytes = vec![];
        project_file::write(&file, &mut bytes, Default::default()).unwrap();
        let restored = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(&restored.document, session.document());
        let saved = multicad_core::commands::Session::new(restored.document).unwrap();
        let rust = compile(GeometryBackend::Rust, &saved);
        let solid = rust.solid(request.body).unwrap();
        let shape = &rust.snapshot().shapes[&request.feature].topology;
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.solid_count().unwrap(), 3);
        near(solid.volume_mm3().unwrap(), 360.);
        let Resolution::Resolved { geometry, .. } =
            references::resolve(&capture, shape, saved.revision())
        else {
            panic!("rotated member lost its reference after pitch {pitch}");
        };
        assert_eq!(geometry == Some(original_position), pitch == "61");

        #[cfg(feature = "occt")]
        let native = compile(GeometryBackend::OpenCascade, &saved);
        #[cfg(feature = "occt")]
        near(
            native.solid(request.body).unwrap().volume_mm3().unwrap(),
            360.,
        );
        for index in 0..3 {
            let member = Origin {
                source: request.feature,
                role: format!("member/{index}"),
            };
            let points = member_points(shape, &member);
            assert_eq!(points.len(), 8);
            #[cfg(feature = "occt")]
            {
                let native_points = member_points(
                    &native.snapshot().shapes[&request.feature].topology,
                    &member,
                );
                assert_eq!(native_points.len(), 8);
                for point in points {
                    assert!(
                        native_points.iter().any(|expected| {
                            point
                                .iter()
                                .zip(expected)
                                .all(|(a, b)| (a - b).abs() < 1e-7)
                        }),
                        "member {index} at pitch {pitch} has no native vertex for {point:?}"
                    );
                }
            }
        }
    }
}
