use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Mutation},
    model::{
        self, BodyInput, PrimitiveFrame, PrimitiveParameters, PrimitiveRequest,
        SplitOperationRequest, SplitPlane, SplitSideV2, SplitTool, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_model::Compiler;
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
            frame: PrimitiveFrame {
                origin: [length(0.), length(0.), length(z)],
                ..Default::default()
            },
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
        let native = Compiler::default()
            .compile(prepared.prepared.document(), f.session.revision() + 1)
            .unwrap();
        assert_eq!(native.snapshot().bodies.len(), 4 + usize::from(keep_tools));
        for target in &request.targets {
            for output in &target.outputs {
                let solid = native.solid(output.body).unwrap();
                let inside = output.side == SplitSideV2::InsideTool;
                near(
                    solid.volume_mm3().unwrap(),
                    if inside { 600. } else { 400. },
                );
                if inside {
                    near(solid.bounds().unwrap().min[1], 4.);
                } else {
                    near(solid.bounds().unwrap().max[1], 4.);
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
                frame: PrimitiveFrame {
                    origin: [length(5.), length(0.), length(0.)],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                },
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
            frame: PrimitiveFrame {
                origin: [length(0.), length(0.), offset.clone()],
                ..Default::default()
            },
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
