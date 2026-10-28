use super::*;
use crate::egui::{Event, FullOutput, Pos2, RawInput, Rect, Vec2};

fn fixture() -> (Source, EntityId, EntityId, EntityId) {
    let mut source = Source {
        u: [1., 0., 0.],
        sketch: Default::default(),
    };
    let a = source.add_point([0., 0.]);
    let b = source.add_point([20., 0.]);
    let target = source.add_line(a, b);
    let a = source.add_point([5., -10.]);
    let b = source.add_point([5., 10.]);
    let left = source.add_line(a, b);
    let a = source.add_point([15., -10.]);
    let b = source.add_point([15., 10.]);
    let right = source.add_line(a, b);
    (source, target, left, right)
}
fn frame(
    ctx: &egui::Context,
    panel: &mut Panel,
    source: &Source,
    events: Vec<Event>,
) -> FullOutput {
    let mut output = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300., 760.))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            assert!(panel.show(ui, source, &[], true).is_none());
        },
    );
    output.textures_delta.clear();
    output
}
fn text_center(output: &FullOutput, text: &str) -> Pos2 {
    fn find(shape: &egui::Shape, text: &str) -> Option<Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.size() * 0.5),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|s| find(&s.shape, text))
        .expect("Visible pick-lane control")
}
fn click(ctx: &egui::Context, panel: &mut Panel, source: &Source, pos: Pos2) {
    for pressed in [true, false] {
        frame(
            ctx,
            panel,
            source,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
    }
}
#[test]
fn tool_first_curve_lanes_keep_target_and_toggle_boundary_identities_without_source_edits() {
    let (source, target, left, right) = fixture();
    let before = source.clone();
    let mut panel = Panel::default();
    assert_eq!(panel.curve_pick_lane(), None);
    panel.begin(OperationTool::Trim);
    assert_eq!(panel.curve_pick_lane(), Some(CurvePickLane::Target));
    panel
        .set_curve_pick(&source, target, [8., 0.], false)
        .unwrap();
    assert_eq!(panel.curve_target(), Some(target));
    assert_eq!(panel.curve_pick_lane(), Some(CurvePickLane::Boundaries));
    assert!(
        panel
            .set_curve_pick(&source, target, [8., 0.], true)
            .is_err()
    );
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, right, [15., 0.], true)
        .unwrap();
    assert_eq!(panel.curve_boundaries(), &BTreeSet::from([left, right]));
    assert_eq!(
        panel.curve_selection(),
        BTreeSet::from([target, left, right])
    );
    panel.set_curve_pick(&source, left, [5., 0.], true).unwrap();
    assert_eq!(panel.curve_boundaries(), &BTreeSet::from([right]));
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    let OperationRequest::Trim(TrimRequest::TrimCurve(request), _) =
        panel.request(&source, &[]).unwrap()
    else {
        panic!("Trim request")
    };
    assert_eq!(request.target, target);
    assert_eq!(request.pick_mm, [8., 0.]);
    assert_eq!(request.boundaries, BTreeSet::from([left]));
    assert_eq!(source, before);
    panel.cancel();
    assert_eq!(panel.curve_pick_lane(), None);
    assert!(panel.curve_selection().is_empty());
}
#[test]
fn actual_pick_lane_buttons_retarget_or_change_span_and_bad_picks_leave_the_form_intact() {
    let (source, target, left, _) = fixture();
    let mut panel = Panel::default();
    panel.begin(OperationTool::Trim);
    panel
        .set_curve_pick(&source, target, [8., 0.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(&ctx, &mut panel, &source, text_center(&output, "Pick span"));
    assert_eq!(panel.curve_pick_lane(), Some(CurvePickLane::Span));
    assert!(
        panel
            .set_curve_pick(&source, left, [5., 0.], false)
            .is_err()
    );
    assert!(
        panel
            .set_curve_pick(&source, target, [f64::NAN, 0.], false)
            .is_err()
    );
    assert_eq!(panel.pick_mm, [8., 0.]);
    panel
        .set_curve_pick(&source, target, [12., 0.], false)
        .unwrap();
    assert_eq!(panel.pick_mm, [12., 0.]);
    assert_eq!(panel.curve_boundaries(), &BTreeSet::from([left]));
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(
        &ctx,
        &mut panel,
        &source,
        text_center(&output, "Pick target"),
    );
    assert!(
        panel
            .set_curve_pick(&source, EntityId::new(), [0., 0.], false)
            .is_err()
    );
    assert_eq!(panel.curve_target(), Some(target));
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    assert_eq!(panel.curve_target(), Some(left));
    assert!(panel.curve_boundaries().is_empty());
}
