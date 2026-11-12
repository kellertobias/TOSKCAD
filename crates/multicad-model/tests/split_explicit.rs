use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Mutation},
    model::{
        self, BodyInput, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        SplitOperationRequest, SplitPlane, SplitSideV2, SplitTool, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::{Compiler, GeometryBackend};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn add(session: &mut Session, edits: Vec<Mutation>) {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Split fixture".into(),
            edits,
        })
        .unwrap();
}
struct Fixture {
    session: Session,
    targets: Vec<BodyInput>,
    tool: BodyInput,
}
fn fixture() -> Fixture {
    let mut session = Session::new(Document::new("Explicit native Split").unwrap()).unwrap();
    let mut targets = vec![];
    let mut tool = None;
    for (name, size, origin) in [
        ("Left target", [10.; 3], [0.; 3]),
        ("Right target", [10.; 3], [20., 0., 0.]),
        ("Shell cutter", [40., 12., 12.], [-1., 4., -1.]),
    ] {
        let request = PrimitiveRequest::new(
            name,
            PrimitiveParameters::Box {
                size: size.map(length),
                frame: PrimitiveFrame {
                    origin: origin.map(length),
                    ..Default::default()
                },
            },
        );
        let prepared =
            model::prepare_primitive(&session, session.revision(), actor(), &request).unwrap();
        session.commit(prepared.prepared).unwrap();
        let input = BodyInput {
            body: request.body,
            producer: request.feature,
        };
        if name == "Shell cutter" {
            tool = Some(input);
        } else {
            targets.push(input);
        }
    }
    Fixture {
        session,
        targets,
        tool: tool.unwrap(),
    }
}
fn plane(z: f64) -> SplitTool {
    SplitTool::Plane {
        source: SplitPlane::Authored {
            frame: Box::new(PrimitiveFrame {
                origin: [length(0.), length(0.), length(z)],
                ..Default::default()
            }),
        },
    }
}
#[test]
fn multiple_targets_split_by_an_authored_plane_apply_once_and_reopen_with_positive_negative_identity()
 {
    let mut f = fixture();
    let request = SplitOperationRequest::new(f.targets.clone(), plane(4.), false);
    let before = f.session.document().clone();
    let revision = f.session.revision();
    let prepared = model::prepare_split_operation(&f.session, revision, actor(), &request).unwrap();
    let native = Compiler::default()
        .compile(prepared.prepared.document(), revision + 1)
        .unwrap();
    assert_eq!(
        native.snapshot().bodies.len(),
        5,
        "four split outputs and the unrelated shell cutter"
    );
    for target in &request.targets {
        for output in &target.outputs {
            let solid = native.solid(output.body).unwrap();
            near(
                solid.volume_mm3().unwrap(),
                if output.side == SplitSideV2::PositivePlane {
                    600.
                } else {
                    400.
                },
            );
            assert_eq!(solid.solid_count().unwrap(), 1);
            assert_eq!(
                model::read_split_v2(prepared.prepared.document(), output.feature)
                    .unwrap()
                    .target,
                target.input
            );
        }
    }
    assert_eq!(f.session.document(), &before);
    f.session.commit(prepared.prepared).unwrap();
    assert_eq!(f.session.revision(), revision + 1);
    let saved = f.session.document().clone();
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &saved);
    let file = ProjectFile::new(saved.clone()).unwrap();
    assert!(file.capabilities().contains(model::SPLIT_V2_CAPABILITY));
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut &bytes[..],
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    assert_eq!(reopened.document, saved);
    let native = Compiler::default().compile(&reopened.document, 0).unwrap();
    for target in &request.targets {
        for output in &target.outputs {
            near(
                native.solid(output.body).unwrap().volume_mm3().unwrap(),
                if output.side == SplitSideV2::PositivePlane {
                    600.
                } else {
                    400.
                },
            );
        }
    }
    let mut legacy = project_file::supported_capabilities();
    legacy.remove(model::SPLIT_V2_CAPABILITY);
    assert!(project_file::read(&mut &bytes[..], Default::default(), &legacy).is_err());
}
#[test]
fn a_closed_model_partitions_every_target_at_its_real_boundary_and_keep_tool_is_explicit() {
    for keep_tools in [false, true] {
        let f = fixture();
        let request = SplitOperationRequest::new(
            f.targets.clone(),
            SplitTool::Body {
                input: f.tool.clone(),
            },
            keep_tools,
        );
        let prepared =
            model::prepare_split_operation(&f.session, f.session.revision(), actor(), &request)
                .unwrap();
        let document = prepared.prepared.document();
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(document.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        assert_eq!(&reopened.document, document);
        for backend in [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ] {
            let result = Compiler::default()
                .compile_with_backend(&reopened.document, f.session.revision() + 1, backend, None)
                .unwrap();
            assert_eq!(result.snapshot().bodies.len(), 4 + usize::from(keep_tools));
            for target in &request.targets {
                for output in &target.outputs {
                    let solid = result.solid(output.body).unwrap();
                    assert_eq!(solid.uses_rust_geometry(), backend == GeometryBackend::Rust);
                    assert_eq!(solid.solid_count().unwrap(), 1);
                    let inside = output.side == SplitSideV2::InsideTool;
                    near(
                        solid.volume_mm3().unwrap(),
                        if inside { 600. } else { 400. },
                    );
                    let bounds = solid.bounds().unwrap();
                    near(if inside { bounds.min[1] } else { bounds.max[1] }, 4.);
                    if backend == GeometryBackend::Rust {
                        let topology = &result.snapshot().shapes[&output.feature].topology;
                        assert!(topology.elements.iter().any(|element| {
                            element.key.kind == multicad_core::references::TopologyKind::Face
                                && element
                                    .origins
                                    .iter()
                                    .any(|origin| origin.source == f.tool.producer)
                        }));
                    }
                }
            }
        }
    }
}
#[test]
fn one_invalid_target_rejects_the_complete_native_candidate_without_committing_any_output() {
    let f = fixture();
    let before = f.session.document().clone();
    let request = SplitOperationRequest::new(
        f.targets.clone(),
        SplitTool::Plane {
            source: SplitPlane::Authored {
                frame: Box::new(PrimitiveFrame {
                    origin: [length(5.), length(0.), length(0.)],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                }),
            },
        },
        false,
    );
    let prepared =
        model::prepare_split_operation(&f.session, f.session.revision(), actor(), &request)
            .unwrap();
    assert!(
        Compiler::default()
            .compile(prepared.prepared.document(), f.session.revision() + 1)
            .is_err(),
        "the second target does not cross x=5"
    );
    assert_eq!(f.session.document(), &before);
    assert!(
        request
            .targets
            .iter()
            .flat_map(|target| &target.outputs)
            .all(|output| f.session.document().get(output.body).is_none())
    );
}
#[test]
fn canonical_and_parameter_bound_planes_rebuild_saved_sides_without_swapping_output_ids() {
    use multicad_core::{constraint_sketch::Dimension, parameters::BoundExpression};
    let mut f = fixture();
    let parameter = Entity::new(
        "cut_height",
        EntityData::Parameter {
            expression: "4".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let parameter_id = parameter.id;
    add(&mut f.session, vec![Mutation::Insert(parameter)]);
    let offset = Dimension::Expression {
        expression: BoundExpression::bind(f.session.document(), "=$cut_height", "mm", None)
            .unwrap(),
    };
    let authored = SplitTool::Plane {
        source: SplitPlane::Authored {
            frame: Box::new(PrimitiveFrame {
                origin: [length(0.), length(0.), offset.clone()],
                ..Default::default()
            }),
        },
    };
    let construction = Entity::new(
        "Construction plane",
        EntityData::Plane {
            origin_mm: [0.; 3],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    let plane_id = construction.id;
    add(&mut f.session, vec![Mutation::Insert(construction)]);
    for tool in [
        authored,
        SplitTool::Plane {
            source: SplitPlane::Existing {
                plane: plane_id,
                offset,
            },
        },
    ] {
        let mut session = Session::new(f.session.document().clone()).unwrap();
        let request = SplitOperationRequest::new(f.targets.clone(), tool, false);
        let prepared =
            model::prepare_split_operation(&session, session.revision(), actor(), &request)
                .unwrap();
        session.commit(prepared.prepared).unwrap();
        for value in ["4", "6"] {
            let mut parameter = session.document().get(parameter_id).unwrap().clone();
            let EntityData::Parameter { expression, .. } = &mut parameter.data else {
                panic!()
            };
            *expression = value.into();
            add(&mut session, vec![Mutation::Replace(parameter)]);
            let native = Compiler::default()
                .compile(session.document(), session.revision())
                .unwrap();
            for target in &request.targets {
                for output in &target.outputs {
                    let height = value.parse::<f64>().unwrap();
                    near(
                        native.solid(output.body).unwrap().volume_mm3().unwrap(),
                        if output.side == SplitSideV2::PositivePlane {
                            (10. - height) * 100.
                        } else {
                            height * 100.
                        },
                    );
                    let source = model::read_split_v2(session.document(), output.feature).unwrap();
                    assert!(
                        source
                            .inputs()
                            .unwrap()
                            .iter()
                            .any(|link| link.target == parameter_id)
                    );
                }
            }
        }
    }
}

#[test]
fn saved_face_offset_plane_keeps_authored_side_frames_after_edit() {
    use multicad_core::{
        constraint_sketch::Dimension,
        document::Link,
        parameters::BoundExpression,
        references::{AnalyticGeometry, Attachment, GeometryIntent, Origin},
        sketch::OFFSET_PLANE_ROLE,
    };

    let mut f = fixture();
    let compiler = Compiler {
        omit_display: true,
        ..Default::default()
    };
    let stock = compiler
        .compile_with_backend(
            f.session.document(),
            f.session.revision(),
            GeometryBackend::Rust,
            None,
        )
        .unwrap();
    let target = f.targets[0].clone();
    let topology = &stock.snapshot().shapes[&target.producer].topology;
    let anchor = Origin {
        source: target.producer,
        role: "cap.end".into(),
    };
    let cap = topology
        .elements
        .iter()
        .find(|element| {
            element.origins.contains(&anchor)
                && matches!(element.geometry, Some(AnalyticGeometry::Plane { .. }))
        })
        .unwrap();
    let mut intent = GeometryIntent::capture(topology, cap.key).unwrap();
    intent.attachment = Some(Attachment {
        anchor,
        offset_mm: [0.; 3],
        angle_rad: 0.,
    });
    let face = Entity::new(
        "Top face datum",
        EntityData::Plane {
            origin_mm: [0., 0., 10.],
            normal: [0., 0., 1.],
            anchor: Some(Link {
                target: target.producer,
                role: "face_attachment".into(),
                geometry: Some(intent),
            }),
        },
    );
    let offset = Entity::new(
        "Two millimetres inside the top",
        EntityData::Plane {
            origin_mm: [0., 0., -2.],
            normal: [0., 0., 1.],
            anchor: Some(Link::new(face.id, OFFSET_PLANE_ROLE)),
        },
    );
    let plane = offset.id;
    let parameter = Entity::new(
        "cut_depth",
        EntityData::Parameter {
            expression: "-2".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    let depth = parameter.id;
    add(
        &mut f.session,
        vec![face, offset, parameter]
            .into_iter()
            .map(Mutation::Insert)
            .collect(),
    );
    let request = SplitOperationRequest::new(
        vec![target.clone()],
        SplitTool::Plane {
            source: SplitPlane::Existing {
                plane,
                offset: Dimension::Expression {
                    expression: BoundExpression::bind(
                        f.session.document(),
                        "=$cut_depth",
                        "mm",
                        None,
                    )
                    .unwrap(),
                },
            },
        },
        false,
    );
    let prepared =
        model::prepare_split_operation(&f.session, f.session.revision(), actor(), &request)
            .unwrap();
    f.session.commit(prepared.prepared).unwrap();

    for (value, split_z) in [("-2", 6.), ("-4", 4.)] {
        let mut parameter = f.session.document().get(depth).unwrap().clone();
        let EntityData::Parameter { expression, .. } = &mut parameter.data else {
            unreachable!("cut depth is a parameter");
        };
        *expression = value.into();
        add(&mut f.session, vec![Mutation::Replace(parameter)]);
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(f.session.document().clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut bytes.as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project;
        for backend in [
            GeometryBackend::Rust,
            #[cfg(feature = "occt")]
            GeometryBackend::OpenCascade,
        ] {
            let rebuilt = compiler
                .compile_with_backend(&reopened.document, 0, backend, None)
                .unwrap();
            assert!(rebuilt.snapshot().references.all_resolved());
            let mut sum = 0.;
            for output in &request.targets[0].outputs {
                let result = rebuilt.solid(output.body).unwrap();
                let positive = output.side == SplitSideV2::PositivePlane;
                let volume = result.volume_mm3().unwrap();
                sum += volume;
                near(
                    volume,
                    if positive {
                        (10. - split_z) * 100.
                    } else {
                        split_z * 100.
                    },
                );
                let topology = &rebuilt.snapshot().shapes[&output.feature].topology;
                let role = if positive {
                    "split/plane/positive"
                } else {
                    "split/plane/negative"
                };
                let frames: Vec<_> = topology
                    .elements
                    .iter()
                    .flat_map(|element| &element.frames)
                    .filter(|anchored| {
                        anchored.origin.source == output.feature && anchored.origin.role == role
                    })
                    .map(|anchored| anchored.frame)
                    .collect();
                assert_eq!(frames.len(), 1);
                near(frames[0].origin_mm[2], split_z);
                assert_eq!(
                    frames[0].normal(),
                    [0., 0., if positive { -1. } else { 1. }]
                );
                assert_eq!(rebuilt.snapshot().bodies[&output.body], output.feature);
            }
            near(sum, 1000.);
        }
    }
}

#[test]
fn rust_plane_split_obeys_reversed_axes_and_rejects_ambiguous_cuts() {
    let f = fixture();
    let mut reversed = PrimitiveFrame {
        origin: [0., 0., 4.].map(length),
        u: [0.6, 0.8, 0.],
        v: [0.8, -0.6, 0.],
    };
    let build = |frame| {
        let request = SplitOperationRequest::new(
            vec![f.targets[0].clone()],
            SplitTool::Plane {
                source: SplitPlane::Authored {
                    frame: Box::new(frame),
                },
            },
            false,
        );
        let prepared =
            model::prepare_split_operation(&f.session, f.session.revision(), actor(), &request)
                .unwrap();
        let result = Compiler::default().compile_with_backend(
            prepared.prepared.document(),
            f.session.revision() + 1,
            GeometryBackend::Rust,
            None,
        );
        (request, result)
    };
    let (request, result) = build(reversed.clone());
    let result = result.unwrap();
    for output in &request.targets[0].outputs {
        let positive = output.side == SplitSideV2::PositivePlane;
        let solid = result.solid(output.body).unwrap();
        near(
            solid.volume_mm3().unwrap(),
            if positive { 400. } else { 600. },
        );
        let bounds = solid.bounds().unwrap();
        near(
            if positive {
                bounds.max[2]
            } else {
                bounds.min[2]
            },
            4.,
        );
    }

    reversed.origin[2] = length(1e-8);
    assert!(
        build(reversed)
            .1
            .err()
            .unwrap()
            .message
            .contains("too close")
    );
    let tilted = PrimitiveFrame {
        origin: [0., 0., 4.].map(length),
        v: [0., 0.6, 0.8],
        ..Default::default()
    };
    let (request, result) = build(tilted);
    let result = result.unwrap();
    // y < 0.75*z - 3 inside the 10 mm cube is a triangular prism:
    // 10 * (10 - 4) * 4.5 / 2 = 135 mm³ on the positive side.
    for output in &request.targets[0].outputs {
        near(
            result.solid(output.body).unwrap().volume_mm3().unwrap(),
            if output.side == SplitSideV2::PositivePlane {
                135.
            } else {
                865.
            },
        );
    }
}

#[test]
fn angled_plane_split_keeps_saved_output_ids_through_reopen_undo_and_redo() {
    let mut f = fixture();
    let before = f.session.document().clone();
    let request = SplitOperationRequest::new(
        vec![f.targets[0].clone()],
        SplitTool::Plane {
            source: SplitPlane::Authored {
                frame: Box::new(PrimitiveFrame {
                    origin: [0., 0., 4.].map(length),
                    u: [1., 0., 0.],
                    v: [0., 0.6, 0.8],
                }),
            },
        },
        false,
    );
    let prepared =
        model::prepare_split_operation(&f.session, f.session.revision(), actor(), &request)
            .unwrap();
    f.session.commit(prepared.prepared).unwrap();
    let saved = f.session.document().clone();
    let check = |document, revision| {
        let compiled = Compiler::default()
            .compile_with_backend(document, revision, GeometryBackend::Rust, None)
            .unwrap();
        for output in &request.targets[0].outputs {
            near(
                compiled.solid(output.body).unwrap().volume_mm3().unwrap(),
                if output.side == SplitSideV2::PositivePlane {
                    135.
                } else {
                    865.
                },
            );
            assert_eq!(
                model::read_split_v2(document, output.feature)
                    .unwrap()
                    .target,
                f.targets[0]
            );
        }
    };
    check(&saved, f.session.revision());
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(saved.clone()).unwrap(),
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
    assert_eq!(reopened.document, saved);
    check(&reopened.document, 0);
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before);
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &saved);
    check(f.session.document(), f.session.revision());
}
