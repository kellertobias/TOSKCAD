use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    constraint_sketch::{Dimension, source::Source},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{
        self, AdoptExtrudeGroupRequest, BodyInput, Extent, ExtrudeBooleanRequest,
        ExtrudeGroupDraft, ExtrudeGroupEdit, ExtrudeGroupTarget, ExtrudeParameters, ExtrudeRequest,
        Operation, PrimitiveParameters, PrimitiveRequest, Termination, length,
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
            label: "Native conversion fixture edit".into(),
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
    let project = ProjectFile::new(session.document().clone()).unwrap();
    let mut bytes = vec![];
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
struct Fixture {
    session: Session,
    stock: PrimitiveRequest,
    extrusion: ExtrudeRequest,
    depth: EntityId,
}
impl Fixture {
    fn new() -> Self {
        let (sketch, edits) = Source::create(Datum::Xy, "Two independent pockets").unwrap();
        let mut doc = Document::new("Native operation conversion")
            .unwrap()
            .edited(&edits)
            .unwrap();
        let mut source = Source::read(&doc, sketch).unwrap().0;
        source.add_rectangle([1., 1.], [3., 3.]).unwrap();
        source.add_rectangle([5., 5.], [7., 7.]).unwrap();
        let regions = sketch_regions::extract(&source);
        assert_eq!(regions.regions.len(), 2);
        assert!(regions.diagnostics.is_empty());
        doc = doc.edited(&source.replace(&doc, sketch).unwrap()).unwrap();
        let depth = Entity::new(
            "depth",
            EntityData::Parameter {
                expression: "5".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        doc = doc.edited(&[Mutation::Insert(depth.clone())]).unwrap();
        let extrusion = ExtrudeRequest::new(
            sketch,
            regions.regions.iter().map(|r| r.intent()).collect(),
            ExtrudeParameters {
                extent: Extent::OneSided {
                    end: Termination::Distance {
                        distance: Dimension::Expression {
                            expression: BoundExpression::bind(&doc, "=$depth", "mm", None).unwrap(),
                        },
                    },
                },
                ..Default::default()
            },
        );
        let mut session = Session::new(doc).unwrap();
        let stock = PrimitiveRequest::new(
            "Stock",
            PrimitiveParameters::Box {
                size: [length(10.), length(10.), length(10.)],
                frame: Default::default(),
            },
        );
        let prepared = model::prepare_primitive(&session, 0, actor(), &stock).unwrap();
        session.commit(prepared.prepared).unwrap();
        Self {
            session,
            stock,
            extrusion,
            depth: depth.id,
        }
    }
    fn stock_input(&self) -> BodyInput {
        BodyInput {
            body: self.stock.body,
            producer: self.stock.feature,
        }
    }
    fn set_depth(&mut self, depth: f64) {
        let mut entity = self.session.document().get(self.depth).unwrap().clone();
        entity.name = "renamed_depth".into();
        let EntityData::Parameter { expression, .. } = &mut entity.data else {
            panic!()
        };
        *expression = depth.to_string();
        apply(&mut self.session, vec![Mutation::Replace(entity)]);
    }
}

#[test]
fn persisted_v1_boolean_converts_to_new_bodies_and_back_with_one_undo() {
    let mut f = Fixture::new();
    let group = ExtrudeGroupDraft::new("Pocket operation", None);
    let group_id = group.id;
    let mut request = ExtrudeBooleanRequest::new(
        f.extrusion.clone(),
        Operation::Cut,
        vec![f.stock_input()],
        false,
    );
    request.group = Some(group);
    let first_boolean = request.booleans[0].feature;
    let prepared =
        model::prepare_extrude_boolean(&f.session, f.session.revision(), actor(), &request)
            .unwrap();
    f.session.commit(prepared.prepared).unwrap();
    // Existing Boolean creation keeps the original schema1 wire definition.
    let EntityData::Extension { definition, .. } =
        &f.session.document().get(group_id).unwrap().data
    else {
        panic!()
    };
    assert_eq!(definition.schema_version, 1);
    assert!(definition.data.get("kind").is_none());
    f.session = reopen(&f.session);
    let state = model::inspect_extrude_group(f.session.document(), group_id).unwrap();
    let original_tools = state.extrusion.clone();
    let initial = compile(&f.session);
    near(initial.snapshot().shapes[&first_boolean].volume_mm3, 960.);
    assert_eq!(initial.snapshot().bodies.len(), 1);
    let mut edit = ExtrudeGroupEdit::from_state(&state).unwrap();
    edit.operation = Operation::NewBody;
    edit.keep_tools = false;
    edit.targets.clear();
    let before = f.session.document().clone();
    let prepared =
        model::prepare_extrude_group_edit(&f.session, f.session.revision(), actor(), &edit)
            .unwrap();
    assert_eq!(prepared.removed, vec![first_boolean]);
    assert_eq!(prepared.restored, vec![f.stock_input()]);
    let native = Compiler::default()
        .compile(prepared.prepared.document(), f.session.revision() + 1)
        .unwrap();
    assert_eq!(f.session.document(), &before);
    assert_eq!(native.snapshot().bodies.len(), 3);
    near(native.snapshot().shapes[&f.stock.feature].volume_mm3, 1000.);
    for tool in &f.extrusion.outputs {
        assert_eq!(native.snapshot().bodies[&tool.body], tool.feature);
        near(native.snapshot().shapes[&tool.feature].volume_mm3, 20.);
    }
    f.session.commit(prepared.prepared).unwrap();
    assert!(f.session.document().get(first_boolean).is_none());
    f.session = reopen(&f.session);
    let new_body = model::inspect_extrude_group(f.session.document(), group_id).unwrap();
    assert!(new_body.booleans.is_empty());
    assert_eq!(new_body.extrusion, original_tools);
    let EntityData::Extension { definition, .. } =
        &f.session.document().get(group_id).unwrap().data
    else {
        panic!()
    };
    assert_eq!(definition.schema_version, 2);
    f.set_depth(6.);
    let resized = compile(&f.session);
    near(
        resized.snapshot().shapes[&f.stock.feature].volume_mm3,
        1000.,
    );
    for tool in &f.extrusion.outputs {
        near(resized.snapshot().shapes[&tool.feature].volume_mm3, 24.);
    }
    let mut edit = ExtrudeGroupEdit::from_state(
        &model::inspect_extrude_group(f.session.document(), group_id).unwrap(),
    )
    .unwrap();
    edit.operation = Operation::Cut;
    let next_boolean = EntityId::new();
    edit.targets.push(ExtrudeGroupTarget::Add {
        target: f.stock_input(),
        feature: next_boolean,
        name: "Explicit restored stock target".into(),
    });
    let new_body_document = f.session.document().clone();
    let revision = f.session.revision();
    let prepared = model::prepare_extrude_group_edit(&f.session, revision, actor(), &edit).unwrap();
    let preview = Compiler::default()
        .compile(prepared.prepared.document(), revision + 1)
        .unwrap();
    near(preview.snapshot().shapes[&next_boolean].volume_mm3, 952.);
    assert_eq!(preview.snapshot().bodies.len(), 1);
    assert_eq!(f.session.document(), &new_body_document);
    f.session.commit(prepared.prepared).unwrap();
    assert_eq!(f.session.revision(), revision + 1);
    let state = model::inspect_extrude_group(f.session.document(), group_id).unwrap();
    assert_eq!(state.group, group_id);
    assert_eq!(state.extrusion, original_tools);
    assert_eq!(state.booleans[0].feature, next_boolean);
    assert_ne!(next_boolean, first_boolean);
    assert_eq!(state.booleans[0].target, f.stock_input());
    f.session.undo(f.session.revision(), actor()).unwrap();
    assert_eq!(f.session.document(), &new_body_document);
    let undone = compile(&f.session);
    assert_eq!(undone.snapshot().bodies.len(), 3);
    near(undone.snapshot().shapes[&f.stock.feature].volume_mm3, 1000.);
    f.session.redo(f.session.revision(), actor()).unwrap();
    f.session = reopen(&f.session);
    near(
        compile(&f.session).snapshot().shapes[&next_boolean].volume_mm3,
        952.,
    );
}

#[test]
fn selected_legacy_tool_adoption_preserves_source_and_does_not_guess_same_sketch_siblings() {
    let mut f = Fixture::new();
    let prepared =
        model::prepare_extrude(&f.session, f.session.revision(), actor(), &f.extrusion).unwrap();
    f.session.commit(prepared.prepared).unwrap();
    f.session = reopen(&f.session);
    let chosen = f.extrusion.outputs[0].clone();
    let sibling = f.extrusion.outputs[1].clone();
    let original = f.session.document().clone();
    let group = ExtrudeGroupDraft::new("Only the selected legacy extrusion", None);
    let group_id = group.id;
    let mut request = AdoptExtrudeGroupRequest::new(
        f.session.document(),
        BodyInput {
            body: chosen.body,
            producer: chosen.feature,
        },
        group,
    )
    .unwrap();
    request.edit.operation = Operation::Cut;
    let boolean = EntityId::new();
    request.edit.targets.push(ExtrudeGroupTarget::Add {
        target: f.stock_input(),
        feature: boolean,
        name: "Explicit selected pocket".into(),
    });
    let prepared =
        model::prepare_adopt_extrude_group(&f.session, f.session.revision(), actor(), &request)
            .unwrap();
    let preview = Compiler::default()
        .compile(prepared.prepared.document(), f.session.revision() + 1)
        .unwrap();
    assert_eq!(f.session.document(), &original);
    assert_eq!(preview.snapshot().bodies.len(), 2);
    near(preview.snapshot().shapes[&boolean].volume_mm3, 980.);
    assert_eq!(preview.snapshot().bodies[&sibling.body], sibling.feature);
    near(preview.snapshot().shapes[&sibling.feature].volume_mm3, 20.);
    assert!(!preview.snapshot().bodies.contains_key(&chosen.body));
    f.session.commit(prepared.prepared).unwrap();
    for id in [chosen.body, chosen.feature, sibling.body, sibling.feature] {
        assert_eq!(f.session.document().get(id), original.get(id));
    }
    let state = model::inspect_extrude_group(f.session.document(), group_id).unwrap();
    assert_eq!(state.extrusion.outputs, vec![chosen.clone()]);
    assert_eq!(state.extrusion.parameters, f.extrusion.parameters);
    assert_eq!(state.extrusion.sketch, f.extrusion.sketch);
    assert_eq!(
        model::extrude_group_owner(f.session.document(), chosen.feature).unwrap(),
        Some(group_id)
    );
    assert_eq!(
        model::extrude_group_owner(f.session.document(), sibling.feature).unwrap(),
        None
    );
    f.session = reopen(&f.session);
    f.set_depth(7.);
    let rebuilt = compile(&f.session);
    near(rebuilt.snapshot().shapes[&chosen.feature].volume_mm3, 28.);
    near(rebuilt.snapshot().shapes[&sibling.feature].volume_mm3, 28.);
    near(rebuilt.snapshot().shapes[&boolean].volume_mm3, 972.);
    assert_eq!(rebuilt.snapshot().bodies.len(), 2);
    assert_eq!(rebuilt.snapshot().bodies[&sibling.body], sibling.feature);
    assert_eq!(
        model::inspect_extrude_group(f.session.document(), group_id).unwrap(),
        state
    );
}
