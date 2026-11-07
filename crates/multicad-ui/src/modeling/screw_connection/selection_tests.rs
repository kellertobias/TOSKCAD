//! Painted display focus changes neither authored source nor preview identity.
use super::*;
#[path = "../../../../multicad-mcp/tests/support/screw_connection.rs"]
mod fixture;
fn paint(
    ctx: &egui::Context,
    panel: &mut ScrewConnectionPanel,
    document: &Document,
) -> egui::FullOutput {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(244., 360.),
            )),
            ..Default::default()
        },
        |ui| {
            egui::ScrollArea::vertical()
                .id_salt("component-focus-scroll")
                .show(ui, |ui| {
                    crate::inspector_actions::scope(
                        ui,
                        Some(crate::inspector_actions::Owner::Model),
                        crate::registry::Mode::Model.color(),
                        |ui| {
                            panel.show(ui, document, true, PreviewState::Idle);
                        },
                    );
                });
        },
    )
}
fn request(panel: &ScrewConnectionPanel, document: &Document) -> ScrewConnectionRequest {
    let ScrewConnectionCommand::Set(request) = panel.request(document, true).unwrap() else {
        panic!("set")
    };
    *request
}
#[test]
fn painted_component_focus_scrolls_to_hardware_and_receiver_without_changing_request_or_stamp() {
    let (document, source, _) = fixture::fixture();
    let original_document = document.clone();
    let mut panel = ScrewConnectionPanel::from_request(&document, source.clone(), false).unwrap();
    let context = egui::Context::default();
    context.style_mut_of(egui::Theme::Dark, |style| style.animation_time = 0.);
    context.style_mut_of(egui::Theme::Light, |style| style.animation_time = 0.);
    for _ in 0..2 {
        paint(&context, &mut panel, &document).drop_without_applying_deltas();
    }
    let stamp = panel.stamp();
    let before = request(&panel, &document);
    for path in [
        OccurrencePath {
            root: source.hardware[1].instance,
            members: vec![],
        },
        source.receivers[0].source.occurrence.clone(),
        OccurrencePath {
            root: source.hardware[0].instance,
            members: vec![],
        },
    ] {
        assert!(panel.focus_path(&path));
        let mut visible = false;
        for _ in 0..20 {
            let output = paint(&context, &mut panel, &document);
            visible|=output.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text=="Selected in preview" && shape.clip_rect.contains(text.pos+text.galley.size()*0.5)));
            output.drop_without_applying_deltas();
        }
        assert!(
            visible,
            "The selected card must scroll into the244pt Inspector viewport"
        );
        assert_eq!(panel.stamp(), stamp);
        assert_eq!(request(&panel, &document), before);
        assert!(panel.focused_path(&path));
    }
    let focused = panel.focus;
    panel.pick = Some(ScrewConnectionPick {
        receiver: source.axis.source.receiver.clone(),
        occurrence: source.axis.source.occurrence.clone(),
        kind: ScrewConnectionPickKind::EntryAxis,
    });
    assert!(!panel.focus_path(&source.receivers[0].source.occurrence));
    assert_eq!(panel.focus, focused, "An active face picker keeps priority");
    assert_eq!(document, original_document);
}
