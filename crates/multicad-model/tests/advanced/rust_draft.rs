use super::common::*;
use multicad_core::{
    commands::{Request, Session},
    document::{Entity, EntityData, EntityId, Link, Mutation},
    model::{self, AdvancedParameters, AdvancedRequest, DraftParameters, PrimitiveFrame},
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution,
    },
};
use multicad_kernel::{
    GeometryPolicy, PlaneFrame, Solid, TreatmentFace, rust_kernel::step_read::read_analytic_solid,
};
use multicad_model::{Compiler, GeometryBackend};

fn face_metrics(solid: &Solid) -> Vec<[i64; 8]> {
    let mut faces: Vec<_> = solid
        .faces()
        .unwrap()
        .into_iter()
        .map(|face| {
            let (point, normal) = face.plane.unwrap();
            let offset = point
                .into_iter()
                .zip(normal)
                .map(|(a, b)| a * b)
                .sum::<f64>();
            let [x, y, z] = normal;
            let [a, b, c] = face.center_mm;
            [x, y, z, offset, face.area_mm2, a, b, c].map(|n| (n * 1e6).round() as i64)
        })
        .collect();
    faces.sort_unstable();
    faces
}

fn cardinal(axis: usize, positive: bool) -> [f64; 3] {
    let mut normal = [0.; 3];
    normal[axis] = if positive { 1. } else { -1. };
    normal
}

fn selected_side(
    solid: &Solid,
    normal: [f64; 3],
    feature: EntityId,
    role: String,
) -> TreatmentFace {
    let topology = solid.topology_snapshot(feature, 0).unwrap();
    let face = topology
        .elements
        .iter()
        .find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame })
            if frame.normal().into_iter().zip(normal).all(|(a, b)| (a - b).abs() < 1e-9))
        })
        .unwrap();
    TreatmentFace {
        key: face.key,
        origin: Origin {
            source: feature,
            role,
        },
    }
}

fn drafted_volume(
    size: [f64; 3],
    up: usize,
    pull: f64,
    radians: f64,
    neutral_level: f64,
    sides: &[(usize, bool)],
) -> f64 {
    let [u, v] = [(up + 1) % 3, (up + 2) % 3];
    // Each selected side reduces its section width by pull * tan(angle)
    // per axial millimeter. Integrate the product of the two linear widths.
    let [du, dv] = [u, v].map(|axis| {
        -(sides.iter().filter(|&&(side, _)| side == axis).count() as f64) * pull * radians.tan()
    });
    let [a, b] = [size[u] - du * neutral_level, size[v] - dv * neutral_level];
    let height = size[up];
    a * b * height + (a * dv + b * du) * height.powi(2) / 2. + du * dv * height.powi(3) / 3.
}

#[test]
fn rust_draft_modifies_adjacent_opposite_and_all_four_sides_for_each_cardinal_pull() {
    #[cfg(feature = "occt")]
    use multicad_kernel::RigidTransform;
    use multicad_kernel::rust_kernel::Rational;

    let policy = GeometryPolicy::default();
    let size = [30., 20., 10.];
    let origin = [10., -7., 5.];
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame {
            origin,
            ..Default::default()
        },
        policy,
    )
    .unwrap();
    #[cfg(feature = "occt")]
    let native_stock = Solid::box_solid(size, policy)
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: origin,
            ..Default::default()
        })
        .unwrap();
    let feature = EntityId::new();
    for up in 0..3 {
        let [u, v] = [(up + 1) % 3, (up + 2) % 3];
        let neutral_level = size[up] * 0.3;
        let mut neutral_origin = origin;
        neutral_origin[up] += neutral_level;
        let neutral = PlaneFrame {
            origin: neutral_origin,
            u: cardinal(u, true),
            v: cardinal(v, true),
        };
        for sides in [
            vec![(u, true), (v, true)],
            vec![(u, false), (u, true)],
            vec![(u, false), (u, true), (v, false), (v, true)],
        ] {
            let selections = sides
                .iter()
                .map(|&(side, positive)| {
                    selected_side(
                        &stock,
                        cardinal(side, positive),
                        feature,
                        format!("draft/{side}/{positive}"),
                    )
                })
                .collect::<Vec<_>>();
            for sense in [-1., 1.] {
                let pull = cardinal(up, sense > 0.);
                for degrees in [-5_f64, 5.] {
                    let radians = degrees.to_radians();
                    let result = stock
                        .draft_faces_named(&selections, pull, neutral, radians, feature, policy)
                        .unwrap();
                    assert_eq!(result.solid_count().unwrap(), 1);
                    near(
                        result.volume_mm3().unwrap(),
                        drafted_volume(size, up, sense, radians, neutral_level, &sides),
                    );
                    assert_eq!(result.faces().unwrap().len(), 6);
                    let topology = result.topology_snapshot(feature, 1).unwrap();
                    for (&(side, positive), selection) in sides.iter().zip(&selections) {
                        let modified = topology
                            .elements
                            .iter()
                            .find(|element| {
                                element.origins.contains(&selection.origin)
                                    && matches!(
                                        element.geometry,
                                        Some(AnalyticGeometry::Plane { .. })
                                    )
                            })
                            .unwrap();
                        let Some(AnalyticGeometry::Plane { frame }) = modified.geometry else {
                            unreachable!()
                        };
                        let mut expected = cardinal(side, positive);
                        expected[up] = sense * radians.tan();
                        let norm = expected.into_iter().map(|n| n * n).sum::<f64>().sqrt();
                        for (actual, expected) in frame.normal().into_iter().zip(expected) {
                            near(actual, expected / norm);
                        }
                        let mut neutral_point = neutral_origin;
                        neutral_point[side] = origin[side] + if positive { size[side] } else { 0. };
                        let distance: f64 = frame
                            .normal()
                            .into_iter()
                            .enumerate()
                            .map(|(i, n)| n * (neutral_point[i] - frame.origin_mm[i]))
                            .sum();
                        near(distance, 0.);
                    }
                    #[cfg(feature = "occt")]
                    {
                        let selections = sides
                            .iter()
                            .map(|&(side, positive)| {
                                selected_side(
                                    &native_stock,
                                    cardinal(side, positive),
                                    feature,
                                    format!("draft/{side}/{positive}"),
                                )
                            })
                            .collect::<Vec<_>>();
                        let native = native_stock
                            .draft_faces_named(&selections, pull, neutral, radians, feature, policy)
                            .unwrap();
                        near(result.volume_mm3().unwrap(), native.volume_mm3().unwrap());
                        let a = result.bounds().unwrap();
                        let b = native.bounds().unwrap();
                        for (a, b) in a
                            .min
                            .into_iter()
                            .chain(a.max)
                            .zip(b.min.into_iter().chain(b.max))
                        {
                            near(a, b);
                        }
                        assert_eq!(face_metrics(&result), face_metrics(&native));
                    }
                }
            }
        }
    }
    near(stock.volume_mm3().unwrap(), 6000.);
    assert_eq!(stock.bounds().unwrap().min, origin);
}

#[test]
fn saved_four_side_draft_keeps_every_selected_datum_after_signed_parameter_edits() {
    let (mut session, input) = fixture();
    let build = |session: &Session, backend| {
        Compiler {
            omit_display: true,
            ..Default::default()
        }
        .compile_with_backend(session.document(), session.revision(), backend, None)
        .unwrap()
    };
    let source = build(&session, GeometryBackend::Rust);
    let topology = &source.snapshot().shapes[&input.producer].topology;
    let normals = [[-1., 0., 0.], [1., 0., 0.], [0., -1., 0.], [0., 1., 0.]];
    let captures = normals.map(|normal| {
        let face = topology.elements.iter().find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == normal)
        }).unwrap();
        GeometryIntent::capture(topology, face.key).unwrap()
    });
    let caps = [[0., 0., -1.], [0., 0., 1.]].map(|normal| {
        let face = topology.elements.iter().find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == normal)
        }).unwrap();
        GeometryIntent::capture(topology, face.key).unwrap()
    });
    let mut neutral = PrimitiveFrame::default();
    neutral.origin[2] = bound(&session, "mirror_z", "mm");
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Draft(DraftParameters {
            angle: bound(&session, "draft", "deg"),
            neutral,
            ..Default::default()
        }),
        captures.to_vec(),
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let initial = build(&session, GeometryBackend::Rust);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let mut owners = Vec::new();
    let mut edits = Vec::new();
    for (capture, normal) in captures.iter().zip(normals) {
        let anchor = Origin {
            source: request.feature,
            role: format!("face/{}", capture.id),
        };
        let face = topology
            .elements
            .iter()
            .find(|element| {
                element.origins.contains(&anchor)
                    && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
            })
            .unwrap();
        let Some(AnalyticGeometry::Plane { frame }) = face.geometry else {
            unreachable!()
        };
        let mut intent = GeometryIntent::capture(topology, face.key).unwrap();
        intent.attachment = Some(Attachment {
            anchor,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        let reference = intent.id;
        let datum = Entity::new(
            "Drafted side datum",
            EntityData::Plane {
                origin_mm: frame.origin_mm,
                normal: frame.normal(),
                anchor: Some(Link {
                    target: request.feature,
                    role: "face_attachment".into(),
                    geometry: Some(intent),
                }),
            },
        );
        owners.push((datum.id, reference, normal));
        edits.push(Mutation::Insert(datum));
    }
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach four drafted side datums".into(),
            edits,
        })
        .unwrap();

    let all_sides = [(0, false), (0, true), (1, false), (1, true)];
    for (degrees, level, width) in [(5_f64, 0., 30.), (-5., 0., 30.), (5., 3., 40.)] {
        apply_parameter(&mut session, "draft", &degrees.to_string());
        apply_parameter(&mut session, "mirror_z", &level.to_string());
        apply_parameter(&mut session, "width", &width.to_string());
        let mut bytes = Vec::new();
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let opened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let reopened = Session::new(opened.project.document).unwrap();
        let expected_volume = drafted_volume(
            [width, 20., 10.],
            2,
            1.,
            degrees.to_radians(),
            level,
            &all_sides,
        );
        let rust = build(&reopened, GeometryBackend::Rust);
        let solid = rust.solid(input.body).unwrap();
        near(solid.volume_mm3().unwrap(), expected_volume);
        assert_eq!(solid.solid_count().unwrap(), 1);
        for backend in [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ] {
            let evaluated = build(&reopened, backend);
            let actual = evaluated.solid(input.body).unwrap();
            near(actual.volume_mm3().unwrap(), expected_volume);
            assert_eq!(face_metrics(actual), face_metrics(solid));
            let report = &evaluated.snapshot().references;
            assert!(report.all_resolved(), "{report:?}");
            for &(owner, reference, normal) in &owners {
                let Resolution::Resolved {
                    frame: Some(frame), ..
                } = report
                    .get(ReferenceAddress { owner, reference })
                    .unwrap()
                    .resolution
                else {
                    panic!("A selected side datum lost its attachment");
                };
                let mut expected = normal;
                expected[2] = degrees.to_radians().tan();
                let norm = expected.into_iter().map(|n| n * n).sum::<f64>().sqrt();
                for (actual, expected) in frame.normal().into_iter().zip(expected) {
                    near(actual, expected / norm);
                }
                let point = [
                    if normal[0] > 0. { width } else { 0. },
                    if normal[1] > 0. { 20. } else { 0. },
                    level,
                ];
                near(
                    frame
                        .normal()
                        .into_iter()
                        .enumerate()
                        .map(|(i, n)| n * (point[i] - frame.origin_mm[i]))
                        .sum(),
                    0.,
                );
            }
            let topology = &evaluated.snapshot().shapes[&request.feature].topology;
            for cap in &caps {
                assert!(matches!(
                    references::resolve(cap, topology, reopened.revision()),
                    Resolution::Resolved { .. }
                ));
            }
        }
        if level == 3. {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("four-side-draft.step");
            solid.write_step(&path).unwrap();
            let tolerance = GeometryPolicy::default().linear_tolerance_mm;
            let decoded =
                read_analytic_solid(&std::fs::read_to_string(&path).unwrap(), tolerance).unwrap();
            near(decoded.volume_mm3, expected_volume);
            assert_eq!(decoded.face_count, 6);
            assert!(Solid::step_topology_matches(
                &solid
                    .topology_snapshot(request.feature, reopened.revision())
                    .unwrap(),
                &decoded,
                tolerance,
            ));
            #[cfg(feature = "occt")]
            {
                let native = Solid::read_step(&path, GeometryPolicy::default()).unwrap();
                near(native.volume_mm3().unwrap(), expected_volume);
                assert_eq!(face_metrics(&native), face_metrics(solid));
            }
        }
    }
    let before = session.document().clone();
    let revision = session.revision();
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Draft(parameters) = &mut edit.parameters else {
        unreachable!()
    };
    parameters.angle = model::angle(89.);
    let prepared = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
    let error = Compiler::default()
        .compile_with_backend(
            prepared.prepared.document(),
            revision,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(error.entity, Some(request.feature));
    assert!(error.message.contains("crosses the opposite side"));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}

#[test]
fn saved_rust_draft_rebuilds_signed_bound_angles_and_keeps_the_neutral_face_line() {
    let (mut session, input) = fixture();
    let build = |session: &Session, backend| {
        Compiler::default()
            .compile_with_backend(session.document(), session.revision(), backend, None)
            .unwrap()
    };
    let model = build(&session, GeometryBackend::Rust);
    let topology = &model.snapshot().shapes[&input.producer].topology;
    let side = |normal| {
        topology.elements.iter().find(|e| {
        matches!(e.geometry,Some(AnalyticGeometry::Plane{frame})if frame.normal()==normal)
    }).unwrap()
    };
    let selected = GeometryIntent::capture(topology, side([1., 0., 0.]).key).unwrap();
    let retained = GeometryIntent::capture(topology, side([-1., 0., 0.]).key).unwrap();
    let selection = selected.id;
    let mut neutral = PrimitiveFrame::default();
    neutral.origin[2] = bound(&session, "mirror_z", "mm");
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Draft(DraftParameters {
            angle: bound(&session, "draft", "deg"),
            neutral,
            ..Default::default()
        }),
        vec![selected],
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let model = build(&session, GeometryBackend::Rust);
    let topology = &model.snapshot().shapes[&request.feature].topology;
    let anchor = Origin {
        source: request.feature,
        role: format!("face/{selection}"),
    };
    let face = topology
        .elements
        .iter()
        .find(|e| {
            e.origins.contains(&anchor)
                && matches!(e.geometry, Some(AnalyticGeometry::Plane { .. }))
        })
        .unwrap();
    let mut intent = GeometryIntent::capture(topology, face.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = intent.id;
    let datum = Entity::new(
        "Drafted side datum",
        EntityData::Plane {
            origin_mm: [30., 0., 0.],
            normal: [1., 0., 0.],
            anchor: Some(Link {
                target: request.feature,
                role: "face_attachment".into(),
                geometry: Some(intent),
            }),
        },
    );
    let owner = datum.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach drafted side datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();

    for (degrees, level, width) in [(5_f64, 0., 30.), (-5., 0., 30.), (5., 3., 40.)] {
        apply_parameter(&mut session, "draft", &degrees.to_string());
        apply_parameter(&mut session, "mirror_z", &level.to_string());
        apply_parameter(&mut session, "width", &width.to_string());
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let document = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        let reopened = Session::new(document).unwrap();
        let rust = build(&reopened, GeometryBackend::Rust);
        let rust_solid = rust.solid(input.body).unwrap();
        assert!(rust_solid.uses_rust_geometry());
        let a = degrees.to_radians();
        let expected_volume = 200. * (width + (level - 5.) * a.tan());
        let max_x = (width + level * a.tan()).max(width + (level - 10.) * a.tan());
        let expected_faces = face_metrics(rust_solid);
        assert_eq!(expected_faces.len(), 6);
        for backend in [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ] {
            let model = build(&reopened, backend);
            let solid = model.solid(input.body).unwrap();
            near(solid.volume_mm3().unwrap(), expected_volume);
            assert_eq!(solid.solid_count().unwrap(), 1);
            let bounds = solid.bounds().unwrap();
            for (actual, expected) in bounds
                .min
                .into_iter()
                .chain(bounds.max)
                .zip([0., 0., 0., max_x, 20., 10.])
            {
                near(actual, expected);
            }
            assert_eq!(face_metrics(solid), expected_faces);
            let report = &model.snapshot().references;
            assert!(report.all_resolved(), "{report:?}");
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = report
                .get(ReferenceAddress { owner, reference })
                .unwrap()
                .resolution
            else {
                panic!("drafted side attachment was lost");
            };
            let normal = frame.normal();
            for (actual, expected) in normal.into_iter().zip([a.cos(), 0., a.sin()]) {
                near(actual, expected);
            }
            let neutral_point = [width, 0., level];
            let signed_distance = normal
                .into_iter()
                .enumerate()
                .map(|(i, n)| n * (neutral_point[i] - frame.origin_mm[i]))
                .sum();
            near(signed_distance, 0.);
            let topology = &model.snapshot().shapes[&request.feature].topology;
            assert!(
                matches!(references::resolve(&retained,topology,reopened.revision()),
                Resolution::Resolved{geometry:Some(AnalyticGeometry::Plane{frame}),..} if frame.normal()==[-1.,0.,0.] && frame.origin_mm[0]==0.)
            );
            for &id in model.snapshot().occurrences.keys() {
                near(
                    model.solid(id).unwrap().volume_mm3().unwrap(),
                    expected_volume,
                );
            }
        }
        if level == 3. {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("drafted-box.step");
            rust_solid.write_step(&path).unwrap();
            let tolerance = GeometryPolicy::default().linear_tolerance_mm;
            let decoded =
                read_analytic_solid(&std::fs::read_to_string(&path).unwrap(), tolerance).unwrap();
            near(decoded.volume_mm3, expected_volume);
            assert_eq!(decoded.face_count, 6);
            let source = rust_solid.topology_snapshot(EntityId::new(), 0).unwrap();
            assert!(Solid::step_topology_matches(&source, &decoded, tolerance));
        }
    }
    let before = session.document().clone();
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Draft(p) = &mut edit.parameters else {
        unreachable!()
    };
    p.angle = model::angle(89.);
    let preview =
        model::prepare_advanced_edit(&session, session.revision(), actor(), &edit).unwrap();
    let error = Compiler::default()
        .compile_with_backend(
            preview.prepared.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(error.entity, Some(request.feature));
    assert!(error.message.contains("crosses the opposite side"));
    assert_eq!(session.document(), &before);
}

#[test]
fn rust_draft_accepts_negative_cardinal_pull_and_refuses_uncertified_faces_or_planes() {
    use multicad_kernel::{GeometryErrorKind, PlaneFrame, TreatmentFace, rust_kernel::Rational};

    let policy = GeometryPolicy::default();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let id = EntityId::new();
    let untouched = stock.topology_snapshot(id, 0).unwrap();
    let face = |ordinal, role: &str| TreatmentFace {
        key: references::TopologyKey {
            kind: references::TopologyKind::Face,
            ordinal,
        },
        origin: Origin {
            source: id,
            role: role.into(),
        },
    };
    let selected = face(4, "draft side");
    let plane = PlaneFrame {
        origin: [3., 0., 0.],
        u: [0., 1., 0.],
        v: [0., 0., -1.],
    };
    let angle = 5_f64.to_radians();
    let result = stock
        .draft_faces_named(
            std::slice::from_ref(&selected),
            [-1., 0., 0.],
            plane,
            angle,
            id,
            policy,
        )
        .unwrap();
    near(result.volume_mm3().unwrap(), 6000. + 3600. * angle.tan());
    near(result.bounds().unwrap().max[1], 20. + 27. * angle.tan());
    let opposite = stock
        .draft_faces_named(
            &[selected.clone(), face(2, "second face")],
            [-1., 0., 0.],
            plane,
            angle,
            id,
            policy,
        )
        .unwrap();
    near(opposite.volume_mm3().unwrap(), 6000. + 7200. * angle.tan());
    near(opposite.bounds().unwrap().min[1], -27. * angle.tan());
    near(opposite.bounds().unwrap().max[1], 20. + 27. * angle.tan());
    for (faces, pull, neutral) in [
        (vec![selected.clone()], [-0.6, 0., 0.8], plane),
        (vec![selected.clone()], [-1., 0., 0.], PlaneFrame::default()),
        (vec![face(3, "cap")], [-1., 0., 0.], plane),
        (
            vec![selected, face(3, "cap after valid side")],
            [-1., 0., 0.],
            plane,
        ),
    ] {
        let error = stock
            .draft_faces_named(&faces, pull, neutral, angle, id, policy)
            .err()
            .unwrap();
        assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    }
    assert_eq!(stock.topology_snapshot(id, 0).unwrap(), untouched);
    near(stock.volume_mm3().unwrap(), 6000.);
}
