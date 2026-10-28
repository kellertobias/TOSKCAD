//! Repeated real-host layout includes tool cancellation and the normal Instance
//! Inspector, rather than testing one form inside an unconstrained root Ui.
use super::*;
use crate::modeling::native_fixture::NativeBackend;
use multicad_core::{
    document::Document,
    model::{
        self, BodyInput, InstanceCopyRequest, InstanceCopySource, PrimitiveParameters,
        PrimitiveRequest, length,
    },
};
use std::time::{Duration, Instant};
fn fixture() -> (Session, SketchEditor, crate::viewport::SceneFeed) {
    let mut session =
        Session::new(Document::new("Resizable native placement inspector").unwrap()).unwrap();
    let source = PrimitiveRequest::new(
        "Source block with a descriptive component name",
        PrimitiveParameters::Box {
            size: [length(60.), length(40.), length(12.)],
            frame: Default::default(),
        },
    );
    let prepared = model::prepare_primitive(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        &source,
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let copy = InstanceCopyRequest::new(
        InstanceCopySource::Part {
            source: BodyInput {
                body: source.body,
                producer: source.feature,
            },
        },
        Default::default(),
    );
    let prepared = model::prepare_instance_copies(
        &session,
        session.revision(),
        Actor::local(Channel::Ui),
        std::slice::from_ref(&copy),
    )
    .unwrap();
    session.commit(prepared.prepared).unwrap();
    let mut editor = SketchEditor::default();
    editor.set_model_backend(Box::new(NativeBackend::new()));
    editor.tree_selection.insert(copy.instance);
    let mut feed = crate::viewport::SceneFeed::default();
    feed.set_backend(Box::new(NativeBackend::new()));
    let until = Instant::now() + Duration::from_secs(3);
    while !feed.is_current(session.document().id(), session.revision()) && Instant::now() < until {
        feed.tick(session.document(), session.revision());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        feed.is_current(session.document().id(), session.revision()),
        "{:?}",
        feed.error()
    );
    (session, editor, feed)
}
fn frame(
    ctx: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut crate::viewport::SceneFeed,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 1000.))),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| editor.show_project(ui, session, false, feed, &Default::default()),
    )
}
fn click(
    ctx: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    feed: &mut crate::viewport::SceneFeed,
    label: &str,
) {
    let out = frame(ctx, editor, session, feed, vec![]);
    let position = out
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing {label}"));
    out.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            ctx,
            editor,
            session,
            feed,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
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
fn real_host_inspector_stays_user_sized_through_alignment_picking_cancel_and_instance_properties() {
    for scale in [1., 2.] {
        let (mut session, mut editor, mut feed) = fixture();
        let original = session.document().clone();
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        editor
            .begin_alignment(&session, &Default::default(), &feed, Channel::Ui)
            .unwrap();
        for iteration in 0..180 {
            if iteration == 40 {
                click(&ctx, &mut editor, &mut session, &mut feed, "Pick geometry");
                assert!(editor.model.alignment_picking());
            }
            if iteration == 80 {
                click(&ctx, &mut editor, &mut session, &mut feed, "Cancel");
                assert!(!editor.model.active());
            }
            frame(&ctx, &mut editor, &mut session, &mut feed, vec![])
                .drop_without_applying_deltas();
            let inspector =
                egui::containers::panel::PanelState::load(&ctx, egui::Id::new("source-inspector"))
                    .expect("host inspector");
            assert!(
                inspector.size().x <= 251.,
                "Scale {scale}, frame {iteration}: Inspector grew to {}",
                inspector.size().x
            );
            assert!(
                editor.viewport_rect.unwrap().width() >= 850.,
                "Viewport was starved at frame {iteration}"
            );
        }
        assert_eq!(session.document(), &original);
        assert!(!editor.model.active());
    }
}
#[test]
fn runaway_persisted_inspector_is_bounded_after_cancel_and_across_opened_documents() {
    let ctx = egui::Context::default();
    ctx.data_mut(|data| {
        data.insert_persisted(
            egui::Id::new("source-inspector"),
            egui::containers::panel::PanelState {
                outer_rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(1134., 1000.)),
            },
        )
    });
    for _ in 0..2 {
        let (mut session, mut editor, mut feed) = fixture();
        let original = session.document().clone();
        for iteration in 0..100 {
            frame(&ctx, &mut editor, &mut session, &mut feed, vec![])
                .drop_without_applying_deltas();
            let inspector =
                egui::containers::panel::PanelState::load(&ctx, egui::Id::new("source-inspector"))
                    .unwrap();
            assert!(
                inspector.size().x <= 568.,
                "Frame {iteration}: persisted Inspector consumed {}",
                inspector.size().x
            );
            assert!(editor.viewport_rect.unwrap().width() >= 550.);
        }
        assert_eq!(session.document(), &original);
    }
}
