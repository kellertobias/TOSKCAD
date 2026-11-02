use super::*;
use multicad_core::{document::Document, project_file::ProjectResources};
use std::time::{Duration, Instant};

fn editor(s: &Session) -> LibraryEditor {
    let mut editor = LibraryEditor::default();
    editor.set_backend(Box::new(
        crate::modeling::native_fixture::NativeBackend::new(),
    ));
    editor.open(s);
    editor
}
fn preview(editor: &mut LibraryEditor, s: &Session) -> Result<(), String> {
    let resources = ProjectResources::default();
    editor.refresh_preview(s, &resources, Channel::Ui)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while editor.pending() {
        assert!(
            Instant::now() < deadline,
            "native definition preview timed out"
        );
        editor.poll(s, &resources);
        std::thread::sleep(Duration::from_millis(1));
    }
    if let Some(error) = &editor.error {
        return Err(error.clone());
    }
    assert!(editor.can_apply(s, &resources));
    Ok(())
}
fn apply(editor: &mut LibraryEditor, s: &mut Session) -> Result<(), String> {
    preview(editor, s)?;
    editor.apply(s, &Default::default(), Channel::Ui)
}
fn set_stock(editor: &mut LibraryEditor, f: impl FnOnce(&mut Stock)) {
    let d = editor.draft.as_mut().unwrap();
    let Some(LibraryDefinition::Stock(mut stock)) = read(&d.entity).unwrap() else {
        panic!("stock draft")
    };
    f(&mut stock);
    let definition = LibraryDefinition::Stock(stock);
    if let EntityData::Extension {
        definition: wire,
        inputs,
    } = &mut d.entity.data
    {
        wire.data = serde_json::to_value(&definition).unwrap();
        *inputs = definition.links();
    }
}
#[test]
fn native_material_preview_is_neutral_and_apply_and_history_keep_one_identity() {
    let mut s = Session::new(Document::new("Project materials").unwrap()).unwrap();
    let before = s.document().clone();
    let mut editor = editor(&s);
    editor.create(&s, 0);
    let id = editor.draft.as_ref().unwrap().entity.id;
    editor.draft.as_mut().unwrap().entity.name = "Reusable oak".into();
    assert!(
        editor
            .apply(&mut s, &Default::default(), Channel::Ui)
            .is_err()
    );
    preview(&mut editor, &s).unwrap();
    assert_eq!(s.document(), &before);
    assert_eq!(s.revision(), 0);
    editor
        .apply(&mut s, &Default::default(), Channel::Ui)
        .unwrap();
    assert_eq!(s.revision(), 1);
    assert_eq!(s.document().get(id).unwrap().name, "Reusable oak");
    assert!(!editor.dirty());
    assert_eq!(s.activity().last().unwrap().actor.channel, Channel::Ui);
    s.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &before);
    editor.open(&s);
    assert!(
        editor.draft.is_none(),
        "Undo must not retain an authorable stale definition"
    );
    s.redo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert!(s.document().get(id).is_some());
}
#[test]
fn new_stock_in_empty_project_creates_companion_material_in_one_native_checked_edit() {
    let mut s = Session::new(Document::new("Empty stock").unwrap()).unwrap();
    let before = s.document().clone();
    let mut editor = editor(&s);
    editor.create_stock(&s, 1);
    let d = editor.draft.as_ref().unwrap();
    let stock = d.entity.id;
    let material = d.companion.as_ref().unwrap().id;
    assert_ne!(stock, material);
    assert_eq!(stock_material(&d.entity), Some(material));
    apply(&mut editor, &mut s).unwrap();
    assert_eq!(s.revision(), 1);
    assert_eq!(s.document().entities().count(), 2);
    assert!(diagnostics(s.document()).unwrap().is_empty());
    s.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &before);
}
#[test]
fn material_first_sequence_creates_panel_sheet_and_profile_in_the_same_catalog() {
    let mut s = Session::new(Document::new("Shared catalog").unwrap()).unwrap();
    let mut editor = editor(&s);
    editor.create(&s, 0);
    let material = editor.draft.as_ref().unwrap().entity.id;
    apply(&mut editor, &mut s).unwrap();
    for kind in 0..3 {
        editor.create_stock(&s, kind);
        assert!(editor.draft.as_ref().unwrap().companion.is_none());
        let id = editor.draft.as_ref().unwrap().entity.id;
        apply(&mut editor, &mut s).unwrap();
        let Some(LibraryDefinition::Stock(stock)) = read(s.document().get(id).unwrap()).unwrap()
        else {
            panic!("committed stock")
        };
        assert_eq!(stock.material, material);
        assert_eq!(stock_index(&stock.specification), kind);
    }
    assert_eq!(s.revision(), 4);
    assert_eq!(s.document().entities().count(), 4);
    assert!(diagnostics(s.document()).unwrap().is_empty());
}
#[test]
fn invalid_missing_and_stale_definitions_refuse_without_source_edits() {
    let mut s = Session::new(Document::new("Invalid definitions").unwrap()).unwrap();
    let mut editor = editor(&s);
    editor.create(&s, 0);
    if let EntityData::Material { density_kg_m3, .. } =
        &mut editor.draft.as_mut().unwrap().entity.data
    {
        *density_kg_m3 = Some(-10.);
    }
    let before = s.document().clone();
    assert!(preview(&mut editor, &s).is_err());
    assert_eq!(s.document(), &before);
    editor.create_stock(&s, 0);
    let stock = editor.draft.as_ref().unwrap().entity.id;
    editor.draft.as_mut().unwrap().entity.name = "Birch ply".into();
    editor.draft.as_mut().unwrap().companion = None;
    set_stock(&mut editor, |s| s.material = stock);
    let problem = preview(&mut editor, &s).unwrap_err();
    assert!(
        problem.contains("Birch ply") && problem.contains("manufacturing material"),
        "{problem}"
    );
    assert_eq!(s.document(), &before);
    editor.create(&s, 0);
    s.apply(multicad_core::commands::Request {
        expected_revision: 0,
        actor: Actor::local(Channel::Ui),
        label: "Rename".into(),
        edits: vec![multicad_core::document::Mutation::RenameDocument(
            "New source".into(),
        )],
    })
    .unwrap();
    assert!(
        preview(&mut editor, &s)
            .unwrap_err()
            .to_lowercase()
            .contains("stale")
    );
    assert!(s.document().entities().next().is_none());
    editor.discard(&s);
    assert!(!editor.dirty());
}
#[test]
fn checked_preview_cannot_authorize_changed_draft_resources_session_or_channel() {
    let mut s = Session::new(Document::new("Preview guards").unwrap()).unwrap();
    let mut editor = editor(&s);
    editor.create(&s, 0);
    preview(&mut editor, &s).unwrap();
    let original = s.document().clone();
    editor.draft.as_mut().unwrap().entity.name = "Changed since preview".into();
    assert!(!editor.can_apply(&s, &Default::default()));
    assert!(editor.preview_scene(&s, &Default::default()).is_none());
    editor.poll(&s, &Default::default());
    assert!(!editor.pending());
    preview(&mut editor, &s).unwrap();
    let mut resources = ProjectResources::default();
    resources
        .metadata
        .insert("review".into(), serde_json::json!("changed"));
    assert!(!editor.can_apply(&s, &resources));
    let foreign = Session::new(s.document().clone()).unwrap();
    assert!(!editor.can_apply(&foreign, &Default::default()));
    assert!(
        editor
            .apply(&mut s, &Default::default(), Channel::Mcp)
            .is_err()
    );
    assert_eq!(s.document(), &original);
    editor.discard(&s);
    assert_eq!(s.document(), &original);
}
#[test]
fn shared_panel_thickness_preview_updates_real_geometry_and_undo_refreshes_clean_selection() {
    let (mut s, part) = crate::modeling::panel_fixture::flat_panel();
    let stock = s
        .document()
        .entities()
        .find(|e| matches!(read(e), Ok(Some(LibraryDefinition::Stock(_)))))
        .unwrap()
        .clone();
    let stock_id = stock.id;
    let before = s.document().clone();
    let mut editor = editor(&s);
    editor.select(&s, stock);
    set_stock(&mut editor, |s| {
        if let StockKind::Panel { thickness_mm, .. } = &mut s.specification {
            *thickness_mm = 24.;
        }
    });
    preview(&mut editor, &s).unwrap();
    let bounds = editor
        .preview_scene(&s, &Default::default())
        .unwrap()
        .native_instance_bounds(part)
        .unwrap();
    assert!((bounds[1][2] - bounds[0][2] - 24.).abs() < 1e-6);
    assert_eq!(s.document(), &before);
    editor
        .apply(&mut s, &Default::default(), Channel::Ui)
        .unwrap();
    assert!(s.document().get(part).is_some());
    assert!(s.document().get(stock_id).is_some());
    s.undo(s.revision(), Actor::local(Channel::Ui)).unwrap();
    assert_eq!(s.document(), &before);
    editor.open(&s);
    assert_eq!(
        &editor.draft.as_ref().unwrap().entity,
        s.document().get(stock_id).unwrap()
    );
    assert!(!editor.dirty());
}
#[test]
fn a_shared_stock_edit_that_breaks_dependent_geometry_cannot_apply() {
    let (s, _, _) = crate::modeling::panel_fixture::carcass();
    let stock = s
        .document()
        .entities()
        .find(|e| matches!(read(e), Ok(Some(LibraryDefinition::Stock(_)))))
        .unwrap()
        .clone();
    let before = s.document().clone();
    let mut editor = editor(&s);
    editor.select(&s, stock);
    set_stock(&mut editor, |s| {
        if let StockKind::Panel { thickness_mm, .. } = &mut s.specification {
            *thickness_mm = 400.;
        }
    });
    let error = preview(&mut editor, &s).unwrap_err();
    assert!(!error.is_empty());
    assert!(!editor.can_apply(&s, &Default::default()));
    assert_eq!(s.document(), &before);
}
#[test]
fn a_display_scene_without_native_source_metadata_cannot_authorize_apply() {
    use crate::viewport::{Scene, SceneBackend, SceneReply, SceneStamp};
    struct NoReceipt(Option<SceneReply>);
    impl SceneBackend for NoReceipt {
        fn request(&mut self, _: &Document, stamp: SceneStamp) -> Result<(), String> {
            self.0 = Some(SceneReply {
                stamp,
                result: Ok(std::sync::Arc::new(Scene::new(stamp, vec![], vec![])?)),
            });
            Ok(())
        }
        fn poll(&mut self) -> Option<SceneReply> {
            self.0.take()
        }
    }
    let s = Session::new(Document::new("No receipt").unwrap()).unwrap();
    let mut editor = editor(&s);
    editor.set_backend(Box::new(NoReceipt(None)));
    editor.create(&s, 0);
    let error = preview(&mut editor, &s).unwrap_err();
    assert!(error.contains("source-bound"), "{error}");
    assert!(!editor.can_apply(&s, &Default::default()));
    assert_eq!(s.revision(), 0);
}
