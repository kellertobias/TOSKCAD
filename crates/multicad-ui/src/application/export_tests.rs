//! Window lifecycle checks use a controlled output adapter; native STEP geometry
//! and filesystem publication have separate multicad-model integration tests.
use super::*;
use crate::exchange::{ExportBackend, ExportTool};
use multicad_core::{exchange::*, project_file::ContentHash};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct State {
    replies: VecDeque<ExportReply>,
    run: Option<(EntityId, Document, u64, PreviewKey, Actor)>,
}
struct Backend(Arc<Mutex<State>>);
impl ExportBackend for Backend {
    fn preview(
        &mut self,
        d: &Document,
        r: u64,
        set: EntityId,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        let selection = resolve(d, r, set)?;
        let file = filenames(&selection, d.name())?.remove(0);
        let recipe = selection.recipes[0].id;
        let p = ExportPreview {
            requested_by: a.clone(),
            key: PreviewKey {
                source: selection.stamp.clone(),
                set,
                plan_hash: ContentHash::document(d).unwrap(),
            },
            provider: "Controlled lifecycle adapter".into(),
            selection,
            recipes: vec![RecipeReport {
                recipe,
                status: RecipeStatus::Ready,
                files: 1,
                excluded_aids: vec![],
            }],
            artifacts: vec![ExportArtifact {
                planar: None,
                mesh: None,
                drawing: None,
                schedule: None,
                output: file,
                producer: EntityId::new(),
                solid: EntityId::new(),
                volume_mm3: 1.,
                solid_count: 1,
                bounds_mm: [[0.; 3], [1.; 3]],
            }],
            excluded: vec![],
        };
        self.0.lock().unwrap().replies.push_back(ExportReply {
            id,
            document: d.id(),
            revision: r,
            actor: a,
            result: ExportResult::Preview(Ok(Box::new(p))),
        });
        Ok(id)
    }
    fn run(
        &mut self,
        d: &Document,
        r: u64,
        key: &PreviewKey,
        _: &Path,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        self.0.lock().unwrap().run = Some((id, d.clone(), r, key.clone(), a));
        Ok(id)
    }
    fn cancel(&mut self, _: EntityId) -> bool {
        true
    }
    fn poll(&mut self) -> Option<ExportReply> {
        self.0.lock().unwrap().replies.pop_front()
    }
    fn pending(&self) -> bool {
        false
    }
}
struct Store;
impl DocumentStorage for Store {
    fn save(&mut self, _: &Path, _: &Document) -> Result<(), String> {
        Ok(())
    }
    fn load(&mut self, _: &Path) -> Result<Document, String> {
        Err("unused".into())
    }
}
fn fixture() -> (Application, Arc<Mutex<State>>) {
    let mut app = Application::new(Box::new(Store)).unwrap();
    app.pending_files.clear();
    let p = app.current_mut();
    let mut e = Entity::new(
        "Panel",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    e.id = EntityId::new();
    p.session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Ui),
            label: "Lifecycle source".into(),
            edits: vec![Mutation::Insert(e)],
        })
        .unwrap();
    p.editor.reload(&p.session);
    let state = Arc::new(Mutex::new(State::default()));
    let shared = state.clone();
    app.set_export_backends(Arc::new(move || Box::new(Backend(shared.clone()))))
        .unwrap();
    (app, state)
}
fn frame(app: &mut Application, ctx: &egui::Context) {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400., 900.),
            )),
            ..Default::default()
        },
        |ui| app.show(ui),
    )
    .drop_without_applying_deltas();
}
fn ready(app: &mut Application) {
    app.invoke(Command::Export(ExportTool::NewSet), &[], Channel::Ui)
        .unwrap();
    app.invoke(Command::Export(ExportTool::SaveSet), &[], Channel::Ui)
        .unwrap();
    app.invoke(Command::Export(ExportTool::Preview), &[], Channel::Mcp)
        .unwrap();
    let p = app.current_mut();
    p.exchange.poll(&p.session);
}
#[test]
fn export_drafts_gate_other_editors_save_undo_and_close_routes_to_export() {
    let (mut app, _) = fixture();
    app.invoke(Command::Export(ExportTool::NewSet), &[], Channel::Ui)
        .unwrap();
    assert!(app.current().pending_edits());
    for command in [
        Command::Save,
        Command::Undo,
        Command::Parameter,
        Command::Mode(Mode::Model),
        Command::Drawing(crate::drawings::DrawingTool::AddPage),
    ] {
        assert!(!app.enabled(command));
        assert!(app.invoke(command, &[], Channel::Ui).is_err());
    }
    app.invoke(Command::Apply, &[], Channel::Mcp).unwrap();
    assert!(!app.current().pending_edits());
    assert_eq!(app.mode, Mode::Export);
    app.invoke(Command::Export(ExportTool::NewSet), &[], Channel::Ui)
        .unwrap();
    app.invoke(Command::Discard, &[], Channel::Ui).unwrap();
    assert!(!app.current().pending_edits());
}
#[test]
fn accepted_write_guards_quit_until_hidden_tab_reply_and_file_dialog_blocks_new_jobs() {
    let (mut app, state) = fixture();
    ready(&mut app);
    let rev = app.current().session.revision();
    app.current_mut().session.mark_saved(rev).unwrap();
    assert!(
        !app.should_defer_close(),
        "Read-only preview is not an unsaved source change"
    );
    app.file_action = Some(FileAction::Open);
    assert!(
        app.invoke(
            Command::Export(ExportTool::Run),
            &["/private/tmp"],
            Channel::Mcp
        )
        .is_err()
    );
    app.file_action = None;
    app.invoke(
        Command::Export(ExportTool::Run),
        &["/private/tmp"],
        Channel::Mcp,
    )
    .unwrap();
    assert!(app.should_defer_close());
    let ctx = egui::Context::default();
    assert!(app.confirm_close_without_saving(&ctx).is_err());
    app.invoke(Command::New, &[], Channel::Ui).unwrap();
    let active = app.active;
    assert_ne!(active, 0);
    let (id, d, r, key, a) = state.lock().unwrap().run.take().unwrap();
    state.lock().unwrap().replies.push_back(ExportReply {
        id,
        document: d.id(),
        revision: r,
        actor: a.clone(),
        result: ExportResult::Run(Ok(RunReceipt {
            actor: a,
            key,
            directory: "/private/tmp/verified".into(),
            files: vec![],
            reused: false,
        })),
    });
    frame(&mut app, &ctx);
    assert!(!app.projects[0].exchange.writing());
    assert_eq!(app.active, active);
    assert!(
        !app.shortcut_context(&ctx).viewport,
        "Parameters has no hidden camera pan"
    );
    app.invoke(Command::Mode(Mode::Export), &[], Channel::Ui)
        .unwrap();
    assert!(
        !app.shortcut_context(&ctx).viewport,
        "Export keyboard input cannot move a hidden model camera"
    );
}
