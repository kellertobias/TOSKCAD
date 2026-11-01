use super::*;
use multicad_core::{constraint_sketch::Dimension, model, parameters::BoundExpression, symbols::*};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct State {
    library: Option<ProjectFile>,
    writes: Vec<ProjectFile>,
}
struct Storage(Arc<Mutex<State>>);
impl DocumentStorage for Storage {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        Err("Unused legacy path".into())
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        Err("Unused legacy path".into())
    }
    fn open_project(&mut self, _: &Path) -> Result<crate::storage::LoadedProject, StorageError> {
        Ok(crate::storage::LoadedProject {
            file: self
                .0
                .lock()
                .unwrap()
                .library
                .clone()
                .ok_or("Missing library".to_string())?,
            stamp: None,
            diagnostics: vec![],
        })
    }
    fn save_project(
        &mut self,
        _: &Path,
        project: &ProjectFile,
        mode: SaveMode,
    ) -> Result<crate::storage::SavedProject, StorageError> {
        assert_eq!(mode, SaveMode::Create);
        project.validate()?;
        self.0.lock().unwrap().writes.push(project.clone());
        Ok(crate::storage::SavedProject {
            stamp: FileStamp {
                manifest: multicad_core::project_file::ContentHash::document(&project.document)?,
                length: 1,
            },
            warnings: vec![],
        })
    }
}
fn seed() -> (Session, EntityId, EntityId, SymbolPlaceRequest) {
    let mut session = Session::new(Document::new("Symbol host checks").unwrap()).unwrap();
    let width = Entity::new(
        "width",
        EntityData::Parameter {
            expression: "60".into(),
            unit: "mm".into(),
            exposed: true,
            inputs: vec![],
        },
    );
    session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Parameter".into(),
            edits: vec![Mutation::Insert(width.clone())],
        })
        .unwrap();
    let stock = model::PrimitiveRequest::new(
        "Cabinet",
        model::PrimitiveParameters::Box {
            size: [
                Dimension::Expression {
                    expression: BoundExpression::bind(session.document(), "=$width", "mm", None)
                        .unwrap(),
                },
                model::length(40.),
                model::length(12.),
            ],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &stock,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let author = SymbolAuthorRequest::new(
        "Cabinet",
        stock.body,
        SymbolKind::Dynamic,
        vec![width.id],
        None,
    );
    let definition = author.definition;
    session
        .commit(
            prepare_author(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                &author,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    let request = SymbolPlaceRequest::new(
        session.document(),
        definition,
        "Placed cabinet",
        BTreeMap::from([(
            width.id,
            SymbolValue {
                value: 80.,
                unit: "mm".into(),
            },
        )]),
        Default::default(),
    )
    .unwrap();
    session
        .commit(
            prepare_place(
                &session,
                session.revision(),
                Actor::local(Channel::Ui),
                &request,
            )
            .unwrap()
            .prepared,
        )
        .unwrap();
    (session, width.id, definition, request)
}
fn app(document: Document) -> (Application, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut app = Application::with_document(Box::new(Storage(state.clone())), document).unwrap();
    app.pending_files.clear();
    (app, state)
}
fn drain(app: &mut Application) {
    let until = Instant::now() + Duration::from_secs(5);
    while !app.pending_files.is_empty() {
        app.poll_files();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn generic_parameter_table_preserves_private_symbol_parameters_and_overrides() {
    let (mut session, width, definition, request) = seed();
    let owned = owned_entities(session.document()).unwrap();
    let private = request.ids[&width];
    assert!(owned.contains(&private));
    let before = session.document().get(private).unwrap().clone();
    let mut editor = ParameterEditor::new(&session);
    assert!(editor.rows.iter().any(|row| row.id == width));
    assert!(!editor.rows.iter().any(|row| owned.contains(&row.id)));
    editor
        .rows
        .iter_mut()
        .find(|row| row.id == width)
        .unwrap()
        .expression = "70".into();
    editor.apply(&mut session, Channel::Ui).unwrap();
    assert_eq!(session.document().get(private), Some(&before));
    assert_eq!(
        read_instance(session.document(), request.instance)
            .unwrap()
            .overrides[&width]
            .value,
        80.
    );
    assert_eq!(
        read_definition(session.document(), definition)
            .unwrap()
            .revision,
        "1"
    );
}
#[test]
fn library_save_exports_only_pinned_definition_and_does_not_acknowledge_project_save() {
    let (session, _, definition, _) = seed();
    let (mut app, state) = app(session.document().clone());
    let before = app.current().session.document().clone();
    let revision = app.current().session.revision();
    assert!(app.current().session.dirty());
    app.file_document = Some(app.current().local_id);
    app.file_action = Some(FileAction::SymbolSave(definition));
    app.save_symbol_library("/tmp/symbol-library-review.multicad".into(), definition)
        .unwrap();
    drain(&mut app);
    let saved = state.lock().unwrap().writes[0].clone();
    assert_eq!(saved.document.entities().count(), 1);
    assert_eq!(definitions(&saved.document).unwrap().len(), 1);
    assert_eq!(
        read_definition(&saved.document, definition)
            .unwrap()
            .pin()
            .unwrap(),
        read_definition(&before, definition).unwrap().pin().unwrap()
    );
    assert_eq!(app.current().session.document(), &before);
    assert_eq!(app.current().session.revision(), revision);
    assert!(app.current().session.dirty());
    assert!(app.current().path.is_none());
    assert!(app.file_action.is_none());
}
#[test]
fn library_load_is_read_only_and_stale_target_cannot_receive_it() {
    let (session, _, definition, _) = seed();
    let (mut app, state) = app(session.document().clone());
    state.lock().unwrap().library = Some(ProjectFile::new(session.document().clone()).unwrap());
    let before = app.current().session.document().clone();
    app.file_action = Some(FileAction::SymbolOpen);
    app.load_symbol_library("/tmp/symbol-library-review.multicad".into())
        .unwrap();
    drain(&mut app);
    assert_eq!(app.projects.len(), 1);
    assert_eq!(app.current().session.document(), &before);
    assert!(app.file_action.is_none());
    let p = app.current();
    let guard = p
        .session
        .prepare(Request {
            expected_revision: p.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Read library".into(),
            edits: vec![],
        })
        .unwrap();
    let pending = PendingFile::SymbolOpen {
        local: p.local_id,
        guard,
        resources: p.resources.clone(),
    };
    let project = ProjectFile::new(before.clone()).unwrap();
    let p = app.current_mut();
    p.session
        .apply(Request {
            expected_revision: p.session.revision(),
            actor: Actor::local(Channel::Ui),
            label: "Rename".into(),
            edits: vec![Mutation::RenameDocument("Changed".into())],
        })
        .unwrap();
    assert!(
        app.accept_file_reply(
            pending,
            FileResult::Loaded(Ok(Box::new(crate::storage::LoadedProject {
                file: project,
                stamp: None,
                diagnostics: vec![]
            })))
        )
        .is_err()
    );
    assert_eq!(app.projects.len(), 1);
    assert_eq!(app.current().session.document().name(), "Changed");
    assert_eq!(
        definitions(app.current().session.document()).unwrap()[0].0,
        definition
    );
}
#[test]
fn generated_symbol_source_is_not_offered_as_an_independent_sketch_or_feature_edit() {
    let (session, _, _, request) = seed();
    let mut editor = SketchEditor::default();
    let feature = request
        .ids
        .values()
        .copied()
        .find(|id| {
            matches!(
                session.document().get(*id).map(|e| &e.data),
                Some(EntityData::Feature { .. })
            )
        })
        .unwrap();
    editor.set_model_backend(Box::new(
        crate::modeling::native_fixture::NativeBackend::new(),
    ));
    assert!(!editor.feature_edit_ready(&session, &Default::default(), feature));
}

#[test]
fn another_tool_closes_read_only_library_without_leaving_a_hidden_inspector_owner() {
    let (session, _, _, _) = seed();
    let (mut app, _) = app(session.document().clone());
    let factory: crate::viewport::BackendFactory =
        Arc::new(|| Box::new(crate::modeling::native_fixture::NativeBackend::new()));
    app.set_scene_backends(factory.clone());
    app.set_model_backends(factory);
    app.mode = Mode::Model;
    let mechanical = app
        .registry
        .workspace(Mode::Model)
        .tabs
        .iter()
        .position(|tab| tab.title == "Mechanical")
        .unwrap();
    app.tab = mechanical;
    app.invoke(Command::Extension("library"), &[], Channel::Ui)
        .unwrap();
    assert_eq!(app.tab, mechanical);
    assert!(app.current().sketch.symbol_active());
    assert!(!app.current().pending_edits());
    assert!(app.enabled(Command::Extension("symbol")));
    app.invoke(Command::Extension("symbol"), &[], Channel::Ui)
        .unwrap();
    assert!(app.current().sketch.symbol_active());
    assert!(app.current().pending_edits());
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    app.invoke(Command::Extension("library"), &[], Channel::Ui)
        .unwrap();
    app.invoke(
        Command::Modeling(crate::modeling::ModelingTool::Box),
        &[],
        Channel::Ui,
    )
    .unwrap();
    assert!(!app.current().sketch.symbol_active());
    assert!(app.current().sketch.model_active());
}
