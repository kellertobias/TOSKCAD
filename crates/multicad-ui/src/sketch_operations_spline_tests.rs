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
pub(super) fn frame(
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
pub(super) fn text_center(output: &FullOutput, text: &str) -> Pos2 {
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
pub(super) fn click(ctx: &egui::Context, panel: &mut Panel, source: &Source, pos: Pos2) {
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

fn spline_fixture(rational: bool) -> (Source, EntityId, EntityId, EntityId) {
    use multicad_core::constraint_sketch::{Curve, CurveKind, SplineBasis};
    let (mut source, _, left, right) = fixture();
    let poles = [[0., 1.], [4., 3.], [7., 2.], [10., 1.]]
        .into_iter()
        .map(|point| source.add_point(point))
        .collect();
    let mut basis = SplineBasis::open_uniform(3, 4).unwrap();
    if rational {
        basis.weights = Some(vec![1., 0.5, 2., 1.]);
    }
    let target = EntityId::new();
    source.sketch.curves.push(Curve {
        id: target,
        construction: false,
        kind: CurveKind::BSpline { poles, basis },
    });
    (source, target, left, right)
}
fn spline_request(panel: &Panel, source: &Source) -> ExtendSpline {
    let OperationRequest::Trim(TrimRequest::ExtendSpline(request), _) =
        panel.request(source, &[]).unwrap()
    else {
        panic!("Spline request")
    };
    request
}
fn contacts(left: EntityId, right: EntityId) -> Vec<Contact> {
    // Controller callback fixtures test selection state; they are not geometry proof.
    vec![
        Contact {
            point_mm: [12., 0.],
            parameter: 0.75,
            boundaries: BTreeSet::from([left]),
        },
        Contact {
            point_mm: [18., 0.],
            parameter: 0.9,
            boundaries: BTreeSet::from([right]),
        },
    ]
}
#[test]
fn rational_and_nonrational_spline_forms_bound_continuation_without_altering_the_source() {
    for rational in [false, true] {
        let (source, target, left, _) = spline_fixture(rational);
        let before = source.clone();
        let mut panel = Panel::default();
        panel.begin(OperationTool::Extend);
        panel
            .set_curve_pick(&source, target, [10., 1.], false)
            .unwrap();
        panel
            .set_curve_pick(&source, left, [5., 0.], false)
            .unwrap();
        let request = spline_request(&panel, &source);
        assert_eq!(request.target, target);
        assert_eq!(request.endpoint, Endpoint::End);
        assert_eq!(request.max_length_mm, 100.);
        assert_eq!(request.contact_index, None);
        for invalid in [0., -1., f64::NAN, f64::INFINITY, 1e6 + 1.] {
            panel.max_length_mm = invalid;
            assert!(panel.request(&source, &[]).is_err());
        }
        panel.max_length_mm = 1e6;
        assert_eq!(spline_request(&panel, &source).max_length_mm, 1e6);
        assert_eq!(source, before);
    }
}
#[test]
fn multiple_contacts_require_an_actual_user_choice_and_input_changes_clear_it() {
    let (source, target, left, right) = spline_fixture(true);
    let mut panel = Panel::default();
    panel.begin(OperationTool::Extend);
    panel
        .set_curve_pick(&source, target, [10., 1.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, right, [15., 0.], true)
        .unwrap();
    let request = spline_request(&panel, &source);
    panel
        .set_extension_contacts(&request, contacts(left, right))
        .unwrap();
    assert!(
        panel
            .request(&source, &[])
            .unwrap_err()
            .contains("Choose an exact contact")
    );
    assert_eq!(panel.extension_contact, None);
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(
        &ctx,
        &mut panel,
        &source,
        text_center(&output, "Choose exact contact"),
    );
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(
        &ctx,
        &mut panel,
        &source,
        text_center(&output, "Contact 2 · X 18.000000 / Y 0.000000 mm"),
    );
    assert_eq!(spline_request(&panel, &source).contact_index, Some(1));
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(
        &ctx,
        &mut panel,
        &source,
        text_center(&output, "End endpoint"),
    );
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(
        &ctx,
        &mut panel,
        &source,
        text_center(&output, "Start endpoint"),
    );
    assert!(panel.extension_contacts.is_none());
    assert_eq!(spline_request(&panel, &source).contact_index, None);
    assert_eq!(panel.endpoint, Endpoint::Start);
    assert!(
        panel
            .set_extension_contacts(&request, contacts(left, right))
            .is_err()
    );
}
#[test]
fn contact_payloads_are_bounded_ordered_and_bound_to_the_exact_operation_inputs() {
    let (source, target, left, right) = spline_fixture(false);
    let mut panel = Panel::default();
    panel.begin(OperationTool::Extend);
    panel
        .set_curve_pick(&source, target, [10., 1.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, right, [15., 0.], true)
        .unwrap();
    let request = spline_request(&panel, &source);
    let good = contacts(left, right);
    for bad in [
        vec![good[1].clone(), good[0].clone()],
        vec![good[0].clone(); 257],
        vec![Contact {
            parameter: f64::NAN,
            ..good[0].clone()
        }],
        vec![Contact {
            point_mm: [f64::INFINITY, 0.],
            ..good[0].clone()
        }],
        vec![Contact {
            boundaries: BTreeSet::from([target]),
            ..good[0].clone()
        }],
    ] {
        assert!(panel.set_extension_contacts(&request, bad).is_err());
        assert!(panel.extension_contacts.is_none());
    }
    panel
        .set_extension_contacts(&request, vec![good[0].clone()])
        .unwrap();
    assert_eq!(spline_request(&panel, &source).contact_index, None); // Single hit needs no arbitrary choice.
    panel.clear_extension_contacts();
    assert!(panel.extension_contacts.is_none());
    panel.max_length_mm = 50.;
    assert!(panel.set_extension_contacts(&request, good).is_err());
    let changed = spline_request(&panel, &source);
    panel.set_extension_contacts(&changed, vec![]).unwrap();
    assert!(
        panel
            .request(&source, &[])
            .unwrap_err()
            .contains("No exact boundary contact")
    );
    panel.cancel();
    assert!(panel.extension_contacts.is_none());
}
#[test]
fn a_spline_can_be_a_boundary_for_an_analytic_target_and_a_target_for_trim() {
    let (source, spline, left, _) = spline_fixture(false);
    let mut panel = Panel::default();
    panel.begin(OperationTool::Trim);
    panel
        .set_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    panel
        .set_curve_pick(&source, spline, [7., 1.], false)
        .unwrap();
    let OperationRequest::Trim(TrimRequest::TrimCurve(request), _) =
        panel.request(&source, &[]).unwrap()
    else {
        panic!("Exact trim")
    };
    assert_eq!(request.target, left);
    assert_eq!(request.boundaries, BTreeSet::from([spline]));
}

#[test]
fn only_recorded_viewport_span_picks_can_request_projection_and_numeric_edits_revoke_it() {
    let (source, target, left, _) = fixture();
    let mut panel = Panel::default();
    panel.begin(OperationTool::Trim);
    panel
        .set_viewport_curve_pick(&source, target, [8., 0.05], false)
        .unwrap();
    assert_eq!(panel.viewport_span_pick(), Some((target, [8., 0.05])));
    panel
        .set_viewport_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    assert_eq!(panel.viewport_span_pick(), Some((target, [8., 0.05])));
    panel.pick_lane = CurvePickLane::Span;
    panel
        .set_curve_pick(&source, target, [8., 0.05], false)
        .unwrap();
    assert_eq!(panel.viewport_span_pick(), None);
    panel.pick_lane = CurvePickLane::Span;
    panel
        .set_viewport_curve_pick(&source, target, [9., 0.02], false)
        .unwrap();
    panel.clear_viewport_pick();
    assert_eq!(panel.viewport_span_pick(), None);
    panel.pick_lane = CurvePickLane::Target;
    panel
        .set_viewport_curve_pick(&source, target, [7., 0.01], false)
        .unwrap();
    panel.pick_mm = [6., 0.01];
    assert_eq!(panel.viewport_span_pick(), None);
    panel.pick_lane = CurvePickLane::Target;
    panel
        .set_viewport_curve_pick(&source, left, [5., 0.], false)
        .unwrap();
    assert_eq!(panel.viewport_span_pick(), Some((left, [5., 0.])));
    panel.cancel();
    assert_eq!(panel.viewport_span_pick(), None);
    panel.begin(OperationTool::Extend);
    panel
        .set_viewport_curve_pick(&source, target, [8., 0.05], false)
        .unwrap();
    assert_eq!(panel.viewport_span_pick(), None);
}

#[test]
fn typing_numeric_span_coordinates_revokes_the_viewport_projection_marker() {
    let (source, target, _, _) = fixture();
    let mut panel = Panel::default();
    panel.begin(OperationTool::Trim);
    panel
        .set_viewport_curve_pick(&source, target, [8., 0.05], false)
        .unwrap();
    let ctx = egui::Context::default();
    let output = frame(&ctx, &mut panel, &source, vec![]);
    click(&ctx, &mut panel, &source, text_center(&output, "8.0"));
    frame(&ctx, &mut panel, &source, vec![Event::Text("9".into())]).drop_without_applying_deltas();
    assert_eq!(panel.pick_mm[0], 9.);
    assert_eq!(panel.viewport_span_pick(), None);
    assert!(panel.viewport_pick.is_none());
}
