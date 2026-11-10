use super::common::*;
use multicad_core::{
    constraint_sketch::Dimension,
    model::{self, *},
    project_file::{self, ProjectFile},
    references::{self, AnalyticGeometry, GeometryIntent, Origin, Resolution, TopologySnapshot},
};
use multicad_model::{Compiler, GeometryBackend};

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
