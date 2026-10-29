use super::forms::Operation;
use super::*;
use crate::{
    egui,
    inspector_actions::{self, Owner},
    modeling::native_fixture,
};
use multicad_core::{
    commands::{Actor, Channel, Request, Session},
    document::{Document, Entity, EntityData, EntityId, Mutation},
    model::{self, PrimitiveParameters, PrimitiveRequest, length},
    parameters::{BoundExpression, Unit},
    project_file::{self, ProjectFile, ProjectResources},
    symbols::{self, SymbolAuthorRequest, SymbolKind, SymbolValue},
};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
fn actor() -> Actor {
    Actor::local(Channel::Ui)
}
fn fixture() -> (Session, PrimitiveRequest, EntityId) {
    let mut session = Session::new(Document::new("Symbol UI").unwrap()).unwrap();
    let parameter = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "10 mm".into(),
            unit: "mm".into(),
            exposed: false,
            inputs: vec![],
        },
    );
    let width = parameter.id;
    session
        .commit(
            session
                .prepare(Request {
                    expected_revision: 0,
                    actor: actor(),
                    label: "Parameter".into(),
                    edits: vec![Mutation::Insert(parameter)],
                })
                .unwrap(),
        )
        .unwrap();
    let part = PrimitiveRequest::new(
        "Cabinet",
        PrimitiveParameters::Box {
            size: [
                multicad_core::constraint_sketch::Dimension::Expression {
                    expression: BoundExpression::bind(session.document(), "=$width", "mm", None)
                        .unwrap(),
                },
                length(20.),
                length(30.),
            ],
            frame: Default::default(),
        },
    );
    session
        .commit(
            model::prepare_primitive(&session, session.revision(), actor(), &part)
                .unwrap()
                .prepared,
        )
        .unwrap();
    (session, part, width)
}
fn controller() -> SymbolController {
    let mut c = SymbolController::default();
    c.set_backend(Box::new(native_fixture::NativeBackend::new()));
    c
}
fn wait(c: &mut SymbolController, s: &Session, r: &ProjectResources) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while c.pending() {
        c.poll(s, r);
        assert!(Instant::now() < deadline, "native preview timeout");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn begin(c: &mut SymbolController, s: &Session, tool: SymbolTool, selection: SymbolSelection) {
    c.begin(tool, s, &Default::default(), None, selection, Channel::Ui)
        .unwrap();
}
fn embed(s: &mut Session, part: EntityId, width: EntityId) -> EntityId {
    let request = SymbolAuthorRequest::new(
        "Cabinet symbol",
        part,
        SymbolKind::Dynamic,
        vec![width],
        None,
    );
    let result = symbols::prepare_author(s, s.revision(), actor(), &request).unwrap();
    let id = result.definition;
    s.commit(result.prepared).unwrap();
    id
}
fn frame(
    ctx: &egui::Context,
    c: &mut SymbolController,
    s: &mut Session,
    events: Vec<egui::Event>,
    height: f32,
) -> (egui::FullOutput, Option<SymbolOutcome>) {
    let mut outcome = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(250., height),
            )),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            inspector_actions::scope(ui, Some(Owner::Symbols), egui::Color32::BLUE, |ui| {
                outcome = c.show(ui, s, &Default::default(), true);
            });
        },
    );
    output.textures_delta.clear();
    (output, outcome)
}
fn point(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(s) if s.galley.text() == text => Some(s.pos + s.galley.size() * 0.5),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, text))
        .unwrap_or_else(|| panic!("Missing painted control: {text}"))
}
fn click(
    ctx: &egui::Context,
    c: &mut SymbolController,
    s: &mut Session,
    pos: egui::Pos2,
    height: f32,
) -> Option<SymbolOutcome> {
    let mut outcome = None;
    for pressed in [true, false] {
        let (output, value) = frame(
            ctx,
            c,
            s,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            height,
        );
        output.drop_without_applying_deltas();
        outcome = value.or(outcome);
    }
    outcome
}
#[test]
fn create_dynamic_author_uses_explicit_exposure_and_fixed_footer_native_apply() {
    let (mut s, part, width) = fixture();
    let baseline = s.document().clone();
    let revision = s.revision();
    let mut c = controller();
    begin(
        &mut c,
        &s,
        SymbolTool::Create,
        SymbolSelection {
            component: Some(part.body),
            ..Default::default()
        },
    );
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 600.);
    let widthpoint = point(&output, "width");
    click(&ctx, &mut c, &mut s, widthpoint, 600.);
    assert!(
        matches!(&c.form.as_ref().unwrap().operation,Operation::Author(r)if r.exposed==[width])
    );
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 360.);
    let preview = point(&output, "Preview");
    let cancel = point(&output, "Cancel");
    assert!(preview.y > 250. && cancel.y < 360.);
    click(&ctx, &mut c, &mut s, preview, 360.);
    assert_eq!(s.document(), &baseline);
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 360.);
    let outcome = click(&ctx, &mut c, &mut s, point(&output, "Apply"), 360.).unwrap();
    let SymbolOutcome::Applied { selected, .. } = outcome else {
        panic!()
    };
    assert_eq!(s.revision(), revision + 1);
    let definition = symbols::read_definition(s.document(), selected[0]).unwrap();
    assert_eq!(definition.exposed, [width]);
    assert!(matches!(
        s.document().get(width).unwrap().data,
        EntityData::Parameter { exposed: false, .. }
    ));
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &baseline);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(
        symbols::read_definition(s.document(), selected[0]).unwrap(),
        definition
    );
}
#[test]
fn library_place_edit_failure_and_cancel_keep_per_instance_values_source_neutral() {
    let (mut s, part, width) = fixture();
    let definition = embed(&mut s, part.body, width);
    let baseline = s.document().clone();
    let mut c = controller();
    begin(&mut c, &s, SymbolTool::Library, Default::default());
    assert!(!c.dirty());
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 700.);
    click(&ctx, &mut c, &mut s, point(&output, "Place"), 700.);
    assert!(c.dirty());
    if let Operation::Place { values, .. } = &mut c.form.as_mut().unwrap().operation {
        values.0.get_mut(&width).unwrap().enabled = true;
        values.0.get_mut(&width).unwrap().text = "30".into();
    } else {
        panic!()
    }
    c.form.as_mut().unwrap().changed();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    assert_eq!(s.document(), &baseline);
    let SymbolOutcome::Applied { selected, .. } = c.apply(&mut s, &Default::default()).unwrap()
    else {
        panic!()
    };
    let instance = selected[0];
    let first = symbols::read_instance(s.document(), instance).unwrap();
    let record = instance;
    assert_eq!(first.overrides[&width].value, 30.);
    begin(&mut c, &s, SymbolTool::Place, Default::default());
    c.form
        .as_mut()
        .unwrap()
        .place(s.document(), definition)
        .unwrap();
    if let Operation::Place { values, pose, .. } = &mut c.form.as_mut().unwrap().operation {
        values.0.get_mut(&width).unwrap().enabled = true;
        values.0.get_mut(&width).unwrap().text = "50".into();
        pose.translation[0] = "100".into();
    }
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    let SymbolOutcome::Applied {
        selected: second, ..
    } = c.apply(&mut s, &Default::default()).unwrap()
    else {
        panic!()
    };
    let native = c.display_scene(&s, &Default::default()).unwrap();
    let first_bounds = native.native_instance_bounds(instance).unwrap();
    let second_bounds = native.native_instance_bounds(second[0]).unwrap();
    assert!(((first_bounds[1][0] - first_bounds[0][0]) - 30.).abs() < 1e-7);
    assert!(((second_bounds[1][0] - second_bounds[0][0]) - 50.).abs() < 1e-7);
    assert!((second_bounds[0][0] - 100.).abs() < 1e-7);
    assert_eq!(
        symbols::read_instance(s.document(), record)
            .unwrap()
            .overrides[&width]
            .value,
        30.
    );
    let before = s.document().clone();
    let revision = s.revision();
    c.begin_existing(&s, &Default::default(), None, instance, Channel::Ui)
        .unwrap();
    if let Operation::Edit { values, .. } = &mut c.form.as_mut().unwrap().operation {
        values.0.get_mut(&width).unwrap().text = "-5".into();
    }
    let result = c.preview(&s, &Default::default(), Channel::Ui);
    if result.is_ok() {
        wait(&mut c, &s, &Default::default());
    }
    assert!(!c.can_apply(&s, &Default::default()));
    assert!(c.error.is_some());
    assert!(c.active());
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), revision);
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 360.);
    assert_eq!(
        click(&ctx, &mut c, &mut s, point(&output, "Cancel"), 360.),
        Some(SymbolOutcome::Cancelled)
    );
    assert_eq!(s.document(), &before);
    let file = ProjectFile::new(s.document().clone()).unwrap();
    let mut bytes = vec![];
    project_file::write(&file, &mut bytes, Default::default()).unwrap();
    let reopened = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap();
    assert_eq!(
        symbols::instances(&reopened.project.document, definition)
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn session_resources_and_source_changes_revoke_native_receipts_and_library_actions_do_not_mutate() {
    let (mut s, part, width) = fixture();
    let definition = embed(&mut s, part.body, width);
    let mut c = controller();
    begin(&mut c, &s, SymbolTool::Place, Default::default());
    c.form
        .as_mut()
        .unwrap()
        .place(s.document(), definition)
        .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()));
    let foreign = Session::new(s.document().clone()).unwrap();
    assert!(!c.can_apply(&foreign, &Default::default()));
    let mut resources = ProjectResources::default();
    resources
        .metadata
        .insert("changed".into(), serde_json::json!(true));
    c.poll(&s, &resources);
    assert!(!c.can_apply(&s, &resources));
    assert!(c.display_scene(&s, &resources).is_none());
    c.cancel();
    begin(&mut c, &s, SymbolTool::Place, Default::default());
    c.form
        .as_mut()
        .unwrap()
        .place(s.document(), definition)
        .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    let mut parameter = s.document().get(width).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut parameter.data {
        *expression = "12 mm".into();
    }
    s.commit(
        s.prepare(Request {
            expected_revision: s.revision(),
            actor: actor(),
            label: "Source edit".into(),
            edits: vec![Mutation::Replace(parameter)],
        })
        .unwrap(),
    )
    .unwrap();
    assert!(!c.can_apply(&s, &Default::default()));
    c.cancel();
    let baseline = s.document().clone();
    begin(&mut c, &s, SymbolTool::Library, Default::default());
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 700.);
    click(
        &ctx,
        &mut c,
        &mut s,
        point(&output, "Open library file…"),
        700.,
    );
    assert_eq!(c.take_action(), Some(SymbolAction::ImportLibrary));
    assert_eq!(s.document(), &baseline);
    assert!(!c.dirty());
}
#[test]
fn explicit_revision_comparison_retains_compatible_values_and_updates_one_undoable_candidate() {
    let (mut s, part, width) = fixture();
    let definition = embed(&mut s, part.body, width);
    let place = symbols::SymbolPlaceRequest::new(
        s.document(),
        definition,
        "Cabinet A",
        BTreeMap::from([(
            width,
            SymbolValue {
                value: 30.,
                unit: "mm".into(),
            },
        )]),
        Default::default(),
    )
    .unwrap();
    s.commit(
        symbols::prepare_place(&s, s.revision(), actor(), &place)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut replacement = symbols::read_definition(s.document(), definition).unwrap();
    replacement.revision = "2".into();
    let mut entity = replacement.template.get(width).unwrap().clone();
    if let EntityData::Parameter { expression, .. } = &mut entity.data {
        *expression = "14 mm".into();
    }
    replacement.template = replacement
        .template
        .edited(&[Mutation::Replace(entity)])
        .unwrap();
    let before = s.document().clone();
    let mut c = controller();
    c.set_library_snapshot(vec![replacement.clone()]).unwrap();
    begin(
        &mut c,
        &s,
        SymbolTool::Update,
        SymbolSelection {
            instance: Some(place.instance),
            ..Default::default()
        },
    );
    c.form
        .as_mut()
        .unwrap()
        .set_replacement(s.document(), replacement.clone())
        .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    assert_eq!(s.document(), &before);
    c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(
        symbols::read_definition(s.document(), definition).unwrap(),
        replacement
    );
    assert_eq!(
        symbols::read_instance(s.document(), place.instance)
            .unwrap()
            .overrides[&width]
            .value,
        30.
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn override_values_reject_removed_incompatible_and_nonfinite_inputs() {
    let (mut s, part, width) = fixture();
    let id = embed(&mut s, part.body, width);
    let d = symbols::read_definition(s.document(), id).unwrap();
    let mut fields = super::fields::Overrides::new(
        &d,
        &BTreeMap::from([(
            width,
            SymbolValue {
                value: 3.,
                unit: "cm".into(),
            },
        )]),
    )
    .unwrap();
    assert_eq!(fields.resolve(&d).unwrap()[&width].unit, "cm");
    fields.0.get_mut(&width).unwrap().text = "NaN".into();
    assert!(fields.resolve(&d).is_err());
    fields.0.get_mut(&width).unwrap().text = "3".into();
    fields.0.get_mut(&width).unwrap().unit = Unit::Degree.symbol().into();
    assert!(fields.resolve(&d).is_err());
    let mut removed = d.clone();
    removed.exposed.clear();
    assert!(fields.resolve(&removed).is_err());
    fields.0.get_mut(&width).unwrap().enabled = false;
    assert!(fields.resolve(&removed).unwrap().is_empty());
    let mut conflicting = d.clone();
    conflicting.name = "Different content at same revision".into();
    let mut c = controller();
    assert!(c.set_library_snapshot(vec![d, conflicting]).is_err());
    assert!(c.library.is_empty());
}

#[test]
fn constrained_sketch_extrusion_and_linked_plan_keep_local_ids_through_native_place_apply() {
    use multicad_core::{constraint_sketch::source::Source, model::ExtrudeRequest, sketch::Datum};
    let (sketch, edits) = Source::create(Datum::Xy, "Cabinet footprint").unwrap();
    let document = Document::new("Constrained symbol")
        .unwrap()
        .edited(&edits)
        .unwrap();
    let mut source = Source::read(&document, sketch).unwrap().0;
    source.add_rectangle([0., 0.], [30., 20.]).unwrap();
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut s = Session::new(document).unwrap();
    let profiles =
        multicad_solver::evaluate_document_sketch(s.document(), s.revision(), sketch, None)
            .unwrap()
            .regions()
            .unwrap()
            .regions
            .iter()
            .map(|r| r.intent())
            .collect();
    let extrude = ExtrudeRequest::new(sketch, profiles, Default::default());
    let body = extrude.outputs[0].body;
    s.commit(
        model::prepare_extrude(&s, s.revision(), actor(), &extrude)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut c = controller();
    begin(
        &mut c,
        &s,
        SymbolTool::Create,
        SymbolSelection {
            component: Some(body),
            plan_sketch: Some(sketch),
            ..Default::default()
        },
    );
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    let SymbolOutcome::Applied { selected, .. } = c.apply(&mut s, &Default::default()).unwrap()
    else {
        panic!()
    };
    let definition = selected[0];
    begin(&mut c, &s, SymbolTool::Place, Default::default());
    c.form
        .as_mut()
        .unwrap()
        .place(s.document(), definition)
        .unwrap();
    let before = s.document().clone();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    assert_eq!(s.document(), &before);
    let SymbolOutcome::Applied { selected, .. } = c.apply(&mut s, &Default::default()).unwrap()
    else {
        panic!()
    };
    let instance = selected[0];
    let ownership = symbols::read_instance(s.document(), instance).unwrap();
    let definition = symbols::read_definition(s.document(), definition).unwrap();
    assert!(
        ownership.ids.len() > definition.template.entities().count(),
        "The symbol owns local curve/point/constraint IDs as well as Document entities"
    );
    assert!(
        ownership
            .ids
            .values()
            .any(|id| s.document().get(*id).is_none())
    );
    let (outer, plan) = symbols::symbol_plan(s.document(), instance)
        .unwrap()
        .unwrap();
    assert_eq!(outer, instance);
    assert_eq!(plan, ownership.ids[&sketch]);
    let mapped = Source::read(s.document(), plan).unwrap().0;
    assert_eq!(mapped.sketch.curves.len(), source.sketch.curves.len());
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    s.redo(s.revision(), actor()).unwrap();
    multicad_model::Compiler::default()
        .compile(s.document(), s.revision())
        .unwrap();
}

#[test]
fn loaded_library_import_is_explicit_and_uses_cached_current_snapshot_without_mutating_source() {
    let (mut s, part, width) = fixture();
    let definition = embed(&mut s, part.body, width);
    let mut external = symbols::read_definition(s.document(), definition).unwrap();
    external.symbol = EntityId::new();
    external.name = "External cabinet".into();
    let mut c = controller();
    c.set_library_snapshot(vec![external.clone()]).unwrap();
    begin(&mut c, &s, SymbolTool::Library, Default::default());
    let baseline = s.document().clone();
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 800.);
    click(
        &ctx,
        &mut c,
        &mut s,
        point(&output, "Import definition"),
        800.,
    );
    assert!(c.dirty());
    assert!(matches!(&c.form.as_ref().unwrap().operation,Operation::Import(d)if d==&external));
    assert_eq!(s.document(), &baseline);
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    assert_eq!(s.document(), &baseline);
    c.apply(&mut s, &Default::default()).unwrap();
    assert_eq!(symbols::definitions(s.document()).unwrap().len(), 2);
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &baseline);
}
#[test]
fn removed_override_requires_explicit_visible_resolution_before_native_upgrade() {
    let (mut s, part, width) = fixture();
    let definition = embed(&mut s, part.body, width);
    let place = symbols::SymbolPlaceRequest::new(
        s.document(),
        definition,
        "Cabinet A",
        BTreeMap::from([(
            width,
            SymbolValue {
                value: 30.,
                unit: "mm".into(),
            },
        )]),
        Default::default(),
    )
    .unwrap();
    s.commit(
        symbols::prepare_place(&s, s.revision(), actor(), &place)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let mut replacement = symbols::read_definition(s.document(), definition).unwrap();
    replacement.revision = "no-width-input".into();
    replacement.exposed.clear();
    let baseline = s.document().clone();
    let mut c = controller();
    begin(
        &mut c,
        &s,
        SymbolTool::Update,
        SymbolSelection {
            instance: Some(place.instance),
            ..Default::default()
        },
    );
    c.form
        .as_mut()
        .unwrap()
        .set_replacement(s.document(), replacement)
        .unwrap();
    assert!(c.preview(&s, &Default::default(), Channel::Ui).is_err());
    assert!(!c.can_apply(&s, &Default::default()));
    assert_eq!(s.document(), &baseline);
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut s, vec![], 1200.);
    click(
        &ctx,
        &mut c,
        &mut s,
        point(&output, "Override width"),
        1200.,
    );
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()), "{:?}", c.error);
    c.apply(&mut s, &Default::default()).unwrap();
    assert!(
        symbols::read_instance(s.document(), place.instance)
            .unwrap()
            .overrides
            .is_empty()
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &baseline);
}
#[test]
fn static_inputs_are_fixed_and_incompatible_override_reset_reloads_the_authored_default() {
    let (mut s, part, width) = fixture();
    let id = embed(&mut s, part.body, width);
    let mut d = symbols::read_definition(s.document(), id).unwrap();
    let mut values = super::fields::Overrides::new(
        &d,
        &BTreeMap::from([(
            width,
            SymbolValue {
                value: 42.,
                unit: "deg".into(),
            },
        )]),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let render = |values: &mut super::fields::Overrides, events| {
        ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(250., 400.),
                )),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                values.show(ui, &d);
            },
        )
    };
    let output = render(&mut values, vec![]);
    let reset = point(&output, "Use new default");
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        render(
            &mut values,
            vec![
                egui::Event::PointerMoved(reset),
                egui::Event::PointerButton {
                    pos: reset,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
    assert!(!values.0[&width].enabled);
    assert_eq!(values.0[&width].text, "10");
    assert_eq!(values.0[&width].unit, "mm");
    assert!(values.resolve(&d).unwrap().is_empty());
    d.kind = SymbolKind::Static;
    d.exposed.clear();
    assert!(
        super::fields::Overrides::new(&d, &BTreeMap::new())
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn actual_native_geometry_without_physical_occurrence_receipt_cannot_authorize_symbol_apply() {
    use crate::viewport::Scene;
    let (mut s, part, width) = fixture();
    let definition = embed(&mut s, part.body, width);
    let mut c = controller();
    begin(&mut c, &s, SymbolTool::Place, Default::default());
    c.form
        .as_mut()
        .unwrap()
        .place(s.document(), definition)
        .unwrap();
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    wait(&mut c, &s, &Default::default());
    assert!(c.can_apply(&s, &Default::default()));
    let native = c.display_scene(&s, &Default::default()).unwrap();
    // Retain genuinely native triangles, source hash and full-scope marker while
    // deliberately withholding the independent physical occurrence receipt.
    let incomplete = Scene::new(
        native.stamp(),
        native.geometries().values().cloned().collect(),
        native.instances().to_vec(),
    )
    .unwrap()
    .with_model_evaluation(native.model_evaluation().unwrap().clone())
    .unwrap();
    let request = c.form.as_mut().unwrap().prepare(&s, actor()).unwrap();
    let baseline = s.document().clone();
    let error = super::proof::validate(request.prepared.document(), native.stamp(), &incomplete)
        .unwrap_err();
    assert!(error.contains("leaf-occurrence"), "{error}");
    assert_eq!(s.document(), &baseline);
    let mut resources = ProjectResources::default();
    resources
        .metadata
        .insert("new".into(), serde_json::json!(true));
    c.preview(&s, &Default::default(), Channel::Ui).unwrap();
    c.poll(&s, &resources);
    assert!(!c.can_apply(&s, &resources));
    assert!(!c.pending());
    assert_eq!(s.document(), &baseline);
}
