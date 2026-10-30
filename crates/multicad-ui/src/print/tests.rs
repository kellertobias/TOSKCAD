use super::*;
use multicad_core::{
    commands::Request,
    document::{Document, Entity, EntityData, Mutation},
    project_file::ContentHash,
};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
struct State {
    previews: Vec<(EntityId, u64, PrintRequest, Actor)>,
    writes: Vec<(EntityId, u64, PrintKey, PathBuf, Actor)>,
    replies: VecDeque<PrintReply>,
    cancelled: Vec<EntityId>,
}
struct Backend(Rc<RefCell<State>>);
impl PrintBackend for Backend {
    fn preview(
        &mut self,
        _: &Document,
        revision: u64,
        request: &PrintRequest,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        self.0
            .borrow_mut()
            .previews
            .push((id, revision, request.clone(), actor));
        Ok(id)
    }
    fn write(
        &mut self,
        _: &Document,
        revision: u64,
        _: &PrintRequest,
        expected: &PrintKey,
        path: &Path,
        actor: Actor,
    ) -> Result<EntityId, ExportError> {
        let id = EntityId::new();
        self.0
            .borrow_mut()
            .writes
            .push((id, revision, expected.clone(), path.to_owned(), actor));
        Ok(id)
    }
    fn cancel(&mut self, id: EntityId) -> bool {
        self.0.borrow_mut().cancelled.push(id);
        true
    }
    fn poll(&mut self) -> Option<PrintReply> {
        self.0.borrow_mut().replies.pop_front()
    }
    fn pending(&self) -> bool {
        false
    }
}
fn fixture() -> (Session, PrintWorkspace, Rc<RefCell<State>>, EntityId) {
    let mut s = Session::new(Document::new("Bracket").unwrap()).unwrap();
    let part = Entity::new(
        "Bracket body",
        EntityData::Part {
            features: vec![],
            material: None,
            tags: Default::default(),
        },
    );
    let id = part.id;
    s.apply(Request {
        expected_revision: 0,
        actor: Actor::local(Channel::Ui),
        label: "Part".into(),
        edits: vec![Mutation::Insert(part)],
    })
    .unwrap();
    let state = Rc::new(RefCell::new(State::default()));
    let mut w = PrintWorkspace::default();
    w.set_backend(Box::new(Backend(state.clone()))).unwrap();
    w.capture(vec![id]);
    (s, w, state, id)
}
fn reply_preview(s: &Session, state: &Rc<RefCell<State>>, index: usize, objects: usize) {
    let (id, revision, request, actor) = state.borrow().previews[index].clone();
    let stamp = ExportStamp::capture(s.document(), revision).unwrap();
    let preview = PrintPreview {
        requested_by: actor.clone(),
        key: PrintKey {
            source: stamp.clone(),
            plan_hash: ContentHash::of(b"plan"),
        },
        provider: "test".into(),
        request,
        selection: PrintSelection {
            stamp,
            parts: vec![],
            skipped: vec![],
            processes: vec![],
        },
        // Every object after the first is a placed copy of the bracket.
        objects: (0..objects)
            .map(|n| PrintObject {
                part: EntityId::new(),
                occurrence_path: if n == 0 {
                    vec![]
                } else {
                    vec![EntityId::new()]
                },
                name: if n == 0 {
                    "Bracket body".into()
                } else {
                    format!("Bracket body (Copy {n})")
                },
                role: PrintRole::Finished,
                bounds_mm: [[0.; 3], [1.; 3]],
                volume_mm3: 1.,
                triangles: 12,
                mesh_volume_mm3: 1.,
            })
            .collect(),
        files: vec![],
        processes: vec![],
        findings: vec![],
        blocked: (objects == 0).then(|| "No printable Parts".into()),
    };
    state.borrow_mut().replies.push_back(PrintReply {
        id,
        document: s.document().id(),
        revision,
        actor,
        result: PrintResult::Preview(Ok(Box::new(preview))),
    });
}
fn rename(s: &mut Session) {
    s.apply(Request {
        expected_revision: s.revision(),
        actor: Actor::local(Channel::Mcp),
        label: "Concurrent edit".into(),
        edits: vec![Mutation::RenameDocument("Bracket v2".into())],
    })
    .unwrap();
}

#[test]
fn preview_uses_the_captured_selection_and_settings_and_writing_needs_a_current_preview() {
    let (s, mut w, state, part) = fixture();
    assert!(w.enabled(PrintTool::Preview, &s));
    assert!(!w.enabled(PrintTool::Write, &s));
    assert!(w.begin_write(&s).is_err());
    w.set_aids(AidPolicy::ExcludeRemovable);
    w.set_stl_per_part(true);
    w.request_preview(&s, Channel::Ui).unwrap();
    let request = state.borrow().previews[0].2.clone();
    assert_eq!(request.source, PrintSource::Parts { parts: vec![part] });
    assert_eq!(request.aids, AidPolicy::ExcludeRemovable);
    assert!(request.stl_per_part);
    assert!(w.pending() && !w.enabled(PrintTool::Write, &s));
    reply_preview(&s, &state, 0, 1);
    w.poll(&s);
    assert!(!w.pending() && w.current(&s), "{:?}", w.error());
    assert!(w.enabled(PrintTool::Write, &s));
    assert_eq!(w.begin_write(&s).unwrap(), "Bracket.3mf");
    w.write(&s, Path::new("/private/tmp/bracket"), Channel::Ui)
        .unwrap();
    let written = state.borrow().writes[0].clone();
    assert_eq!(written.3, PathBuf::from("/private/tmp/bracket.3mf"));
    assert_eq!(&written.2, &w.preview().unwrap().key);
    assert!(w.writing());
    // A wrong extension is refused before the service sees it.
    let (s, mut w, state, _) = fixture();
    w.request_preview(&s, Channel::Ui).unwrap();
    reply_preview(&s, &state, 0, 1);
    w.poll(&s);
    assert!(
        w.write(&s, Path::new("/private/tmp/plate.stl"), Channel::Ui)
            .is_err()
    );
    assert!(state.borrow().writes.is_empty());
}

#[test]
fn changed_source_or_settings_make_the_preview_display_only_and_block_writing() {
    let (mut s, mut w, state, _) = fixture();
    w.request_preview(&s, Channel::Ui).unwrap();
    reply_preview(&s, &state, 0, 1);
    w.poll(&s);
    assert!(w.current(&s));
    w.set_stl_per_part(true);
    assert!(!w.current(&s) && !w.enabled(PrintTool::Write, &s));
    w.set_stl_per_part(false);
    assert!(w.current(&s));
    rename(&mut s);
    assert!(!w.current(&s));
    assert!(w.begin_write(&s).unwrap_err().contains("Preview"));
    assert!(
        w.write(&s, Path::new("/private/tmp/stale.3mf"), Channel::Ui)
            .is_err()
    );
    assert!(state.borrow().writes.is_empty());
    // A late reply for the old revision does not replace anything.
    let (mut s, mut w, state, _) = fixture();
    w.request_preview(&s, Channel::Ui).unwrap();
    rename(&mut s);
    reply_preview(&s, &state, 0, 1);
    w.poll(&s);
    assert!(w.preview().is_none());
}

#[test]
fn blocked_previews_tag_source_and_receipts_are_attributed() {
    let (s, mut w, state, _) = fixture();
    w.set_source_tag(Some("printed"));
    w.request_preview(&s, Channel::Ui).unwrap();
    assert_eq!(
        state.borrow().previews[0].2.source,
        PrintSource::Tag {
            tag: "printed".into()
        }
    );
    reply_preview(&s, &state, 0, 0);
    w.poll(&s);
    assert!(w.current(&s) && !w.enabled(PrintTool::Write, &s));
    assert_eq!(w.begin_write(&s).unwrap_err(), "No printable Parts");
    // A receipt that does not match the accepted job is reported.
    let (s, mut w, state, _) = fixture();
    w.request_preview(&s, Channel::Ui).unwrap();
    reply_preview(&s, &state, 0, 1);
    w.poll(&s);
    w.write(&s, Path::new("/private/tmp/a.3mf"), Channel::Ui)
        .unwrap();
    let (id, revision, key, _, actor) = state.borrow().writes[0].clone();
    let mut foreign = key.clone();
    foreign.plan_hash = ContentHash::of(b"other");
    state.borrow_mut().replies.push_back(PrintReply {
        id,
        document: s.document().id(),
        revision,
        actor: actor.clone(),
        result: PrintResult::Write(Ok(PrintReceipt {
            actor: actor.clone(),
            key: foreign,
            files: vec![],
        })),
    });
    w.poll(&s);
    assert!(w.error().unwrap().contains("differs"));
    assert!(!w.writing());
    w.write(&s, Path::new("/private/tmp/b.3mf"), Channel::Ui)
        .unwrap();
    let (id, revision, key, _, actor) = state.borrow().writes[1].clone();
    state.borrow_mut().replies.push_back(PrintReply {
        id,
        document: s.document().id(),
        revision,
        actor: actor.clone(),
        result: PrintResult::Write(Err(ExportError::new(
            ErrorKind::Stale,
            None,
            "Print package plan changed",
        ))),
    });
    let _ = key;
    w.poll(&s);
    let (notice, error) = w.take_notice().unwrap();
    assert!(error && notice.contains("not written"));
}

#[test]
fn print_workspace_renders_its_controls_and_requests_the_save_dialog() {
    let (s, mut w, state, _) = fixture();
    w.request_preview(&s, Channel::Ui).unwrap();
    reply_preview(&s, &state, 0, 2);
    w.poll(&s);
    let ctx = egui::Context::default();
    let mut labels = vec![];
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400., 900.),
            )),
            ..Default::default()
        },
        |ui| w.show(ui, &s, true),
    );
    output.textures_delta.clear();
    for shape in &output.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            labels.push(text.galley.text().to_owned());
        }
    }
    for expected in [
        "Printable Parts",
        "Include removable FDM aids",
        "Preview package",
        "Write 3MF package",
        "FDM process records",
        "Findings",
        "Bracket body (Copy 1)",
    ] {
        assert!(
            labels.iter().any(|l| l == expected),
            "{expected} missing: {labels:?}"
        );
    }
    assert!(
        labels
            .iter()
            .any(|l| l.starts_with("2 object(s), 1 of them placed occurrence(s)")),
        "{labels:?}"
    );
    assert!(!w.take_write_request());
}
