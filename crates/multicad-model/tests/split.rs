use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, EntityData, Mutation},
    model::{
        self, BodyInput, BooleanRequest, Operation, PrimitiveParameters, PrimitiveRequest,
        SplitRequest, SplitSide, length,
    },
};
use multicad_kernel::{BooleanOperation, GeometryPolicy};
use multicad_model::{Compiler, Model, ShapeSnapshot};
use std::{cell::Cell, collections::BTreeSet};
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 3e-5, "{a} != {b}");
}
struct Fixture {
    session: Session,
    target: PrimitiveRequest,
    tool: PrimitiveRequest,
    request: SplitRequest,
}
impl Fixture {
    fn new(keep_tools: bool) -> Self {
        let mut session = Session::new(Document::new("Native semantic split").unwrap()).unwrap();
        let target = PrimitiveRequest::new(
            "Target",
            PrimitiveParameters::Box {
                size: std::array::from_fn(|_| length(10.)),
                frame: Default::default(),
            },
        );
        let tool = PrimitiveRequest::new(
            "Tool",
            PrimitiveParameters::Box {
                size: std::array::from_fn(|_| length(12.)),
                frame: model::PrimitiveFrame {
                    origin: [length(4.), length(-1.), length(-1.)],
                    ..Default::default()
                },
            },
        );
        for request in [&target, &tool] {
            let p =
                model::prepare_primitive(&session, session.revision(), actor(), request).unwrap();
            session.commit(p.prepared).unwrap();
        }
        let request = SplitRequest::new(
            BodyInput {
                body: target.body,
                producer: target.feature,
            },
            BodyInput {
                body: tool.body,
                producer: tool.feature,
            },
            keep_tools,
        );
        Self {
            session,
            target,
            tool,
            request,
        }
    }
    fn commit(&mut self) {
        let p = model::prepare_split(
            &self.session,
            self.session.revision(),
            actor(),
            &self.request,
        )
        .unwrap();
        self.session.commit(p.prepared).unwrap();
    }
    fn compile(&self) -> Model {
        Compiler::default()
            .compile(self.session.document(), self.session.revision())
            .unwrap()
    }
    fn output(&self, side: SplitSide) -> &model::SplitOutput {
        self.request
            .outputs
            .iter()
            .find(|o| o.side == side)
            .unwrap()
    }
    fn shape<'a>(&self, model: &'a Model, side: SplitSide) -> &'a ShapeSnapshot {
        &model.snapshot().shapes[&self.output(side).feature]
    }
    fn tool(&mut self, x: f64, width: f64) {
        let mut p = self.tool.parameters.clone();
        let PrimitiveParameters::Box { frame, size } = &mut p else {
            panic!()
        };
        frame.origin[0] = length(x);
        size[0] = length(width);
        let p = model::prepare_primitive_edit(
            &self.session,
            self.session.revision(),
            actor(),
            self.tool.feature,
            &p,
        )
        .unwrap();
        self.session.commit(p.prepared).unwrap();
    }
    fn apply(&mut self, edits: Vec<Mutation>) {
        self.session
            .apply(Request {
                expected_revision: self.session.revision(),
                actor: actor(),
                label: "Split fixture edit".into(),
                edits,
            })
            .unwrap();
    }
}
#[test]
fn independent_persistent_parts_partition_target_and_recombine_using_owned_exact_geometry() {
    for keep in [false, true] {
        let mut f = Fixture::new(keep);
        let original = f.session.document().clone();
        let revision = f.session.revision();
        let p = model::prepare_split(&f.session, revision, actor(), &f.request).unwrap();
        let candidate = p.prepared.document().clone();
        let preview = Compiler::default()
            .compile(&candidate, revision + 1)
            .unwrap();
        assert_eq!(f.session.document(), &original);
        for (side, volume) in [
            (SplitSide::InsideTool, 600.),
            (SplitSide::OutsideTool, 400.),
        ] {
            let output = f.output(side);
            assert_eq!(preview.snapshot().bodies[&output.body], output.feature);
            let shape = &preview.snapshot().shapes[&output.feature];
            assert_eq!(shape.solid_count, 1);
            near(shape.volume_mm3, volume);
        }
        let inside = preview
            .solid(f.output(SplitSide::InsideTool).feature)
            .unwrap();
        let outside = preview
            .solid(f.output(SplitSide::OutsideTool).feature)
            .unwrap();
        assert!(
            inside
                .boolean(
                    outside,
                    BooleanOperation::Intersect,
                    GeometryPolicy::default()
                )
                .unwrap()
                .is_none()
        );
        let restored = inside
            .boolean(outside, BooleanOperation::Union, GeometryPolicy::default())
            .unwrap()
            .unwrap();
        near(restored.volume_mm3().unwrap(), 1000.);
        assert_eq!(
            preview.snapshot().consumed_bodies,
            if keep {
                BTreeSet::from([f.target.body])
            } else {
                BTreeSet::from([f.target.body, f.tool.body])
            }
        );
        assert!(!preview.snapshot().bodies.contains_key(&f.target.body));
        assert_eq!(preview.snapshot().bodies.contains_key(&f.tool.body), keep);
        assert_eq!(candidate.get(f.target.body), original.get(f.target.body));
        assert_eq!(candidate.get(f.tool.body), original.get(f.tool.body));
        f.session.commit(p.prepared).unwrap();
        let committed = f.session.document().clone();
        f.session.undo(revision + 1, actor()).unwrap();
        assert_eq!(f.session.document(), &original);
        f.session.redo(revision + 2, actor()).unwrap();
        assert_eq!(f.session.document(), &committed);
    }
}
#[test]
fn upstream_tool_edits_keep_semantic_output_ids_when_fragment_size_order_reverses() {
    let mut f = Fixture::new(false);
    f.commit();
    let original_outputs = f.request.outputs.clone();
    let before = f.compile();
    near(f.shape(&before, SplitSide::InsideTool).volume_mm3, 600.);
    f.tool(6., 12.);
    let source = f.session.document().clone();
    let after = f.compile();
    assert_eq!(f.request.outputs, original_outputs);
    near(f.shape(&after, SplitSide::InsideTool).volume_mm3, 400.);
    near(f.shape(&after, SplitSide::OutsideTool).volume_mm3, 600.);
    near(f.shape(&after, SplitSide::InsideTool).bounds.min[0], 6.);
    near(f.shape(&after, SplitSide::OutsideTool).bounds.max[0], 6.);
    assert_eq!(f.session.document(), &source);
    let before_caps = &before.snapshot().shapes[&f.target.feature].topology;
    let after_caps = &after.snapshot().shapes[&f.target.feature].topology;
    assert_eq!(before_caps.elements, after_caps.elements);
}
#[test]
fn no_hit_empty_and_multiple_fragments_fail_native_preview_without_committing_source() {
    for (x, width) in [(12., 12.), (10., 12.), (-1., 12.), (4., 2.)] {
        let mut f = Fixture::new(false);
        f.tool(x, width);
        let original = f.session.document().clone();
        let revision = f.session.revision();
        let p = model::prepare_split(&f.session, revision, actor(), &f.request).unwrap();
        let error = Compiler::default()
            .compile(p.prepared.document(), revision + 1)
            .err()
            .expect("Invalid partition cannot produce a native preview");
        assert!(
            f.request
                .outputs
                .iter()
                .any(|o| Some(o.feature) == error.entity)
        );
        assert!(error.message.contains("Split"));
        drop(p);
        assert_eq!(f.session.revision(), revision);
        assert_eq!(f.session.document(), &original);
    }
}
#[test]
fn deleting_a_peer_is_independent_but_cannot_publish_the_good_half_of_an_invalid_pair() {
    for invalid in [false, true] {
        let mut f = Fixture::new(false);
        if invalid {
            f.tool(4., 2.);
        }
        f.commit();
        let outside = f.output(SplitSide::OutsideTool).clone();
        f.apply(vec![
            Mutation::Remove(outside.body),
            Mutation::Remove(outside.feature),
        ]);
        let original = f.session.document().clone();
        let result = Compiler::default().compile(&original, f.session.revision());
        if invalid {
            let error = result.err().unwrap();
            assert_eq!(error.entity, Some(f.output(SplitSide::InsideTool).feature));
            assert!(
                error.message.contains("OutsideTool") && error.message.contains("disconnected")
            );
        } else {
            let model = result.unwrap();
            assert_eq!(model.snapshot().bodies.len(), 1);
            near(f.shape(&model, SplitSide::InsideTool).volume_mm3, 600.);
        }
        assert_eq!(f.session.document(), &original);
    }
}
#[test]
fn cancellation_and_missing_upstream_geometry_never_return_a_partial_model() {
    let mut f = Fixture::new(false);
    f.commit();
    let original = f.session.document().clone();
    let calls = Cell::new(0);
    let interrupt = || {
        calls.set(calls.get() + 1);
        calls.get() >= 2
    };
    assert!(
        Compiler::default()
            .compile_interruptible(&original, f.session.revision(), Some(&interrupt))
            .is_err()
    );
    assert_eq!(f.session.document(), &original);
    f.apply(vec![Mutation::Remove(f.tool.feature)]);
    let source = f.session.document().clone();
    assert!(
        Compiler::default()
            .compile(&source, f.session.revision())
            .is_err()
    );
    assert_eq!(f.session.document(), &source);
}
#[test]
fn later_target_history_reactivates_only_the_new_version_after_split_consumption() {
    let mut f = Fixture::new(true);
    f.commit();
    let inside = f.output(SplitSide::InsideTool).clone();
    let outside = f.output(SplitSide::OutsideTool).clone();
    let request = BooleanRequest::new(
        Operation::Cut,
        f.request.target.clone(),
        vec![BodyInput {
            body: inside.body,
            producer: inside.feature,
        }],
    );
    let p = model::prepare_boolean(&f.session, f.session.revision(), actor(), &request).unwrap();
    f.session.commit(p.prepared).unwrap();
    let model = f.compile();
    assert_eq!(model.snapshot().bodies[&f.target.body], request.feature);
    assert!(model.snapshot().bodies.contains_key(&outside.body));
    assert!(!model.snapshot().bodies.contains_key(&inside.body));
    assert!(!model.snapshot().consumed_bodies.contains(&f.target.body));
    assert!(model.snapshot().consumed_bodies.contains(&inside.body));
    near(model.snapshot().shapes[&request.feature].volume_mm3, 400.);
    let EntityData::Part { features, .. } = &f.session.document().get(f.target.body).unwrap().data
    else {
        panic!()
    };
    assert_eq!(features, &[f.target.feature, request.feature]);
}

#[test]
fn suppressing_outputs_releases_consumption_but_suppressing_a_required_input_never_falls_back() {
    let mut f = Fixture::new(false);
    f.commit();
    let set_suppressed = |f: &mut Fixture, id, suppressed| {
        let mut entity = f.session.document().get(id).unwrap().clone();
        let EntityData::Feature { suppressed: s, .. } = &mut entity.data else {
            panic!()
        };
        *s = suppressed;
        f.apply(vec![Mutation::Replace(entity)]);
    };
    let inside = f.output(SplitSide::InsideTool).clone();
    let outside = f.output(SplitSide::OutsideTool).clone();
    set_suppressed(&mut f, inside.feature, true);
    let one = f.compile();
    assert_eq!(one.snapshot().bodies.len(), 1);
    assert_eq!(one.snapshot().bodies[&outside.body], outside.feature);
    assert!(!one.snapshot().shapes.contains_key(&inside.feature));
    set_suppressed(&mut f, outside.feature, true);
    let released = f.compile();
    assert_eq!(released.snapshot().bodies.len(), 2);
    assert_eq!(released.snapshot().bodies[&f.target.body], f.target.feature);
    assert_eq!(released.snapshot().bodies[&f.tool.body], f.tool.feature);
    assert!(released.snapshot().consumed_bodies.is_empty());
    set_suppressed(&mut f, inside.feature, false);
    let previous = f.compile();
    let tool = f.tool.feature;
    set_suppressed(&mut f, tool, true);
    let source = f.session.document().clone();
    let error = Compiler::default()
        .compile(&source, f.session.revision())
        .err()
        .unwrap();
    assert_eq!(error.entity, Some(tool));
    assert!(error.message.contains("suppressed"));
    let stale_query = multicad_model::CutTargetQuery {
        tools: vec![inside.feature],
        component: None,
        include: vec![],
        exclude: vec![],
    };
    assert!(
        previous
            .cut_targets(&source, f.session.revision(), &stale_query, None)
            .unwrap_err()
            .message
            .contains("current document")
    );
    assert_eq!(f.session.document(), &source);
}
