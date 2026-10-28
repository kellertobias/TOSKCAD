use super::*;
use multicad_core::{
    commands::Request,
    document::{Document, Entity, EntityData, Mutation},
    project_file::ContentHash,
};
use std::{cell::RefCell, rc::Rc};
#[derive(Default)]
struct State {
    requests: Vec<(EntityId, Document, u64, EntityId, Actor, bool)>,
    replies: VecDeque<ExportReply>,
    cancelled: Vec<EntityId>,
    catalogs: Vec<(EntityId, Document, u64, Scope, Actor)>,
}
struct Backend(Rc<RefCell<State>>);
impl ExportBackend for Backend {
    fn planar_catalog(
        &mut self,
        d: &Document,
        r: u64,
        scope: &Scope,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        self.0
            .borrow_mut()
            .catalogs
            .push((id, d.clone(), r, scope.clone(), a));
        Ok(id)
    }
    fn preview(
        &mut self,
        d: &Document,
        r: u64,
        s: EntityId,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        self.0
            .borrow_mut()
            .requests
            .push((id, d.clone(), r, s, a, false));
        Ok(id)
    }
    fn run(
        &mut self,
        d: &Document,
        r: u64,
        k: &PreviewKey,
        _: &Path,
        a: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        self.0
            .borrow_mut()
            .requests
            .push((id, d.clone(), r, k.set, a, true));
        Ok(id)
    }
    fn cancel(&mut self, id: EntityId) -> bool {
        self.0.borrow_mut().cancelled.push(id);
        true
    }
    fn poll(&mut self) -> Option<ExportReply> {
        self.0.borrow_mut().replies.pop_front()
    }
    fn pending(&self) -> bool {
        false
    }
}
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
#[test]
fn face_catalog_is_source_scoped_read_only_and_rejects_stale_foreign_or_wrong_scope_results() {
    let (s, mut w, state, _) = fixture();
    let original = s.document().clone();
    for mode in 0..4 {
        w.planar_catalog = None;
        w.request_planar_catalog(&s, Channel::Mcp).unwrap();
        let (id, document, revision, scope, actor) =
            state.borrow().catalogs.last().unwrap().clone();
        let mut catalog = PlanarCatalog {
            stamp: ExportStamp::capture(&document, revision).unwrap(),
            scope,
            parts: vec![],
        };
        let mut attributed = actor;
        match mode {
            1 => catalog.stamp.source_hash = ContentHash::of(b"foreign source"),
            2 => {
                catalog.scope.root = ScopeRoot::Explicit {
                    entities: vec![EntityId::new()],
                }
            }
            3 => attributed.id = "foreign actor".into(),
            _ => {}
        }
        state.borrow_mut().replies.push_back(ExportReply {
            id,
            document: document.id(),
            revision,
            actor: attributed,
            result: ExportResult::PlanarCatalog(Ok(Box::new(catalog))),
        });
        w.poll(&s);
        assert_eq!(w.planar_catalog.is_some(), mode == 0);
        assert!(!w.dirty());
        assert_eq!(s.document(), &original);
        assert!(!w.pending());
    }
}
fn fixture() -> (Session, ExportWorkspace, Rc<RefCell<State>>, EntityId) {
    let mut d = Document::new("Shop project").unwrap();
    let part = Entity::new(
        "Panel",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let id = part.id;
    d.insert(part).unwrap();
    let part = id;
    let mut s = Session::new(d).unwrap();
    let set = EntityId::new();
    s.commit(prepare_set(&s, 0, actor(), set, "Shop", &Default::default()).unwrap())
        .unwrap();
    let state = Rc::new(RefCell::new(State::default()));
    let mut w = ExportWorkspace::default();
    w.set_backend(Box::new(Backend(state.clone()))).unwrap();
    w.load(&s, set).unwrap();
    w.directory = "/private/tmp".into();
    (s, w, state, part)
}
fn preview(state: &Rc<RefCell<State>>, index: usize) -> ExportReply {
    let b = state.borrow();
    let (id, d, r, set, a, _) = &b.requests[index];
    let selection = resolve(d, *r, *set).unwrap();
    let files = filenames(&selection, d.name()).unwrap();
    let recipe = selection.recipes[0].id;
    let value = ExportPreview {
        requested_by: a.clone(),
        key: PreviewKey {
            source: selection.stamp.clone(),
            set: *set,
            plan_hash: ContentHash::document(d).unwrap(),
        },
        provider: "controlled UI service".into(),
        selection,
        recipes: vec![RecipeReport {
            recipe,
            status: RecipeStatus::Ready,
            files: 1,
        }],
        artifacts: vec![ExportArtifact {
            planar: None,
            output: files[0].clone(),
            producer: EntityId::new(),
            solid: EntityId::new(),
            bounds_mm: [[0.; 3], [1.; 3]],
            volume_mm3: 1.,
            solid_count: 1,
        }],
        excluded: vec![],
    };
    ExportReply {
        id: *id,
        document: d.id(),
        revision: *r,
        actor: a.clone(),
        result: ExportResult::Preview(Ok(Box::new(value))),
    }
}
fn ready(w: &mut ExportWorkspace, s: &mut Session, state: &Rc<RefCell<State>>) {
    w.execute(s, ExportTool::Preview, None, Channel::Mcp)
        .unwrap();
    let i = state.borrow().requests.len() - 1;
    let reply = preview(state, i);
    state.borrow_mut().replies.push_back(reply);
    w.poll(s);
    assert!(w.enabled(ExportTool::Run, s));
}
#[test]
fn save_draft_is_atomic_and_stale_draft_never_overwrites_newer_source() {
    let (mut s, mut w, _, _) = fixture();
    let before = s.document().clone();
    w.draft.as_mut().unwrap().name = "New name".into();
    assert!(w.dirty());
    assert_eq!(s.document(), &before);
    w.save(&mut s, Channel::Mcp).unwrap();
    assert!(!w.dirty());
    let set = w.selected.unwrap();
    assert_eq!(s.document().get(set).unwrap().name, "New name");
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &before);
    w.load(&s, set).unwrap();
    w.draft.as_mut().unwrap().name = "Unapplied".into();
    s.redo(s.revision(), actor()).unwrap();
    let newer = s.document().clone();
    assert!(w.save(&mut s, Channel::Ui).is_err());
    assert_eq!(s.document(), &newer);
    assert!(w.dirty());
    w.discard(&s);
    assert!(!w.dirty());
    assert_eq!(w.draft.unwrap().name, "New name");
}
#[test]
fn current_preview_run_cancel_and_late_receipt_preserve_accepted_snapshot_attribution() {
    let (mut s, mut w, state, part) = fixture();
    let before = s.document().clone();
    ready(&mut w, &mut s, &state);
    assert_eq!(s.document(), &before);
    assert!(!w.writing());
    let key = w.preview.as_ref().unwrap().key.clone();
    w.execute(&mut s, ExportTool::Run, None, Channel::Mcp)
        .unwrap();
    assert!(w.writing());
    assert!(w.set_backend(Box::new(Backend(state.clone()))).is_err());
    w.execute(&mut s, ExportTool::Cancel, None, Channel::Ui)
        .unwrap();
    assert!(
        w.writing(),
        "Cancellation is not publication acknowledgement"
    );
    let mut changed = s.document().get(part).unwrap().clone();
    changed.name = "Renamed later".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Rename".into(),
        edits: vec![Mutation::Replace(changed)],
    })
    .unwrap();
    assert!(!w.enabled(ExportTool::Run, &s));
    let ctx = egui::Context::default();
    frame(&mut w, &mut s, &ctx, vec![]).drop_without_applying_deltas();
    assert_eq!(w.preview.as_ref().unwrap().key, key);
    assert!(!w.enabled(ExportTool::Run, &s));
    let (id, doc, revision, _, a, _) = state.borrow().requests[1].clone();
    state.borrow_mut().replies.push_back(ExportReply {
        id,
        document: doc.id(),
        revision,
        actor: a.clone(),
        result: ExportResult::Run(Ok(RunReceipt {
            actor: a,
            key,
            directory: "/private/tmp/accepted-bundle".into(),
            files: vec![],
            reused: false,
        })),
    });
    w.poll(&s);
    assert!(!w.writing());
    assert!(w.history[0].text.contains("Mcp"));
    assert!(w.history[0].text.contains(&format!("revision {revision}")));
    assert_eq!(
        w.history[0].directory.as_deref(),
        Some("/private/tmp/accepted-bundle")
    );
    assert_eq!(s.revision(), revision + 1);
}
#[test]
fn stale_foreign_modified_source_and_wrong_receipts_cannot_enable_run() {
    let (mut s, mut w, state, part) = fixture();
    w.execute(&mut s, ExportTool::Preview, None, Channel::Ui)
        .unwrap();
    let mut foreign = preview(&state, 0);
    foreign.actor = actor();
    state.borrow_mut().replies.push_back(foreign);
    w.poll(&s);
    assert!(w.preview.is_none());
    assert!(w.error.as_ref().unwrap().contains("scope"));
    w.execute(&mut s, ExportTool::Preview, None, Channel::Ui)
        .unwrap();
    let delayed = preview(&state, 1);
    let mut changed = s.document().get(part).unwrap().clone();
    changed.name = "Now changed".into();
    s.apply(Request {
        expected_revision: s.revision(),
        actor: actor(),
        label: "Rename".into(),
        edits: vec![Mutation::Replace(changed)],
    })
    .unwrap();
    state.borrow_mut().replies.push_back(delayed);
    w.poll(&s);
    assert!(w.preview.is_none());
    w.load(&s, w.selected.unwrap()).unwrap();
    ready(&mut w, &mut s, &state);
    let clone = s.document().clone();
    let mut changed = clone.get(part).unwrap().clone();
    changed.name = "Same revision but different bytes".into();
    let mut clone = Session::new(clone).unwrap();
    clone
        .apply(Request {
            expected_revision: 0,
            actor: actor(),
            label: "Different canonical snapshot".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    let mut changed = clone.document().get(part).unwrap().clone();
    changed.name = "Same revision, changed again".into();
    clone
        .apply(Request {
            expected_revision: clone.revision(),
            actor: actor(),
            label: "Match revision without matching source".into(),
            edits: vec![Mutation::Replace(changed)],
        })
        .unwrap();
    assert_eq!(clone.revision(), s.revision());
    assert!(
        w.execute(&mut clone, ExportTool::Run, None, Channel::Ui)
            .is_err()
    );
    w.execute(&mut s, ExportTool::Run, None, Channel::Ui)
        .unwrap();
    let index = state.borrow().requests.len() - 1;
    let (id, d, r, set, a, _) = state.borrow().requests[index].clone();
    let mut key = w.preview.as_ref().unwrap().key.clone();
    key.set = EntityId::new();
    state.borrow_mut().replies.push_back(ExportReply {
        id,
        document: d.id(),
        revision: r,
        actor: a.clone(),
        result: ExportResult::Run(Ok(RunReceipt {
            actor: a,
            key,
            directory: "/untrusted".into(),
            files: vec![],
            reused: false,
        })),
    });
    w.poll(&s);
    assert!(w.history.iter().all(|job| job.directory.is_none()));
    assert!(w.error.unwrap().contains("receipt"));
    assert_eq!(w.selected, Some(set));
}
fn frame(
    w: &mut ExportWorkspace,
    s: &mut Session,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400., 900.),
            )),
            events,
            ..Default::default()
        },
        |ui| w.show(ui, s, true),
    )
}
fn click(w: &mut ExportWorkspace, s: &mut Session, ctx: &egui::Context, label: &str) {
    frame(w, s, ctx, vec![]).drop_without_applying_deltas();
    let out = frame(w, s, ctx, vec![]);
    let p = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            w,
            s,
            ctx,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
#[test]
fn native_set_editor_clicks_create_save_reopen_and_lists_exact_unsupported_recipe() {
    let (mut s, mut w, state, _) = fixture();
    let ctx = egui::Context::default();
    let before = s.document().clone();
    click(&mut w, &mut s, &ctx, "New export set");
    assert!(w.dirty());
    assert_eq!(s.document(), &before);
    click(&mut w, &mut s, &ctx, "Save set");
    assert!(!w.dirty());
    let new = w.selected.unwrap();
    assert_eq!(sets(s.document()).count(), 2);
    assert_ne!(s.document(), &before);
    w.request_preview(&s, Channel::Ui).unwrap();
    let i = state.borrow().requests.len() - 1;
    let mut reply = preview(&state, i);
    if let ExportResult::Preview(Ok(value)) = &mut reply.result {
        value.recipes[0].status = RecipeStatus::Unsupported("PDF provider not available".into());
        value.artifacts.clear();
    }
    state.borrow_mut().replies.push_back(reply);
    w.poll(&s);
    assert!(!w.enabled(ExportTool::Run, &s));
    let out = frame(&mut w, &mut s, &ctx, vec![]);
    assert!(out.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(t)if t.galley.text().contains("PDF provider not available"))));
    out.drop_without_applying_deltas();
    let mut reopened = ExportWorkspace::default();
    reopened.load(&s, new).unwrap();
    assert!(reopened.draft == w.draft);
}
