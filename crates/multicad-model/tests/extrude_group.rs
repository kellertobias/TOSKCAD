use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, BodyInput, Extent, ExtrudeBooleanRequest, ExtrudeGroupDraft, ExtrudeGroupEdit,
        ExtrudeGroupTarget, ExtrudeParameters, ExtrudeRequest, Operation, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, Termination, length,
    },
    parameters::BoundExpression,
    project_file::{self, FileLimits, ProjectFile},
    sketch::Datum,
    sketch_regions,
};
use multicad_model::{Compiler, Model};
use std::io::Cursor;

fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
fn apply(session: &mut Session, edits: Vec<Mutation>) {
    session
        .apply(Request {
            expected_revision: session.revision(),
            actor: actor(),
            label: "Group integration fixture edit".into(),
            edits,
        })
        .unwrap();
}
fn compile(session: &Session) -> Model {
    Compiler::default()
        .compile(session.document(), session.revision())
        .unwrap()
}
fn reopen(session: &Session) -> Session {
    let mut bytes = vec![];
    let project = ProjectFile::new(session.document().clone()).unwrap();
    project_file::write(&project, &mut bytes, FileLimits::default()).unwrap();
    let opened = project_file::read(
        &mut Cursor::new(bytes),
        FileLimits::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(&opened.project.document, session.document());
    Session::new(opened.project.document).unwrap()
}
fn body(request: &PrimitiveRequest) -> BodyInput {
    BodyInput {
        body: request.body,
        producer: request.feature,
    }
}
struct Fixture {
    session: Session,
    targets: [PrimitiveRequest; 3],
    depth: EntityId,
    request: ExtrudeBooleanRequest,
    group: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let (sketch, edits) = Source::create(Datum::Xy, "Cut profiles").unwrap();
        let mut document = Document::new("Editable extrusion group")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&document, sketch).unwrap().0;
        source.add_rectangle([8., -1.], [12., 31.]).unwrap();
        let region = sketch_regions::extract(&source).regions[0].intent();
        document = document
            .edited(&source.replace(&document, sketch).unwrap())
            .unwrap();
        let depth = Entity::new(
            "cut_depth",
            EntityData::Parameter {
                expression: "10".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        document = document.edited(&[Mutation::Insert(depth.clone())]).unwrap();
        let mut session = Session::new(document).unwrap();
        let targets = [[0., 0., 0.], [0., 20., 0.], [30., 0., 0.]].map(|origin| {
            let request = PrimitiveRequest::new(
                "Stock",
                PrimitiveParameters::Box {
                    size: [length(10.), length(10.), length(10.)],
                    frame: PrimitiveFrame {
                        origin: origin.map(length),
                        ..Default::default()
                    },
                },
            );
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor(), &request).unwrap();
            session.commit(prepared.prepared).unwrap();
            request
        });
        let parameters = ExtrudeParameters {
            extent: Extent::OneSided {
                end: Termination::Distance {
                    distance: Dimension::Expression {
                        expression: BoundExpression::bind(
                            session.document(),
                            "=$cut_depth",
                            "mm",
                            None,
                        )
                        .unwrap(),
                    },
                },
            },
            ..Default::default()
        };
        let extrusion = ExtrudeRequest::new(sketch, vec![region], parameters);
        let mut request =
            ExtrudeBooleanRequest::new(extrusion, Operation::Cut, vec![body(&targets[0])], false);
        let group = ExtrudeGroupDraft::new("Shared profile cuts", None);
        let group_id = group.id;
        request.group = Some(group);
        let original = session.document().clone();
        let prepared =
            model::prepare_extrude_boolean(&session, session.revision(), actor(), &request)
                .unwrap();
        let candidate = Compiler::default()
            .compile(prepared.prepared.document(), session.revision() + 1)
            .unwrap();
        near(
            candidate.snapshot().shapes[&request.booleans[0].feature].volume_mm3,
            800.,
        );
        assert_eq!(session.document(), &original);
        session.commit(prepared.prepared).unwrap();
        Self {
            session,
            targets,
            depth: depth.id,
            request,
            group: group_id,
        }
    }
}

#[test]
fn persisted_group_edits_rebuild_exact_solids_and_change_explicit_targets_atomically() {
    let mut f = Fixture::new();
    let initial = compile(&f.session);
    let first_boolean = f.request.booleans[0].feature;
    let tool = f.request.extrusion.outputs[0].clone();
    assert_eq!(initial.snapshot().bodies.len(), 3);
    assert!(!initial.snapshot().bodies.contains_key(&tool.body));
    assert!(initial.solid(tool.feature).is_some());
    near(initial.snapshot().shapes[&tool.feature].volume_mm3, 1280.);
    near(
        initial.snapshot().shapes[&f.targets[1].feature].volume_mm3,
        1000.,
    );
    let unrelated = f.session.document().get(f.targets[2].body).unwrap().clone();
    f.session = reopen(&f.session);
    let saved_group = model::inspect_extrude_group(f.session.document(), f.group).unwrap();
    assert_eq!(saved_group.extrusion.outputs, f.request.extrusion.outputs);
    assert_eq!(saved_group.booleans[0].feature, first_boolean);
    assert_eq!(saved_group.booleans[0].target, body(&f.targets[0]));

    // Parameter changes rebuild the consumed tool as well as its final targets;
    // the group retains the authored expression and frozen upstream body IDs.
    let mut parameter = f.session.document().get(f.depth).unwrap().clone();
    parameter.name = "cut_depth_renamed".into();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "6".into();
    apply(&mut f.session, vec![Mutation::Replace(parameter)]);
    let changed = compile(&f.session);
    near(changed.snapshot().shapes[&tool.feature].volume_mm3, 768.);
    near(changed.snapshot().shapes[&first_boolean].volume_mm3, 880.);
    assert_eq!(
        model::inspect_extrude_group(f.session.document(), f.group)
            .unwrap()
            .extrusion,
        saved_group.extrusion,
    );

    let state = model::inspect_extrude_group(f.session.document(), f.group).unwrap();
    let mut edit = ExtrudeGroupEdit::from_state(&state).unwrap();
    let added = EntityId::new();
    edit.parameters.start_offset = length(1.);
    edit.targets.push(ExtrudeGroupTarget::Add {
        target: body(&f.targets[1]),
        feature: added,
        name: "Second explicit cut target".into(),
    });
    let original = f.session.document().clone();
    let revision = f.session.revision();
    let prepared = model::prepare_extrude_group_edit(&f.session, revision, actor(), &edit).unwrap();
    let preview = Compiler::default()
        .compile(prepared.prepared.document(), revision + 1)
        .unwrap();
    assert_eq!(f.session.document(), &original);
    assert!(prepared.restored.is_empty());
    assert!(prepared.removed.is_empty());
    near(preview.snapshot().shapes[&tool.feature].bounds.min[2], 1.);
    near(preview.snapshot().shapes[&tool.feature].bounds.max[2], 7.);
    for producer in [first_boolean, added] {
        near(preview.snapshot().shapes[&producer].volume_mm3, 880.);
    }
    f.session.commit(prepared.prepared).unwrap();
    assert_eq!(f.session.revision(), revision + 1);
    let expanded = model::inspect_extrude_group(f.session.document(), f.group).unwrap();
    assert_eq!(expanded.extrusion.outputs, saved_group.extrusion.outputs);
    assert_eq!(expanded.booleans.len(), 2);
    assert!(
        expanded
            .booleans
            .iter()
            .any(|r| r.feature == first_boolean && r.target == body(&f.targets[0]))
    );
    assert!(
        expanded
            .booleans
            .iter()
            .any(|r| r.feature == added && r.target == body(&f.targets[1]))
    );

    // Removing a target restores its exact predecessor, without touching the
    // retained target's producer or an unrelated Part/history.
    let mut edit = ExtrudeGroupEdit::from_state(&expanded).unwrap();
    edit.targets.retain(|target| !matches!(target, ExtrudeGroupTarget::Retain { body } if *body == f.targets[0].body));
    let before_remove = f.session.document().clone();
    let prepared =
        model::prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit)
            .unwrap();
    assert_eq!(prepared.restored, vec![body(&f.targets[0])]);
    assert_eq!(prepared.removed, vec![first_boolean]);
    let final_preview = Compiler::default()
        .compile(prepared.prepared.document(), f.session.revision() + 1)
        .unwrap();
    assert_eq!(
        final_preview.snapshot().bodies[&f.targets[0].body],
        f.targets[0].feature
    );
    assert_eq!(final_preview.snapshot().bodies[&f.targets[1].body], added);
    near(
        final_preview.snapshot().shapes[&f.targets[0].feature].volume_mm3,
        1000.,
    );
    near(final_preview.snapshot().shapes[&added].volume_mm3, 880.);
    assert_eq!(f.session.document(), &before_remove);
    f.session.commit(prepared.prepared).unwrap();
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &before_remove);
    let undone = compile(&f.session);
    assert_eq!(undone.snapshot().bodies[&f.targets[0].body], first_boolean);
    near(undone.snapshot().shapes[&first_boolean].volume_mm3, 880.);
    f.session.redo(f.session.revision(), actor()).unwrap();
    assert_eq!(
        f.session.document().get(f.targets[2].body).unwrap(),
        &unrelated
    );
    f.session = reopen(&f.session);
    let current = compile(&f.session);
    assert_eq!(current.snapshot().bodies, final_preview.snapshot().bodies);
    near(
        current.snapshot().shapes[&f.targets[2].feature].volume_mm3,
        1000.,
    );
    let remaining = model::inspect_extrude_group(f.session.document(), f.group).unwrap();
    assert_eq!(remaining.booleans.len(), 1);
    assert_eq!(remaining.booleans[0].feature, added);
    assert_eq!(remaining.booleans[0].target, body(&f.targets[1]));
}

#[test]
fn group_associative_plane_edit_survives_reopen_and_rebuilds_consumed_tool_and_target() {
    let mut f = Fixture::new();
    let plane = Entity::new(
        "Associative cut limit",
        EntityData::Plane {
            origin_mm: [0., 0., 7.],
            normal: [0., 0., 1.],
            anchor: None,
        },
    );
    apply(&mut f.session, vec![Mutation::Insert(plane.clone())]);
    let state = model::inspect_extrude_group(f.session.document(), f.group).unwrap();
    let mut edit = ExtrudeGroupEdit::from_state(&state).unwrap();
    edit.parameters.extent = Extent::OneSided {
        end: Termination::ToPlane {
            plane: plane.id,
            offset: Dimension::Expression {
                expression: BoundExpression::bind(
                    f.session.document(),
                    "=$cut_depth / 10",
                    "mm",
                    None,
                )
                .unwrap(),
            },
        },
    };
    let before = f.session.document().clone();
    let prepared =
        model::prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit)
            .unwrap();
    let preview = Compiler::default()
        .compile(prepared.prepared.document(), f.session.revision() + 1)
        .unwrap();
    let tool = &f.request.extrusion.outputs[0];
    let target = f.request.booleans[0].feature;
    near(preview.snapshot().shapes[&tool.feature].bounds.max[2], 8.);
    near(preview.snapshot().shapes[&target].volume_mm3, 840.);
    assert_eq!(f.session.document(), &before);
    f.session.commit(prepared.prepared).unwrap();
    f.session = reopen(&f.session);
    let saved = model::inspect_extrude_group(f.session.document(), f.group).unwrap();
    assert_eq!(saved.extrusion.parameters, edit.parameters);
    assert_eq!(saved.booleans[0].target, body(&f.targets[0]));
    assert_eq!(saved.booleans[0].feature, target);
    assert_eq!(saved.extrusion.outputs, state.extrusion.outputs);
    let mut moved = f.session.document().get(plane.id).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut moved.data else {
        panic!()
    };
    origin_mm[2] = 3.;
    apply(&mut f.session, vec![Mutation::Replace(moved)]);
    let current = compile(&f.session);
    near(current.snapshot().shapes[&tool.feature].bounds.max[2], 4.);
    near(current.snapshot().shapes[&tool.feature].volume_mm3, 512.);
    near(current.snapshot().shapes[&target].volume_mm3, 920.);
    assert_eq!(current.snapshot().bodies[&f.targets[0].body], target);
    assert!(!current.snapshot().bodies.contains_key(&tool.body));
    near(
        current.snapshot().shapes[&f.targets[1].feature].volume_mm3,
        1000.,
    );
    near(
        current.snapshot().shapes[&f.targets[2].feature].volume_mm3,
        1000.,
    );
    assert_eq!(
        model::inspect_extrude_group(f.session.document(), f.group).unwrap(),
        saved,
    );
}
