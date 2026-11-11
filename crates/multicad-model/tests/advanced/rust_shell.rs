use super::common::*;
use multicad_core::{
    commands::{Request, Session},
    document::{Entity, EntityData, EntityId, Link, Mutation},
    model::{self, AdvancedParameters, AdvancedRequest, ShellParameters},
    project_file::{self, ProjectFile},
    references::{
        self, AnalyticGeometry, Attachment, GeometryIntent, Origin, ReferenceAddress, Resolution,
    },
};
use multicad_kernel::{Bounds, GeometryPolicy, PlaneFrame, Solid, TreatmentFace};
use multicad_model::{Compiler, GeometryBackend, Model};

fn compile_rust(session: &Session) -> Model {
    Compiler::default()
        .compile_with_backend(
            session.document(),
            session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap()
}

fn face_metrics(solid: &Solid) -> Vec<[i64; 8]> {
    let quantize = |value: f64| (value * 1e6).round() as i64;
    let mut result: Vec<_> = solid
        .faces()
        .unwrap()
        .into_iter()
        .map(|face| {
            let (point, normal) = face
                .plane
                .expect("rectangular shell must have planar faces");
            let level = point
                .into_iter()
                .zip(normal)
                .map(|(p, n)| p * n)
                .sum::<f64>();
            let [x, y, z] = normal;
            let [a, b, c] = face.center_mm;
            [x, y, z, level, face.area_mm2, a, b, c].map(quantize)
        })
        .collect();
    result.sort_unstable();
    result
}

fn placed_normal(frame: PlaneFrame, axis: usize, positive: bool) -> [f64; 3] {
    let basis = [frame.u, frame.v, frame.normal()];
    basis[axis].map(|n| if positive { n } else { -n })
}

fn selected_opening(
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

fn shell_volume(size: [f64; 3], thickness: f64, openings: &[(usize, bool)]) -> f64 {
    let offset = std::array::from_fn::<_, 3, _>(|axis| {
        let closed = 2 - openings.iter().filter(|&&(side, _)| side == axis).count();
        size[axis] - closed as f64 * thickness
    })
    .into_iter()
    .product::<f64>();
    let stock = size.into_iter().product::<f64>();
    if thickness > 0. {
        stock - offset
    } else {
        offset - stock
    }
}

fn check_mesh_volume(solid: &Solid, expected: f64) {
    use multicad_kernel::rust_kernel::{display::DisplayMesh, manufacturing::certify};
    let receipt = solid
        .manufacturing_mesh_with_receipt(Default::default())
        .unwrap();
    let proof = certify(&DisplayMesh {
        vertices: receipt.mesh.vertices,
        indices: receipt.mesh.indices,
        face_indices: receipt.mesh.face_indices,
    })
    .unwrap();
    assert!(proof.edge_pairs > 0 && proof.vertex_links > 0);
    near(proof.signed_volume_mm3, expected);
}

#[test]
fn rust_shell_supports_adjacent_and_opposite_openings_with_signed_placed_geometry() {
    #[cfg(feature = "occt")]
    use multicad_kernel::RigidTransform;
    use multicad_kernel::rust_kernel::Rational;

    let policy = GeometryPolicy::default();
    let size = [30., 20., 10.];
    for frame in [
        PlaneFrame::default(),
        PlaneFrame {
            origin: [17., 4., 3.],
            u: [0.8, 0.6, 0.],
            v: [-0.36, 0.48, 0.8],
        },
    ] {
        let stock = Solid::rust_box(
            EntityId::new().uuid_bits(),
            [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
            frame,
            policy,
        )
        .unwrap();
        #[cfg(feature = "occt")]
        let native_stock = Solid::box_solid(size, policy)
            .unwrap()
            .transformed(RigidTransform {
                rotation: std::array::from_fn(|i| [frame.u[i], frame.v[i], frame.normal()[i]]),
                translation_mm: frame.origin,
            })
            .unwrap();
        let feature = EntityId::new();
        for openings in [
            vec![(2, false), (2, true)],
            vec![(0, false), (0, true)],
            vec![(0, true), (2, true)],
            vec![(0, false), (0, true), (1, true), (2, true)],
        ] {
            let selected = openings
                .iter()
                .map(|&(axis, positive)| {
                    selected_opening(
                        &stock,
                        placed_normal(frame, axis, positive),
                        feature,
                        format!("opening/{axis}/{positive}"),
                    )
                })
                .collect::<Vec<_>>();
            for thickness in [2., -2.] {
                let volume = shell_volume(size, thickness, &openings);
                let result = stock
                    .shell_named(&selected, thickness, feature, policy)
                    .unwrap();
                assert!(result.uses_rust_geometry());
                assert_eq!(result.solid_count().unwrap(), 1);
                near(result.volume_mm3().unwrap(), volume);
                check_mesh_volume(&result, volume);
                let topology = result.topology_snapshot(feature, 1).unwrap();
                topology.validate().unwrap();
                for (&(axis, positive), selection) in openings.iter().zip(&selected) {
                    let face = topology
                        .elements
                        .iter()
                        .find(|element| {
                            element.origins.contains(&selection.origin)
                                && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
                        })
                        .unwrap();
                    let Some(AnalyticGeometry::Plane { frame: rim }) = face.geometry else {
                        unreachable!()
                    };
                    let normal = placed_normal(frame, axis, positive);
                    for (actual, expected) in rim.normal().into_iter().zip(normal) {
                        near(actual, expected);
                    }
                    let point = std::array::from_fn::<_, 3, _>(|i| {
                        frame.origin[i]
                            + if positive {
                                [frame.u, frame.v, frame.normal()][axis][i] * size[axis]
                            } else {
                                0.
                            }
                    });
                    near(
                        normal
                            .into_iter()
                            .enumerate()
                            .map(|(i, n)| n * (point[i] - rim.origin_mm[i]))
                            .sum(),
                        0.,
                    );
                }
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join("multi-opening-shell.step");
                result.write_step(&path).unwrap();
                let reopened = Solid::import_rust_step(
                    &std::fs::read_to_string(&path).unwrap(),
                    feature,
                    policy,
                )
                .unwrap();
                near(reopened.volume_mm3().unwrap(), volume);
                assert_eq!(face_metrics(&reopened), face_metrics(&result));
                #[cfg(feature = "occt")]
                {
                    let selected = openings
                        .iter()
                        .map(|&(axis, positive)| {
                            selected_opening(
                                &native_stock,
                                placed_normal(frame, axis, positive),
                                feature,
                                format!("opening/{axis}/{positive}"),
                            )
                        })
                        .collect::<Vec<_>>();
                    let native = native_stock
                        .shell_named(&selected, thickness, feature, policy)
                        .unwrap();
                    near(native.volume_mm3().unwrap(), volume);
                    assert_eq!(face_metrics(&native), face_metrics(&result));
                    check_mesh_volume(&native, volume);
                    let a = native.bounds().unwrap();
                    let b = result.bounds().unwrap();
                    for (a, b) in a
                        .min
                        .into_iter()
                        .chain(a.max)
                        .zip(b.min.into_iter().chain(b.max))
                    {
                        near(a, b);
                    }
                    let readback = Solid::read_step(&path, policy).unwrap();
                    assert_eq!(readback.solid_count().unwrap(), 1);
                    near(readback.volume_mm3().unwrap(), volume);
                    assert_eq!(face_metrics(&readback), face_metrics(&result));
                }
            }
        }
        near(stock.volume_mm3().unwrap(), 6000.);
    }
    assert_eq!(shell_volume(size, 2., &[(2, false), (2, true)]), 1840.);
}

#[test]
fn saved_rust_shell_rebuilds_signed_walls_and_linked_occurrences() {
    let (mut session, input) = fixture();
    let source = compile_rust(&session);
    let original = &source.snapshot().shapes[&input.producer].topology;
    let top = original.elements.iter().find(|element| {
        matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == [0.,0.,1.])
    }).unwrap();
    let opening = GeometryIntent::capture(original, top.key).unwrap();
    let selection_id = opening.id;
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Shell(ShellParameters {
            thickness: bound(&session, "wall", "mm"),
        }),
        vec![opening],
    );
    let p = model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(p.prepared).unwrap();

    let original = compile_rust(&session);
    let topology = &original.snapshot().shapes[&request.feature].topology;
    let opening = Origin {
        source: request.feature,
        role: format!("face/{selection_id}"),
    };
    let offset = Origin {
        source: request.feature,
        role: format!("offset/{}/cap.start", input.producer),
    };
    let find = |origin: &Origin| {
        topology
            .elements
            .iter()
            .find(|item| {
                item.origins.contains(origin)
                    && matches!(item.geometry, Some(AnalyticGeometry::Plane { .. }))
            })
            .unwrap()
    };
    let rim = find(&opening);
    let mut rim_intent = GeometryIntent::capture(topology, rim.key).unwrap();
    rim_intent.attachment = Some(Attachment {
        anchor: opening,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let reference = rim_intent.id;
    let datum = Entity::new(
        "Opening datum",
        EntityData::Plane {
            origin_mm: [0., 0., 10.],
            normal: [0., 0., 1.],
            anchor: Some(Link {
                target: request.feature,
                role: "face_attachment".into(),
                geometry: Some(rim_intent),
            }),
        },
    );
    let owner = datum.id;
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach opening datum".into(),
            edits: vec![Mutation::Insert(datum)],
        })
        .unwrap();

    let mut offset_intent = GeometryIntent::capture(topology, find(&offset).key).unwrap();
    offset_intent.attachment = Some(Attachment {
        anchor: offset,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });

    let retained = [
        references::TopologyKind::Face,
        references::TopologyKind::Edge,
        references::TopologyKind::Vertex,
    ]
    .map(|kind| {
        let element = topology
            .elements
            .iter()
            .find(|e| {
                e.key.kind == kind
                    && match e.geometry {
                        Some(AnalyticGeometry::Plane { frame }) => {
                            frame.normal() == [0., 0., -1.] && frame.origin_mm[2] == 0.
                        }
                        Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                            [a_mm, b_mm] == [[0.; 3], [0., 0., 10.]]
                                || [b_mm, a_mm] == [[0.; 3], [0., 0., 10.]]
                        }
                        Some(AnalyticGeometry::Point { position_mm }) => position_mm == [0.; 3],
                        _ => false,
                    }
            })
            .unwrap();
        GeometryIntent::capture(topology, element.key).unwrap()
    });

    for (width, thickness, volume, min, max) in [
        (30., 2., 2672., [0.; 3], [30., 20., 10.]),
        (40., 2., 3392., [0.; 3], [40., 20., 10.]),
        (40., -2., 4672., [-2.; 3], [42., 22., 10.]),
    ] {
        apply_parameter(&mut session, "width", &width.to_string());
        apply_parameter(&mut session, "wall", &thickness.to_string());
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let doc = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        let reopened = Session::new(doc).unwrap();
        let rebuilt = compile_rust(&reopened);
        let solid = rebuilt.solid(input.body).unwrap();
        assert!(solid.uses_rust_geometry());
        assert_eq!(solid.bounds().unwrap(), Bounds { min, max });
        let expected = face_metrics(solid);
        assert_eq!(expected.len(), 11);
        assert_eq!(solid.solid_count().unwrap(), 1);
        near(solid.volume_mm3().unwrap(), volume);
        assert_eq!(rebuilt.snapshot().occurrences.len(), 1);
        for &id in rebuilt.snapshot().occurrences.keys() {
            let linked = rebuilt.solid(id).unwrap();
            near(linked.volume_mm3().unwrap(), volume);
            assert_eq!(linked.bounds().unwrap().min[0], min[0] + 80.);
        }
        let frames = |model: &Model| {
            let refs = &model.snapshot().references;
            assert!(refs.all_resolved());
            let Resolution::Resolved {
                frame: Some(rim), ..
            } = refs
                .get(ReferenceAddress { owner, reference })
                .unwrap()
                .resolution
            else {
                panic!("missing opening datum");
            };
            assert_eq!(rim.normal(), [0., 0., 1.]);
            near(rim.origin_mm[2], 10.);
            let topology = &model.snapshot().shapes[&request.feature].topology;
            let Resolution::Resolved {
                frame: Some(base), ..
            } = references::resolve(&offset_intent, topology, reopened.revision())
            else {
                panic!("missing named offset face");
            };
            for intent in &retained {
                let resolved = references::resolve(intent, topology, reopened.revision());
                assert!(
                    matches!(resolved, Resolution::Resolved { .. }),
                    "{intent:?}: {resolved:?}"
                );
            }
            near(base.origin_mm[2], thickness);
            assert_eq!(base.normal(), [0., 0., thickness.signum()]);
        };
        frames(&rebuilt);
        if cfg!(feature = "occt") {
            let native = Compiler::default()
                .compile_with_backend(
                    reopened.document(),
                    reopened.revision(),
                    GeometryBackend::OpenCascade,
                    None,
                )
                .unwrap();
            let native_solid = native.solid(input.body).unwrap();
            assert_eq!(face_metrics(native_solid), expected);
            near(native_solid.volume_mm3().unwrap(), volume);
            assert_eq!(native_solid.solid_count().unwrap(), 1);
            frames(&native);
        }
        if thickness < 0. {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("rectangular-shell.step");
            solid.write_step(&path).unwrap();
            let reopened = Solid::import_rust_step(
                &std::fs::read_to_string(&path).unwrap(),
                EntityId::new(),
                GeometryPolicy::default(),
            )
            .unwrap();
            assert_eq!(face_metrics(&reopened), expected);
            near(reopened.volume_mm3().unwrap(), volume);
        }
    }
    let before = session.document().clone();
    let revision = session.revision();
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    let AdvancedParameters::Shell(p) = &mut edit.parameters else {
        unreachable!()
    };
    p.thickness = model::length(1000.);
    let candidate = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
    let failure = Compiler::default()
        .compile_with_backend(
            candidate.prepared.document(),
            revision,
            GeometryBackend::Rust,
            None,
        )
        .err()
        .unwrap();
    assert_eq!(failure.entity, Some(request.feature));
    assert!(failure.message.contains("leaves no cavity"));
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}

#[test]
fn saved_two_opening_shell_preserves_rims_offset_refs_undo_and_redo() {
    let (mut session, input) = fixture();
    let source = compile_rust(&session);
    let original = &source.snapshot().shapes[&input.producer].topology;
    let all_openings = [
        [-1., 0., 0.], [1., 0., 0.], [0., -1., 0.], [0., 1., 0.], [0., 0., -1.], [0., 0., 1.],
    ].map(|normal| {
        let face = original.elements.iter().find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame }) if frame.normal() == normal)
        }).unwrap();
        GeometryIntent::capture(original, face.key).unwrap()
    });
    let request = AdvancedRequest::on_body(
        input.clone(),
        AdvancedParameters::Shell(ShellParameters {
            thickness: bound(&session, "wall", "mm"),
        }),
        all_openings[4..].to_vec(),
    );
    let prepared =
        model::prepare_advanced(&session, session.revision(), actor(), &request).unwrap();
    session.commit(prepared.prepared).unwrap();
    let initial = compile_rust(&session);
    let topology = &initial.snapshot().shapes[&request.feature].topology;
    let datum = |element: &references::TopologyElement, anchor: Origin| {
        let Some(AnalyticGeometry::Plane { frame }) = element.geometry else {
            unreachable!()
        };
        let mut intent = GeometryIntent::capture(topology, element.key).unwrap();
        let reference = intent.id;
        intent.attachment = Some(Attachment {
            anchor,
            offset_mm: [0.; 3],
            angle_rad: 0.,
        });
        (
            Entity::new(
                "Shell datum",
                EntityData::Plane {
                    origin_mm: frame.origin_mm,
                    normal: frame.normal(),
                    anchor: Some(Link {
                        target: request.feature,
                        role: "face_attachment".into(),
                        geometry: Some(intent),
                    }),
                },
            ),
            reference,
        )
    };
    let mut edits = Vec::new();
    let mut rims = Vec::new();
    for (end, selection) in all_openings[4..].iter().enumerate() {
        let anchor = Origin {
            source: request.feature,
            role: format!("face/{}", selection.id),
        };
        let face = topology
            .elements
            .iter()
            .find(|element| {
                element.origins.contains(&anchor)
                    && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
            })
            .unwrap();
        let (entity, reference) = datum(face, anchor);
        rims.push((entity.id, reference, end == 1));
        edits.push(Mutation::Insert(entity));
    }
    let offset = topology
        .elements
        .iter()
        .find(|element| {
            matches!(element.geometry, Some(AnalyticGeometry::Plane { frame })
            if frame.normal() == [0., 1., 0.] && frame.origin_mm[1] == 2.)
        })
        .unwrap();
    let anchor = offset
        .origins
        .iter()
        .find(|origin| origin.source == request.feature && origin.role.starts_with("offset/"))
        .unwrap()
        .clone();
    let (offset, offset_reference) = datum(offset, anchor);
    let offset_owner = offset.id;
    edits.push(Mutation::Insert(offset));
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Attach both rims and an offset wall".into(),
            edits,
        })
        .unwrap();
    let retained = [
        references::TopologyKind::Face,
        references::TopologyKind::Edge,
        references::TopologyKind::Vertex,
    ]
    .map(|kind| {
        let element = topology
            .elements
            .iter()
            .find(|element| {
                element.key.kind == kind
                    && match element.geometry {
                        Some(AnalyticGeometry::Plane { frame }) => {
                            frame.normal() == [-1., 0., 0.] && frame.origin_mm[0] == 0.
                        }
                        Some(AnalyticGeometry::Line { a_mm, b_mm }) => {
                            [a_mm, b_mm] == [[0.; 3], [0., 0., 10.]]
                                || [b_mm, a_mm] == [[0.; 3], [0., 0., 10.]]
                        }
                        Some(AnalyticGeometry::Point { position_mm }) => position_mm == [0.; 3],
                        _ => false,
                    }
            })
            .unwrap();
        GeometryIntent::capture(topology, element.key).unwrap()
    });
    for (width, thickness, volume) in [(30., 2_f64, 1840.), (40., 2., 2240.), (40., -2., 2560.)] {
        apply_parameter(&mut session, "width", &width.to_string());
        apply_parameter(&mut session, "wall", &thickness.to_string());
        if thickness < 0. {
            let document = session.document().clone();
            session.undo(session.revision(), actor()).unwrap();
            near(
                compile_rust(&session)
                    .solid(input.body)
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                2240.,
            );
            session.redo(session.revision(), actor()).unwrap();
            assert_eq!(session.document(), &document);
            near(
                compile_rust(&session)
                    .solid(input.body)
                    .unwrap()
                    .volume_mm3()
                    .unwrap(),
                volume,
            );
        }
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
        let rust = compile_rust(&reopened);
        let solid = rust.solid(input.body).unwrap();
        near(solid.volume_mm3().unwrap(), volume);
        check_mesh_volume(solid, volume);
        for backend in [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ] {
            let evaluated = Compiler::default()
                .compile_with_backend(reopened.document(), reopened.revision(), backend, None)
                .unwrap();
            let actual = evaluated.solid(input.body).unwrap();
            near(actual.volume_mm3().unwrap(), volume);
            assert_eq!(face_metrics(actual), face_metrics(solid));
            let report = &evaluated.snapshot().references;
            assert!(report.all_resolved(), "{report:?}");
            for &(owner, reference, end) in &rims {
                let Resolution::Resolved {
                    frame: Some(frame), ..
                } = report
                    .get(ReferenceAddress { owner, reference })
                    .unwrap()
                    .resolution
                else {
                    panic!("An opening rim lost its saved datum");
                };
                assert_eq!(frame.normal(), [0., 0., if end { 1. } else { -1. }]);
                near(frame.origin_mm[2], if end { 10. } else { 0. });
            }
            let Resolution::Resolved {
                frame: Some(frame), ..
            } = report
                .get(ReferenceAddress {
                    owner: offset_owner,
                    reference: offset_reference,
                })
                .unwrap()
                .resolution
            else {
                panic!("The signed offset wall lost its saved datum");
            };
            assert_eq!(frame.normal(), [0., thickness.signum(), 0.]);
            near(frame.origin_mm[1], thickness);
            let topology = &evaluated.snapshot().shapes[&request.feature].topology;
            for intent in &retained {
                assert!(matches!(
                    references::resolve(intent, topology, reopened.revision()),
                    Resolution::Resolved { .. }
                ));
            }
            for &instance in evaluated.snapshot().occurrences.keys() {
                near(
                    evaluated.solid(instance).unwrap().volume_mm3().unwrap(),
                    volume,
                );
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("saved-two-opening-shell.step");
        solid.write_step(&path).unwrap();
        near(
            multicad_model::export::verify_rust_step_shape(
                &path,
                solid,
                &rust.snapshot().shapes[&request.feature],
                1e-7,
            )
            .unwrap(),
            volume,
        );
    }
    let before = session.document().clone();
    let revision = session.revision();
    for openings in [&all_openings[..4], &all_openings[..]] {
        let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
        edit.faces = openings.to_vec();
        let candidate = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
        let failure = Compiler::default()
            .compile_with_backend(
                candidate.prepared.document(),
                revision,
                GeometryBackend::Rust,
                None,
            )
            .err()
            .unwrap();
        assert_eq!(failure.entity, Some(request.feature));
        assert!(failure.message.contains("one connected solid"));
        assert_eq!(session.document(), &before);
        assert_eq!(session.revision(), revision);
    }
    // A valid plate can delete a selected rim. The shared canonical report
    // must identify its saved datum as Missing, even with parallel faces left.
    let mut edit = AdvancedRequest::existing(session.document(), request.feature).unwrap();
    edit.faces = all_openings[..4]
        .iter()
        .chain([&all_openings[5]])
        .cloned()
        .collect();
    let candidate = model::prepare_advanced_edit(&session, revision, actor(), &edit).unwrap();
    let parents = Compiler::default()
        .compile_sources_with_backend(
            candidate.prepared.document(),
            revision,
            &[request.feature],
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    near(
        parents.solid(input.body).unwrap().volume_mm3().unwrap(),
        1600.,
    );
    let topologies = parents
        .snapshot()
        .shapes
        .iter()
        .map(|(id, shape)| (*id, shape.topology.clone()))
        .collect();
    let report = references::inspect(candidate.prepared.document(), revision, &topologies);
    let &(owner, reference, _) = rims.iter().find(|(_, _, end)| *end).unwrap();
    assert_eq!(
        report
            .get(ReferenceAddress { owner, reference })
            .unwrap()
            .resolution,
        Resolution::Missing
    );
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
}

#[test]
fn rust_shell_supports_placed_side_and_adjacent_openings_and_rejects_curved_openings() {
    use multicad_kernel::{GeometryErrorKind, PlaneFrame, TreatmentFace, rust_kernel::Rational};

    let policy = GeometryPolicy::default();
    let n = |value| Rational::new(value, 1).unwrap();
    let id = EntityId::new();
    let frame = PlaneFrame {
        origin: [17., 4., 3.],
        u: [0.6, 0.8, 0.],
        v: [-0.8, 0.6, 0.],
    };
    let solid = Solid::rust_box(id.uuid_bits(), [30, 20, 10].map(n), frame, policy).unwrap();
    let feature = EntityId::new();
    let selection = |ordinal, role| TreatmentFace {
        key: references::TopologyKey {
            kind: references::TopologyKind::Face,
            ordinal,
        },
        origin: Origin {
            source: feature,
            role,
        },
    };
    let cap = selection(5, "side-opening".into());
    for (wall, volume) in [(2., 3312.), (-2., 4752.)] {
        let result = solid
            .shell_named(std::slice::from_ref(&cap), wall, feature, policy)
            .unwrap();
        near(result.volume_mm3().unwrap(), volume);
        let snapshot = result.topology_snapshot(feature, 1).unwrap();
        let opening = snapshot
            .elements
            .iter()
            .find(|e| {
                matches!(e.geometry, Some(AnalyticGeometry::Plane { .. }))
                    && e.origins.contains(&cap.origin)
            })
            .unwrap();
        let Some(AnalyticGeometry::Plane { frame: rim }) = opening.geometry else {
            unreachable!()
        };
        for (actual, expected) in rim.normal().into_iter().zip([-0.6, -0.8, 0.]) {
            near(actual, expected);
        }
        let gap = std::array::from_fn::<_, 3, _>(|i| rim.origin_mm[i] - frame.origin[i])
            .into_iter()
            .zip(rim.normal())
            .map(|(a, b)| a * b)
            .sum();
        near(gap, 0.);
    }
    for (wall, volume) in [(2., 2416.), (-2., 3216.)] {
        let multiple = solid
            .shell_named(
                &[cap.clone(), selection(1, "top-opening".into())],
                wall,
                feature,
                policy,
            )
            .unwrap();
        near(multiple.volume_mm3().unwrap(), volume);
        assert_eq!(multiple.solid_count().unwrap(), 1);
    }
    let curved = Solid::rust_sphere_named(4., PlaneFrame::default(), id, policy).unwrap();
    let error = curved
        .shell_named(
            &[selection(0, "curved-opening".into())],
            2.,
            feature,
            policy,
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, GeometryErrorKind::Unsupported);
    assert!(error.message.contains("one rectangular prism"));
}

#[test]
fn rust_shell_rejects_disconnected_plates_empty_material_and_invalid_selection_atomically() {
    use multicad_kernel::{GeometryErrorKind, rust_kernel::Rational};

    let policy = GeometryPolicy::default();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let feature = EntityId::new();
    let before = stock.topology_snapshot(feature, 0).unwrap();
    for openings in [
        vec![(0, false), (0, true), (1, false), (1, true)],
        vec![
            (0, false),
            (0, true),
            (1, false),
            (1, true),
            (2, false),
            (2, true),
        ],
    ] {
        let selected = openings
            .into_iter()
            .map(|(axis, positive)| {
                selected_opening(
                    &stock,
                    placed_normal(PlaneFrame::default(), axis, positive),
                    feature,
                    format!("opening/{axis}/{positive}"),
                )
            })
            .collect::<Vec<_>>();
        for thickness in [2., -2.] {
            let failure = stock
                .shell_named(&selected, thickness, feature, policy)
                .err()
                .unwrap();
            assert_eq!(failure.kind, GeometryErrorKind::InvalidResult);
            assert!(failure.message.contains("one connected solid"));
        }
    }
    let face = selected_opening(&stock, [0., 0., 1.], feature, "opening".into());
    let duplicate = TreatmentFace {
        origin: Origin {
            source: feature,
            role: "duplicate".into(),
        },
        ..face.clone()
    };
    assert_eq!(
        stock
            .shell_named(&[face.clone(), duplicate], 2., feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Input
    );
    assert_eq!(
        stock
            .shell_named(&[], 2., feature, policy)
            .err()
            .unwrap()
            .kind,
        GeometryErrorKind::Unsupported
    );
    let crossed = stock
        .shell_named(&[face], 1000., feature, policy)
        .err()
        .unwrap();
    assert_eq!(crossed.kind, GeometryErrorKind::InvalidResult);
    assert!(crossed.message.contains("leaves no cavity"));
    assert_eq!(stock.topology_snapshot(feature, 0).unwrap(), before);
    near(stock.volume_mm3().unwrap(), 6000.);
}

#[test]
fn five_openings_leave_one_plate_and_report_the_absent_rim_as_missing() {
    use multicad_kernel::rust_kernel::Rational;
    let policy = GeometryPolicy::default();
    let stock = Solid::rust_box(
        EntityId::new().uuid_bits(),
        [30, 20, 10].map(|n| Rational::new(n, 1).unwrap()),
        PlaneFrame::default(),
        policy,
    )
    .unwrap();
    let feature = EntityId::new();
    let selected =
        [(0, false), (0, true), (1, false), (1, true), (2, true)].map(|(axis, positive)| {
            selected_opening(
                &stock,
                placed_normal(PlaneFrame::default(), axis, positive),
                feature,
                format!("opening/{axis}/{positive}"),
            )
        });
    let original = stock.topology_snapshot(feature, 0).unwrap();
    let removed_top = GeometryIntent::capture(&original, selected[4].key).unwrap();
    for thickness in [2_f64, -2.] {
        let plate = stock
            .shell_named(&selected, thickness, feature, policy)
            .unwrap();
        near(plate.volume_mm3().unwrap(), 1200.);
        assert_eq!(plate.solid_count().unwrap(), 1);
        assert_eq!(plate.faces().unwrap().len(), 6);
        let bounds = plate.bounds().unwrap();
        near(bounds.min[2], thickness.min(0.));
        near(bounds.max[2], thickness.max(0.));
        check_mesh_volume(&plate, 1200.);
        let topology = plate.topology_snapshot(feature, 1).unwrap();
        assert_eq!(
            references::resolve(&removed_top, &topology, 1),
            Resolution::Missing
        );
        assert!(
            !topology
                .elements
                .iter()
                .any(|element| element.origins.contains(&selected[4].origin))
        );
        for opening in &selected[..4] {
            assert!(
                topology
                    .elements
                    .iter()
                    .any(|element| element.origins.contains(&opening.origin))
            );
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("one-wall-plate.step");
        plate.write_step(&path).unwrap();
        let readback =
            Solid::import_rust_step(&std::fs::read_to_string(&path).unwrap(), feature, policy)
                .unwrap();
        assert_eq!(face_metrics(&readback), face_metrics(&plate));
        #[cfg(feature = "occt")]
        {
            let native = Solid::read_step(&path, policy).unwrap();
            assert_eq!(native.solid_count().unwrap(), 1);
            near(native.volume_mm3().unwrap(), 1200.);
            assert_eq!(face_metrics(&native), face_metrics(&plate));
        }
    }
}
