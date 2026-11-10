use super::common::*;
use multicad_core::{
    constraint_sketch::Dimension,
    model::{self, *},
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologySnapshot},
};
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
