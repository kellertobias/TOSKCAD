use super::*;
use multicad_core::drawings::{DrawingEvaluation, Orientation, PaperSize};

#[test]
fn paper_dimensions_and_orientation_are_known_before_native_geometry() {
    let mut page = drawings::Page::new("1");
    page.settings.paper = PaperSize::A3;
    page.settings.orientation = Orientation::Landscape;
    assert_eq!(loading::paper_label(&page), "A3 · 420 × 297 mm · Landscape");
    page.settings.paper = PaperSize::Custom {
        width_mm: 123.25,
        height_mm: 210.5,
    };
    page.settings.orientation = Orientation::Portrait;
    assert_eq!(
        loading::paper_label(&page),
        "Custom · 123.25 × 210.5 mm · Portrait"
    );
}

#[test]
fn indeterminate_bar_keeps_moving_during_a_long_generation_without_claiming_a_percentage() {
    let track = egui::Rect::from_min_size(egui::pos2(20., 10.), egui::vec2(280., 5.));
    let frames: Vec<_> = [0., 0.85, 1.7, 21., 85.]
        .map(|t| loading::marquee_rect(track, t))
        .into();
    for frame in &frames {
        assert!(track.contains_rect(*frame));
        assert!((frame.width() - 70.).abs() < 0.0001);
    }
    assert_ne!(frames[0].left(), frames[1].left());
    assert_ne!(frames[3].left(), frames[4].left());
}

fn sheet_text(
    page: &drawings::Page,
    evaluation: Option<&DrawingEvaluation>,
    state: loading::PreviewState<'_>,
) -> String {
    let ctx = egui::Context::default();
    let mut text = String::new();
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 720.),
                )),
                time: Some(21.),
                ..Default::default()
            },
            |ui| {
                canvas::show(
                    ui,
                    &mut canvas::SheetView::default(),
                    page,
                    1,
                    1,
                    evaluation,
                    None,
                    None,
                    None,
                    state,
                );
            },
        );
        text.clear();
        for shape in &output.shapes {
            if let egui::epaint::Shape::Text(t) = &shape.shape {
                text.push_str(t.galley.text());
                text.push('\n');
            }
        }
        output.drop_without_applying_deltas();
    }
    text
}

#[test]
fn a_pending_sheet_shows_its_frame_title_block_and_named_slots_and_stops_loading_on_completion_or_failure()
 {
    let (mut editor, mut session, source) = tests::view_editor(false);
    editor
        .invoke(&mut session, DrawingTool::Projection, Channel::Ui)
        .unwrap();
    let page = editor.draft.as_mut().unwrap();
    page.settings.paper = PaperSize::A3;
    page.settings.orientation = Orientation::Landscape;
    page.info.title = "Workshop elevation".into();
    page.views[0].name = "Front elevation".into();
    page.views[0].source = drawings::ViewSource::Model {
        entities: vec![source],
    };
    let page = page.clone();
    let pending = sheet_text(&page, None, loading::PreviewState::Generating);
    for label in [
        "A3 · 420 × 297 mm",
        "Workshop elevation",
        "Front elevation",
        "View placeholder",
        "Generating drawing…",
        "Preparing 1 drawing view",
    ] {
        assert!(pending.contains(label), "Missing {label:?}: {pending}");
    }
    assert!(!pending.contains('%'));
    assert!(!pending.contains("Native preview pending"));
    assert_eq!(
        pending.matches("Front elevation").count(),
        1,
        "Pending captions must not collide with placeholder labels: {pending}"
    );
    tests::publish_editor(&mut editor, &session);
    assert!(editor.feed.result.is_some());
    assert!(!editor.feed.pending());
    let ready = sheet_text(
        &page,
        editor.feed.result.as_deref(),
        loading::PreviewState::Idle,
    );
    assert!(!ready.contains("Generating drawing"));
    assert!(!ready.contains("View placeholder"));
    let failed = sheet_text(
        &page,
        None,
        loading::PreviewState::Failed("Model could not be rebuilt"),
    );
    assert!(failed.contains("Drawing generation failed"));
    assert!(failed.contains("Model could not be rebuilt"));
    assert!(!failed.contains("Generating drawing"));
    assert!(failed.contains("Workshop elevation"));
}

#[test]
fn an_unavailable_provider_reports_failure_instead_of_an_endless_generation() {
    let mut feed = evaluation::DrawingFeed::default();
    feed.schedule_page(
        multicad_core::document::Document::new("Unavailable provider").unwrap(),
        0,
        0.,
        EntityId::new(),
    )
    .unwrap();
    assert!(feed.pending());
    assert!(feed.tick(0.));
    assert!(!feed.pending());
    assert_eq!(
        feed.error.as_deref(),
        Some("Native drawing provider is unavailable")
    );
    assert!(!feed.tick(1.));
}
