use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, Mutation},
    hierarchy::{self, Action, ContainerKind},
    model::{
        self, BodyInput, BooleanEdit, BooleanRequest, Operation, PrimitiveFrame,
        PrimitiveParameters, PrimitiveRequest, length,
    },
};
use multicad_model::{Compiler, Model};
use std::collections::BTreeSet;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}
struct Boxes {
    session: Session,
    target: PrimitiveRequest,
    tool: PrimitiveRequest,
}
impl Boxes {
    fn new(origin: [f64; 3], size: [f64; 3]) -> Self {
        Self::with_session(
            Session::new(Document::new("Native Booleans").unwrap()).unwrap(),
            origin,
            size,
        )
    }
    fn with_session(mut session: Session, origin: [f64; 3], size: [f64; 3]) -> Self {
        let target = PrimitiveRequest::new(
            "Target",
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(10.)],
                frame: Default::default(),
            },
        );
        let tool = PrimitiveRequest::new(
            "Tool",
            PrimitiveParameters::Box {
                size: size.map(length),
                frame: PrimitiveFrame {
                    origin: origin.map(length),
                    ..Default::default()
                },
            },
        );
        for request in [&target, &tool] {
            let prepared =
                model::prepare_primitive(&session, session.revision(), actor(), request).unwrap();
            session.commit(prepared.prepared).unwrap();
        }
        Self {
            session,
            target,
            tool,
        }
    }
    fn apply(&mut self, edits: Vec<Mutation>) {
        self.session
            .apply(Request {
                expected_revision: self.session.revision(),
                actor: actor(),
                label: "Fixture edit".into(),
                edits,
            })
            .unwrap();
    }
    fn boolean(&mut self, operation: Operation, keep_tools: bool) -> BooleanRequest {
        let mut request = BooleanRequest::new(
            operation,
            BodyInput {
                body: self.target.body,
                producer: self.target.feature,
            },
            vec![BodyInput {
                body: self.tool.body,
                producer: self.tool.feature,
            }],
        );
        request.keep_tools = keep_tools;
        let prepared =
            model::prepare_boolean(&self.session, self.session.revision(), actor(), &request)
                .unwrap();
        self.session.commit(prepared.prepared).unwrap();
        request
    }
    fn compile(&self) -> Model {
        Compiler::default()
            .compile(self.session.document(), self.session.revision())
            .unwrap()
    }
}
#[test]
fn connected_join_cut_and_intersect_keep_target_identity_and_honor_tool_consumption() {
    for (operation, volume, min_x, max_x) in [
        (Operation::Join, 1500., 0., 15.),
        (Operation::Cut, 500., 0., 5.),
        (Operation::Intersect, 500., 5., 10.),
    ] {
        for keep in [false, true] {
            let mut fixture = Boxes::new([5., 0., 0.], [10., 10., 10.]);
            let tool_source = fixture
                .session
                .document()
                .get(fixture.tool.body)
                .unwrap()
                .clone();
            let request = fixture.boolean(operation, keep);
            let original = fixture.session.document().clone();
            let model = fixture.compile();
            let snapshot = model.snapshot();
            assert_eq!(snapshot.bodies[&fixture.target.body], request.feature);
            assert_eq!(snapshot.bodies.len(), if keep { 2 } else { 1 });
            assert_eq!(snapshot.bodies.contains_key(&fixture.tool.body), keep);
            assert_eq!(
                snapshot.consumed_bodies,
                if keep {
                    BTreeSet::new()
                } else {
                    BTreeSet::from([fixture.tool.body])
                }
            );
            let result = &snapshot.shapes[&request.feature];
            near(result.volume_mm3, volume);
            assert_eq!(result.solid_count, 1);
            near(result.bounds.min[0], min_x);
            near(result.bounds.max[0], max_x);
            near(result.bounds.min[1], 0.);
            near(result.bounds.max[1], 10.);
            assert!(snapshot.shapes.contains_key(&fixture.target.feature));
            assert!(snapshot.shapes.contains_key(&fixture.tool.feature));
            assert!(
                model.solid(fixture.tool.feature).is_some(),
                "Consumed tool's exact history was discarded"
            );
            assert_eq!(
                fixture.session.document().get(fixture.tool.body).unwrap(),
                &tool_source
            );
            let EntityData::Part { features, .. } =
                &original.get(fixture.target.body).unwrap().data
            else {
                panic!()
            };
            assert_eq!(features, &[fixture.target.feature, request.feature]);
            let links = original.get(request.feature).unwrap().references();
            assert!(
                links
                    .iter()
                    .all(|link| link.target != fixture.target.body
                        && link.target != fixture.tool.body)
            );
            assert_eq!(fixture.session.document(), &original);
        }
    }
}
#[test]
fn upstream_dimension_edit_rebuilds_the_frozen_boolean_inputs_and_keep_tools_edit_undo() {
    let mut fixture = Boxes::new([5., 0., 0.], [10., 10., 10.]);
    let request = fixture.boolean(Operation::Join, false);
    let frozen = model::read_boolean(fixture.session.document(), request.feature).unwrap();
    let updated = PrimitiveParameters::Box {
        size: [length(10.), length(10.), length(20.)],
        frame: Default::default(),
    };
    let prepared = model::prepare_primitive_edit(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        fixture.target.feature,
        &updated,
    )
    .unwrap();
    let change = fixture.session.commit(prepared.prepared).unwrap();
    assert!(change.invalidated.contains(&request.feature));
    let result = fixture.compile();
    near(result.snapshot().shapes[&request.feature].volume_mm3, 2500.);
    assert_eq!(
        result.snapshot().bodies[&fixture.target.body],
        request.feature
    );
    assert_eq!(
        model::read_boolean(fixture.session.document(), request.feature).unwrap(),
        frozen
    );
    let before = fixture.session.document().clone();
    let preview = model::prepare_boolean_edit(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        request.feature,
        &BooleanEdit {
            operation: Operation::Join,
            tools: request.tools.clone(),
            keep_tools: true,
        },
    )
    .unwrap();
    assert_eq!(fixture.session.document(), &before);
    fixture.session.commit(preview.prepared).unwrap();
    let restored = fixture.compile();
    assert!(restored.snapshot().bodies.contains_key(&fixture.tool.body));
    assert!(restored.snapshot().consumed_bodies.is_empty());
    near(
        restored.snapshot().shapes[&request.feature].volume_mm3,
        2500.,
    );
    fixture
        .session
        .undo(fixture.session.revision(), actor())
        .unwrap();
    assert_eq!(fixture.session.document(), &before);
    let undone = fixture.compile();
    assert!(!undone.snapshot().bodies.contains_key(&fixture.tool.body));
    near(undone.snapshot().shapes[&request.feature].volume_mm3, 2500.);
}
#[test]
fn empty_or_disconnected_boolean_results_are_explicit_failures_without_default_parts() {
    for (operation, origin, size, reason) in [
        (
            Operation::Cut,
            [-1., -1., -1.],
            [12., 12., 12.],
            "all material",
        ),
        (
            Operation::Intersect,
            [20., 0., 0.],
            [10., 10., 10.],
            "all material",
        ),
        (
            Operation::Join,
            [20., 0., 0.],
            [10., 10., 10.],
            "disconnected",
        ),
        (
            Operation::Cut,
            [4., -1., -1.],
            [2., 12., 12.],
            "disconnected",
        ),
    ] {
        let mut fixture = Boxes::new(origin, size);
        let valid = fixture.compile();
        assert_eq!(valid.snapshot().bodies.len(), 2);
        let request = fixture.boolean(operation, false);
        let original = fixture.session.document().clone();
        let error = Compiler::default()
            .compile(&original, fixture.session.revision())
            .err()
            .expect("Empty/multiple-fragment output cannot masquerade as a valid single Part");
        assert_eq!(error.entity, Some(request.feature));
        assert!(error.message.contains(reason), "{error}");
        assert_eq!(fixture.session.document(), &original);
        assert_ne!(valid.snapshot().stamp.revision, fixture.session.revision());
    }
}
#[test]
fn boolean_uses_target_ownership_even_when_another_component_is_active() {
    let mut session = Session::new(Document::new("Boolean ownership").unwrap()).unwrap();
    let first = hierarchy::prepare(
        &session,
        0,
        actor(),
        Action::Create {
            kind: ContainerKind::Component,
            name: "Owning component".into(),
            parent: None,
        },
    )
    .unwrap();
    let owner = first.created.unwrap();
    session.commit(first.prepared).unwrap();
    let active = hierarchy::prepare(
        &session,
        session.revision(),
        actor(),
        Action::SetActive {
            component: Some(owner),
        },
    )
    .unwrap();
    session.commit(active.prepared).unwrap();
    let mut fixture = Boxes::with_session(session, [5., 0., 0.], [10., 10., 10.]);
    let second = hierarchy::prepare(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        Action::Create {
            kind: ContainerKind::Component,
            name: "Unrelated active component".into(),
            parent: None,
        },
    )
    .unwrap();
    let other = second.created.unwrap();
    fixture.session.commit(second.prepared).unwrap();
    let active = hierarchy::prepare(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        Action::SetActive {
            component: Some(other),
        },
    )
    .unwrap();
    fixture.session.commit(active.prepared).unwrap();
    let request = fixture.boolean(Operation::Cut, false);
    let document = fixture.session.document();
    let index = hierarchy::Index::new(document).unwrap();
    for id in [fixture.target.body, fixture.tool.body, request.feature] {
        assert_eq!(index.owning_component(id).unwrap(), Some(owner));
    }
    assert_eq!(document.hierarchy().active_component, Some(other));
    let model = fixture.compile();
    near(model.snapshot().shapes[&request.feature].volume_mm3, 500.);
}
#[test]
fn missing_or_wrong_kind_history_does_not_fall_back_to_an_earlier_solid() {
    for missing in [true, false] {
        let mut fixture = Boxes::new([5., 0., 0.], [10., 10., 10.]);
        let wrong = Entity::new("Invalid history kind", EntityData::Group {});
        if !missing {
            fixture.apply(vec![Mutation::Insert(wrong.clone())]);
        }
        let mut target = fixture
            .session
            .document()
            .get(fixture.target.body)
            .unwrap()
            .clone();
        let EntityData::Part { features, .. } = &mut target.data else {
            panic!()
        };
        features.push(wrong.id);
        fixture.apply(vec![Mutation::Replace(target)]);
        let error = Compiler::default()
            .compile(fixture.session.document(), fixture.session.revision())
            .err()
            .expect("Invalid final history cannot publish its predecessor as current");
        assert_eq!(error.entity, Some(fixture.target.body));
        assert!(error.message.contains(&wrong.id.to_string()));
    }
}
#[test]
fn intentionally_suppressed_histories_keep_source_parts_without_fabricating_empty_solids() {
    let mut fixture = Boxes::new([5., 0., 0.], [10., 10., 10.]);
    let mut edits = vec![];
    for id in [fixture.target.feature, fixture.tool.feature] {
        let mut entity = fixture.session.document().get(id).unwrap().clone();
        let EntityData::Feature { suppressed, .. } = &mut entity.data else {
            panic!()
        };
        *suppressed = true;
        edits.push(Mutation::Replace(entity));
    }
    fixture.apply(edits);
    let original = fixture.session.document().clone();
    let model = fixture.compile();
    assert!(model.snapshot().bodies.is_empty());
    assert!(model.snapshot().shapes.is_empty());
    assert!(original.get(fixture.target.body).is_some());
    assert!(original.get(fixture.tool.body).is_some());
    assert_eq!(fixture.session.document(), &original);
}
#[test]
fn consumption_is_scoped_to_a_frozen_producer_and_later_target_history_reactivates_the_body() {
    let mut fixture = Boxes::new([5., 0., 0.], [10., 10., 10.]);
    let first = fixture.boolean(Operation::Join, false);
    let initial = fixture.compile();
    assert_eq!(
        initial.snapshot().consumed_bodies,
        BTreeSet::from([fixture.tool.body])
    );
    assert!(!initial.snapshot().bodies.contains_key(&fixture.tool.body));
    let next = BooleanRequest::new(
        Operation::Join,
        BodyInput {
            body: fixture.tool.body,
            producer: fixture.tool.feature,
        },
        vec![BodyInput {
            body: fixture.target.body,
            producer: first.feature,
        }],
    );
    let prepared =
        model::prepare_boolean(&fixture.session, fixture.session.revision(), actor(), &next)
            .unwrap();
    fixture.session.commit(prepared.prepared).unwrap();
    let original = fixture.session.document().clone();
    let current = fixture.compile();
    assert_eq!(current.snapshot().bodies.len(), 1);
    assert_eq!(current.snapshot().bodies[&fixture.tool.body], next.feature);
    assert_eq!(
        current.snapshot().consumed_bodies,
        BTreeSet::from([fixture.target.body])
    );
    near(current.snapshot().shapes[&next.feature].volume_mm3, 1500.);
    assert_eq!(fixture.session.document(), &original);
    let parameters = PrimitiveParameters::Box {
        size: [length(10.), length(10.), length(20.)],
        frame: PrimitiveFrame {
            origin: [length(5.), length(0.), length(0.)],
            ..Default::default()
        },
    };
    let prepared = model::prepare_primitive_edit(
        &fixture.session,
        fixture.session.revision(),
        actor(),
        fixture.tool.feature,
        &parameters,
    )
    .unwrap();
    fixture.session.commit(prepared.prepared).unwrap();
    let updated = fixture.compile();
    assert_eq!(updated.snapshot().bodies[&fixture.tool.body], next.feature);
    assert_eq!(
        updated.snapshot().consumed_bodies,
        BTreeSet::from([fixture.target.body])
    );
    near(updated.snapshot().shapes[&next.feature].volume_mm3, 2500.);
    assert!(
        updated
            .snapshot()
            .shapes
            .contains_key(&fixture.tool.feature)
    );
}
