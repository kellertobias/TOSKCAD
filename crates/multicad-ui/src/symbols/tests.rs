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
#[test]
fn installed_library_load_is_explicit_filtered_and_source_neutral() {
    use crate::library::LibraryEntry;
    use std::sync::Arc;
    let (mut session, _, _) = fixture();
    let before = session.document().clone();
    let revision = session.revision();
    let entry = LibraryEntry {
        origin: crate::library::LibraryOrigin::ListedOriginal,
        id: EntityId::new(),
        document: EntityId::new(),
        path: "/offline/library/custom-washer.tcd".into(),
        name: "Custom washer".into(),
        description: "Steel ring".into(),
        preview: None,
        registered_unix_ms: 1,
        symbol_definitions: 1,
        mechanical_parts: Vec::new(),
        problem: None,
    };
    let mut missing = entry.clone();
    missing.id = EntityId::new();
    missing.name = "Missing washer".into();
    missing.problem = Some("Original file is missing".into());
    let mut plain = entry.clone();
    plain.id = EntityId::new();
    plain.name = "Plain washer".into();
    plain.symbol_definitions = 0;
    let mut c = controller();
    c.set_installed_library(Arc::new(vec![entry.clone(), missing, plain]));
    begin(&mut c, &session, SymbolTool::Library, Default::default());
    let ctx = egui::Context::default();
    c.form.as_mut().unwrap().filter = "STEEL custom".into();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    let button = point(&output, "Load definitions");
    output.drop_without_applying_deltas();
    click(&ctx, &mut c, &mut session, button, 1800.);
    assert_eq!(
        c.take_action(),
        Some(SymbolAction::OpenInstalled(entry.path.clone()))
    );
    assert_eq!(c.take_action(), None);
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    let place = point(&output, "Place");
    output.drop_without_applying_deltas();
    click(&ctx, &mut c, &mut session, place, 1800.);
    assert_eq!(
        c.take_action(),
        Some(SymbolAction::OpenInstalledForPlace(entry.path))
    );
    assert_eq!(c.take_action(), None);
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    assert!(!c.pending());
    c.form.as_mut().unwrap().filter = "missing".into();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    point(&output, "Original file is missing");
    output.drop_without_applying_deltas();
    c.form.as_mut().unwrap().filter = "plain".into();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    point(
        &output,
        "No reusable definitions. Open this file and use Create symbol before placing it here.",
    );
    output.drop_without_applying_deltas();
}
#[test]
fn loaded_hardware_definitions_remain_findable_by_their_source_dimensions() {
    let mut source = Session::new(Document::new("Hardware file").unwrap()).unwrap();
    let request = model::MechanicalPartRequest::new(
        "Ring",
        model::MechanicalPartParameters::Washer {
            bore_diameter: length(3.2),
            outside_diameter: length(7.),
            thickness: length(0.5),
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_mechanical_part(&source, 0, actor(), &request).unwrap();
    source.commit(prepared.prepared).unwrap();
    let author = SymbolAuthorRequest::new(
        "Reusable ring",
        request.body,
        SymbolKind::Static,
        vec![],
        None,
    );
    let prepared = symbols::prepare_author(&source, source.revision(), actor(), &author).unwrap();
    source.commit(prepared.prepared).unwrap();
    let definition = symbols::read_definition(source.document(), author.definition).unwrap();
    let (mut session, _, _) = fixture();
    let before = session.document().clone();
    let mut c = controller();
    c.set_library_snapshot(vec![definition]).unwrap();
    begin(&mut c, &session, SymbolTool::Library, Default::default());
    c.form.as_mut().unwrap().filter = "washer 3.2".into();
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    let import = point(&output, "Import definition");
    let place = point(&output, "Place");
    let dimensions = point(&output, "Bore: 3.2 mm");
    assert!(
        place.y < dimensions.y,
        "Place must precede the card's dimension details"
    );
    assert!(
        import.y < dimensions.y,
        "Explicit Import must remain beside the card actions"
    );
    output.drop_without_applying_deltas();
    click(&ctx, &mut c, &mut session, import, 1800.);
    assert!(matches!(
        c.form.as_ref().unwrap().operation,
        Operation::Import(_)
    ));
    assert_eq!(
        session.document(),
        &before,
        "choosing Import still requires native Preview and Apply"
    );
}
fn catalog_search_fixture(
    name: &str,
    nominal: Option<f64>,
    bore: f64,
    material: &str,
) -> symbols::SymbolDefinition {
    let mut source =
        Session::new(Document::new("Synthetic catalog-search fixture").unwrap()).unwrap();
    let geometry = model::MechanicalPartParameters::Washer {
        bore_diameter: length(bore),
        outside_diameter: length(bore + 4.),
        thickness: length(0.5),
        frame: Default::default(),
    };
    let mut request =
        model::MechanicalPartRequest::new(format!("Physical ring for {name}"), geometry.clone());
    request.provenance = Some(model::MechanicalPartProvenance {
        catalog_id: format!("test-only/{name}"),
        revision: "fixture-1".into(),
        designation: format!("Fixture {name}"),
        source_url: "https://example.invalid/test-only-catalog".into(),
        reference_geometry: geometry,
        catalog_facts: nominal.map(|nominal| model::MechanicalPartCatalogFacts {
            nominal_fastener_diameter_mm: Some(nominal),
            series: Some("Fixture normal series".into()),
            material: Some(material.into()),
            grade: None,
        }),
    });
    let p = model::prepare_mechanical_part(&source, 0, actor(), &request).unwrap();
    source.commit(p.prepared).unwrap();
    let author = SymbolAuthorRequest::new(name, request.body, SymbolKind::Static, vec![], None);
    let p = symbols::prepare_author(&source, source.revision(), actor(), &author).unwrap();
    source.commit(p.prepared).unwrap();
    symbols::read_definition(source.document(), author.definition).unwrap()
}
fn painted_text_count(output: &egui::FullOutput, text: &str) -> usize {
    fn count(shape: &egui::Shape, text: &str) -> usize {
        match shape {
            egui::Shape::Text(value) => usize::from(value.galley.text() == text),
            egui::Shape::Vec(shapes) => shapes.iter().map(|s| count(s, text)).sum(),
            _ => 0,
        }
    }
    output.shapes.iter().map(|s| count(&s.shape, text)).sum()
}
#[test]
fn sourced_nominal_search_is_exact_across_installed_embedded_and_loaded_definitions() {
    use crate::library::{LibraryEntry, LibraryOrigin};
    use std::sync::Arc;
    let definitions = vec![
        catalog_search_fixture("Three fixture", Some(3.), 3.2, "Fixture steel"),
        catalog_search_fixture("Decimal fixture", Some(3.5), 3.7, "Fixture steel"),
        catalog_search_fixture("Thirty fixture", Some(30.), 31., "Fixture brass"),
        catalog_search_fixture("Unattributed M3 fixture", None, 3.2, "Unused material"),
    ];
    let entries: Vec<_> = definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| {
            let mut cards = model::mechanical_part_cards(&definition.template);
            for card in &mut cards {
                card.symbol = Some((definition.symbol, definition.revision.clone()));
            }
            LibraryEntry {
                origin: LibraryOrigin::ListedOriginal,
                id: EntityId::new(),
                document: definition.template.id(),
                path: format!("/offline/library/M3-M30-fixture-{index}.tcd").into(),
                name: format!("Installed {}", definition.name),
                description: "Synthetic search fixtures, not product dimensions".into(),
                preview: None,
                registered_unix_ms: 1,
                symbol_definitions: 1,
                mechanical_parts: cards,
                problem: None,
            }
        })
        .collect();
    let mut session = Session::new(Document::new("Search host").unwrap()).unwrap();
    for definition in &definitions {
        let prepared =
            symbols::prepare_import(&session, session.revision(), actor(), definition).unwrap();
        session.commit(prepared.prepared).unwrap();
    }
    let before = session.document().clone();
    let revision = session.revision();
    let mut c = controller();
    c.set_installed_library(Arc::new(entries.clone()));
    c.set_library_snapshot(definitions.clone()).unwrap();
    begin(&mut c, &session, SymbolTool::Library, Default::default());
    let ctx = egui::Context::default();
    for (query, selected) in [
        ("M3", Some(0)),
        ("m3 steel normal", Some(0)),
        ("M3.5", Some(1)),
        ("M30", Some(2)),
        ("M3 brass", None),
        ("M3 31", None),
        ("M3 3.7", None),
        ("M3 M30", None),
        ("M4.5", None),
        ("M9", None),
    ] {
        c.form.as_mut().unwrap().filter = query.into();
        let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 6000.);
        for (index, definition) in definitions.iter().enumerate() {
            let expected = usize::from(selected == Some(index));
            assert_eq!(
                painted_text_count(&output, &format!("Installed {}", definition.name)),
                expected,
                "Installed {query}"
            );
            assert_eq!(
                painted_text_count(&output, &definition.name),
                expected * 2,
                "Embedded and loaded {query}"
            );
        }
        if selected == Some(0) {
            assert_eq!(
                painted_text_count(&output, "Source nominal fastener diameter: 3 mm"),
                3
            );
            assert_eq!(
                painted_text_count(&output, "Source grade: not specified"),
                3
            );
        }
        output.drop_without_applying_deltas();
    }
    c.form.as_mut().unwrap().filter = "washer 3.2".into();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 6000.);
    for index in [0, 3] {
        assert_eq!(painted_text_count(&output, &definitions[index].name), 2);
    }
    output.drop_without_applying_deltas();
    let mut changed = entries;
    changed[0].mechanical_parts[0]
        .catalog_facts
        .as_mut()
        .unwrap()
        .nominal_fastener_diameter_mm = Some(30.);
    c.set_installed_library(Arc::new(changed));
    c.form.as_mut().unwrap().filter = "M3".into();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 6000.);
    assert_eq!(
        painted_text_count(&output, "Installed Three fixture"),
        0,
        "Updated Library snapshot must refresh its cached facts"
    );
    assert_eq!(
        painted_text_count(&output, "Three fixture"),
        2,
        "Embedded and loaded pins keep their own facts"
    );
    output.drop_without_applying_deltas();
    assert_eq!(session.document(), &before);
    assert_eq!(session.revision(), revision);
    assert!(!c.pending());
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

#[test]
fn graphical_architectural_authoring_keeps_live_construction_and_public_slot_ids_across_revisions()
{
    use super::architecture::test_support;
    use multicad_core::symbols::architecture::*;
    let (original, interface) = test_support::definition(ArchitectureRole::Fence);
    // The authoring document keeps its live canonical graph. The exported
    // fixture's closed template has that same graph and document identity.
    let mut session = Session::new(original.template.clone()).unwrap();
    let imported = symbols::prepare_import(&session, 0, actor(), &original).unwrap();
    session.commit(imported.prepared).unwrap();
    let mut form = forms::Form::new(
        SymbolTool::Create,
        session.document(),
        SymbolSelection::default(),
    )
    .unwrap();
    form.author_construction(session.document(), interface.construction)
        .unwrap();
    let forms::Operation::ArchitecturalAuthor { request, .. } = &form.operation else {
        panic!("Expected the graphical interface author")
    };
    assert_eq!(request.blueprint, original.root);
    assert_eq!(request.interface, interface);
    assert_eq!(request.symbol.symbol, original.symbol);
    assert_ne!(request.symbol.revision, original.revision);
    let before = session.document().clone();
    let prepared = form.prepare(&session, actor()).unwrap();
    let incoming =
        symbols::read_definition(prepared.prepared.document(), prepared.definition).unwrap();
    assert_eq!(
        blueprint_for_definition(&incoming).unwrap().unwrap(),
        interface
    );
    assert_eq!(incoming.root, original.root);
    assert_eq!(incoming.symbol, original.symbol);
    assert_eq!(session.document(), &before);
    session.commit(prepared.prepared).unwrap();
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn library_cards_count_typed_construction_inputs_and_preserve_ordinary_symbol_summaries() {
    use super::architecture::test_support;
    use multicad_core::symbols::architecture::*;
    let (mut wall, mut blueprint) = test_support::definition(ArchitectureRole::Wall);
    blueprint.materials.clear();
    let mut root = wall.template.get(wall.root).unwrap().clone();
    root.data = EntityData::Extension {
        definition: blueprint.definition().unwrap(),
        inputs: blueprint.inputs(),
    };
    wall.template = wall.template.edited(&[Mutation::Replace(root)]).unwrap();
    wall.validate().unwrap();
    let (fence, _) = test_support::definition(ArchitectureRole::Fence);
    let (mut session, part, width) = fixture();
    let ordinary_id = embed(&mut session, part.body, width);
    let ordinary = symbols::read_definition(session.document(), ordinary_id).unwrap();
    let mut c = controller();
    c.set_library_snapshot(vec![wall, fence, ordinary]).unwrap();
    begin(&mut c, &session, SymbolTool::Library, Default::default());
    let before = session.document().clone();
    let (output, _) = frame(
        &egui::Context::default(),
        &mut c,
        &mut session,
        vec![],
        2400.,
    );
    for label in [
        "1 exposed inputs · Path-driven wall",
        "3 exposed inputs · Path-driven fence",
        "1 exposed inputs · 3D",
    ] {
        point(&output, label);
    }
    assert_eq!(session.document(), &before);
}

#[test]
fn wall_update_paints_changed_typed_defaults_and_keeps_the_receiving_override() {
    use super::architecture::test_support;
    use multicad_core::symbols::architecture::*;
    let (old, blueprint) = test_support::definition(ArchitectureRole::Wall);
    let (mut session, definition) = test_support::consumer(&old);
    let mut inputs = ArchitectureInputs::default();
    inputs
        .dimensions
        .insert(blueprint.dimensions[0].id, length(2800.));
    let place = ArchitecturePlaceRequest::new(
        session.document(),
        definition,
        "Receiving wall",
        inputs.clone(),
        ArchitectureContext::Wall {
            path: vec![[length(0.), length(0.)], [length(4000.), length(0.)]],
            closed: false,
            level: None,
            base_z: length(0.),
            flipped: false,
            details: None,
        },
    )
    .unwrap();
    session
        .commit(
            prepare_architecture_place(&session, session.revision(), actor(), &place)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let baseline = session.document().clone();
    let mut next = old.clone();
    next.revision = "2".into();
    let model::WallTypeConstruction::Layered(mut construction) =
        model::read_wall_construction(&next.template, blueprint.construction).unwrap()
    else {
        unreachable!()
    };
    construction.height = length(2900.);
    let mut entity = next.template.get(blueprint.construction).unwrap().clone();
    let EntityData::Extension { definition, .. } = &mut entity.data else {
        unreachable!()
    };
    definition.data = serde_json::to_value(construction).unwrap();
    next.template = next.template.edited(&[Mutation::Replace(entity)]).unwrap();
    let mut c = controller();
    begin(
        &mut c,
        &session,
        SymbolTool::Update,
        SymbolSelection {
            instance: Some(place.output.anchors()[1]),
            ..Default::default()
        },
    );
    c.form
        .as_mut()
        .unwrap()
        .set_replacement(session.document(), next)
        .unwrap();
    let (output, _) = frame(
        &egui::Context::default(),
        &mut c,
        &mut session,
        vec![],
        2400.,
    );
    for label in [
        "Changed construction inputs/defaults",
        "Height",
        "Before default: 2700 mm",
        "After default: 2900 mm",
        "Override Height",
        "Authored default: 2900 mm",
    ] {
        point(&output, label);
    }
    assert_eq!(session.document(), &baseline);
    c.preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut c, &session, &Default::default());
    c.apply(&mut session, &Default::default()).unwrap();
    let record = read_architecture_instance(session.document(), place.record).unwrap();
    assert_eq!(record.inputs, inputs);
    assert_eq!(record.ids, place.ids);
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &baseline);
}

#[test]
fn painted_architectural_exposure_toggles_only_the_interface_and_native_apply_is_atomic() {
    use super::architecture::test_support;
    use multicad_core::symbols::architecture::*;
    let (definition, interface) = test_support::definition(ArchitectureRole::Fence);
    let mut session = Session::new(definition.template.clone()).unwrap();
    let before = session.document().clone();
    let mut controller = controller();
    begin(
        &mut controller,
        &session,
        SymbolTool::Create,
        SymbolSelection {
            component: Some(interface.construction),
            ..Default::default()
        },
    );
    let context = egui::Context::default();
    let (output, _) = frame(&context, &mut controller, &mut session, vec![], 1600.);
    let height = point(&output, "Height");
    click(&context, &mut controller, &mut session, height, 1600.);
    let Operation::ArchitecturalAuthor { request, .. } =
        &controller.form.as_ref().unwrap().operation
    else {
        panic!("Expected construction interface controls")
    };
    assert!(
        !request
            .interface
            .dimensions
            .iter()
            .any(|slot| slot.target == ArchitectureDimensionTarget::FenceHeight)
    );
    assert_eq!(session.document(), &before);
    controller
        .preview(&session, &Default::default(), Channel::Ui)
        .unwrap();
    wait(&mut controller, &session, &Default::default());
    assert!(
        controller.can_apply(&session, &Default::default()),
        "{:?}",
        controller.error
    );
    let (output, _) = frame(&context, &mut controller, &mut session, vec![], 1600.);
    let outcome = click(
        &context,
        &mut controller,
        &mut session,
        point(&output, "Apply"),
        1600.,
    )
    .unwrap();
    let SymbolOutcome::Applied { selected, .. } = outcome else {
        panic!("Expected one atomic authoring application")
    };
    let published = symbols::read_definition(session.document(), selected[0]).unwrap();
    assert!(
        !blueprint_for_definition(&published)
            .unwrap()
            .unwrap()
            .dimensions
            .iter()
            .any(|slot| slot.target == ArchitectureDimensionTarget::FenceHeight)
    );
    assert_eq!(
        model::read_fence_type(session.document(), interface.construction).unwrap(),
        model::read_fence_type(&before, interface.construction).unwrap()
    );
    session.undo(session.revision(), actor()).unwrap();
    assert_eq!(session.document(), &before);
}

#[test]
fn loaded_library_direct_place_selects_detached_pin_and_cancel_then_apply_in_explicit_parent() {
    let (mut author, part, width) = fixture();
    let id = embed(&mut author, part.body, width);
    let definition = symbols::read_definition(author.document(), id).unwrap();
    let mut other = definition.clone();
    other.symbol = EntityId::new();
    other.name = "Different reusable definition".into();
    let parent = Entity::new(
        "Receiving assembly",
        EntityData::Assembly { instances: vec![] },
    );
    let parent_id = parent.id;
    let document = Document::new("Receiver")
        .unwrap()
        .edited(&[Mutation::Insert(parent)])
        .unwrap();
    let mut session = Session::new(document.clone()).unwrap();
    let mut c = controller();
    c.set_library_snapshot(vec![other, definition.clone()])
        .unwrap();
    let ctx = egui::Context::default();
    for apply in [false, true] {
        begin(
            &mut c,
            &session,
            SymbolTool::Library,
            SymbolSelection {
                parent: Some(parent_id),
                ..Default::default()
            },
        );
        c.form.as_mut().unwrap().filter = "Cabinet symbol".into();
        let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
        let place = point(&output, "Place");
        output.drop_without_applying_deltas();
        click(&ctx, &mut c, &mut session, place, 1800.);
        assert!(
            matches!(&c.form.as_ref().unwrap().operation, Operation::Place { library: Some(source), parent: Some(parent), .. } if source == &definition && *parent == parent_id)
        );
        assert_eq!(session.document(), &document);
        assert!(symbols::definitions(session.document()).unwrap().is_empty());
        let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
        point(&output, "Parent: Receiver / Receiving assembly");
        let preview = point(&output, "Preview");
        output.drop_without_applying_deltas();
        click(&ctx, &mut c, &mut session, preview, 1800.);
        wait(&mut c, &session, &Default::default());
        assert!(c.can_apply(&session, &Default::default()), "{:?}", c.error);
        assert_eq!(session.document(), &document);
        let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
        let button = point(&output, if apply { "Apply" } else { "Cancel" });
        output.drop_without_applying_deltas();
        let outcome = click(&ctx, &mut c, &mut session, button, 1800.).unwrap();
        if apply {
            let SymbolOutcome::Applied { selected, .. } = outcome else {
                panic!("expected applied placement")
            };
            assert_eq!(session.revision(), 1);
            let instance = symbols::read_instance(session.document(), selected[0]).unwrap();
            assert_eq!(
                symbols::read_definition(session.document(), instance.definition).unwrap(),
                definition
            );
            assert_eq!(
                multicad_core::hierarchy::Index::new(session.document())
                    .unwrap()
                    .parent(instance.instance)
                    .unwrap(),
                Some(parent_id)
            );
            session.undo(1, actor()).unwrap();
            assert_eq!(session.document(), &document);
            assert!(!session.can_undo());
            session.redo(2, actor()).unwrap();
            assert!(symbols::read_instance(session.document(), selected[0]).is_ok());
        } else {
            assert!(matches!(outcome, SymbolOutcome::Cancelled));
            assert_eq!(session.document(), &document);
            assert_eq!(session.revision(), 0);
            assert!(!session.can_undo());
        }
    }
}

#[path = "placement_gizmo_tests.rs"]
mod placement_gizmo;

#[test]
fn embedded_place_precedes_source_dimensions_and_long_placement_parent_wraps_inside_inspector() {
    let definition =
        catalog_search_fixture("Detailed embedded ring", Some(3.), 3.2, "Fixture steel");
    let parent_name = "Assembly with a long explicit receiving context ".repeat(6);
    let parent = Entity::new(&parent_name, EntityData::Assembly { instances: vec![] });
    let parent_id = parent.id;
    let document = Document::new("Receiver")
        .unwrap()
        .edited(&[Mutation::Insert(parent)])
        .unwrap();
    let mut session = Session::new(document).unwrap();
    session
        .commit(
            symbols::prepare_import(&session, 0, actor(), &definition)
                .unwrap()
                .prepared,
        )
        .unwrap();
    let before = session.document().clone();
    let mut c = controller();
    begin(
        &mut c,
        &session,
        SymbolTool::Library,
        SymbolSelection {
            parent: Some(parent_id),
            ..Default::default()
        },
    );
    let ctx = egui::Context::default();
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    let place = point(&output, "Place");
    assert!(place.y < point(&output, "Bore: 3.2 mm").y);
    output.drop_without_applying_deltas();
    click(&ctx, &mut c, &mut session, place, 1800.);
    let (output, _) = frame(&ctx, &mut c, &mut session, vec![], 1800.);
    let label = format!("Parent: Receiver / {parent_name}");
    fn rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(shape) if shape.galley.text() == label => {
                Some(egui::Rect::from_min_size(shape.pos, shape.galley.size()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| rect(shape, label)),
            _ => None,
        }
    }
    let parent = output
        .shapes
        .iter()
        .find_map(|shape| rect(&shape.shape, &label))
        .expect("full parent label painted");
    assert!(
        parent.height() > 30.,
        "long parent must wrap onto multiple lines"
    );
    assert!(
        parent.left() >= 0. && parent.right() <= 250.,
        "parent path must stay inside Inspector: {parent:?}"
    );
    output.drop_without_applying_deltas();
    assert_eq!(session.document(), &before);
}
