use super::*;
use crate::sketch_operations::OperationTool;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
mod review {
    use super::*;
    use multicad_core::{
        constraint_sketch::{Curve, CurveKind, SplineBasis},
        sketch_regions::ExactCurve,
    };
    pub struct Fixture {
        pub session: Session,
        pub sketch: EntityId,
        pub target: EntityId,
        pub boundary: EntityId,
    }
    pub fn seed() -> Result<Fixture, Box<dyn std::error::Error>> {
        let (sketch, edits) = Source::create(Datum::Xy, "Spline host input")?;
        let document = Document::new("Native spline host")?.edited(&edits)?;
        let mut source = Source::read(&document, sketch)?.0;
        let poles = [[0., 0.], [5., 10.], [15., 10.], [20., 0.]]
            .into_iter()
            .map(|p| source.add_point(p))
            .collect();
        let target = EntityId::new();
        source.sketch.curves.push(Curve {
            id: target,
            construction: false,
            kind: CurveKind::BSpline {
                poles,
                basis: SplineBasis::open_uniform(3, 4)?,
            },
        });
        let a = source.add_point([10., -20.]);
        let b = source.add_point([10., 20.]);
        let boundary = source.add_line(a, b);
        source.sketch.curves.last_mut().unwrap().construction = true;
        let session = Session::new(document.edited(&source.replace(&document, sketch)?)?)?;
        Ok(Fixture {
            session,
            sketch,
            target,
            boundary,
        })
    }
    pub fn source(f: &Fixture) -> Result<Source, Box<dyn std::error::Error>> {
        Ok(
            Source::read_for_evaluation(
                f.session.document(),
                f.sketch,
                f.session.revision(),
                None,
            )?
            .0,
        )
    }
    pub fn current_curve(f: &Fixture) -> Result<ExactCurve, Box<dyn std::error::Error>> {
        let source = source(f)?;
        let CurveKind::BSpline { poles, basis } = &source
            .sketch
            .curves
            .iter()
            .find(|c| c.id == f.target)
            .unwrap()
            .kind
        else {
            panic!("Spline");
        };
        Ok(ExactCurve::BSpline {
            poles: poles
                .iter()
                .map(|id| {
                    source
                        .sketch
                        .points
                        .iter()
                        .find(|p| p.id == *id)
                        .unwrap()
                        .position_mm
                })
                .collect(),
            basis: basis.clone(),
        })
    }
    pub fn point_at(f: &Fixture, t: f64) -> Result<[f64; 2], Box<dyn std::error::Error>> {
        let curve = multicad_kernel::NativeRegionQueries::curve(&current_curve(f)?);
        Ok(curve.evaluate(t, Default::default())?.point)
    }
}

fn settle(editor: &mut SketchEditor, session: &Session) {
    editor
        .sketch_evaluation
        .sync(session.document(), session.revision(), None);
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.sketch_evaluation.pending() {
        editor.sketch_evaluation.poll();
        assert!(Instant::now() < deadline, "Sketch worker timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
    let id = editor.selected.unwrap();
    editor.constraints.set_current(
        editor
            .sketch_evaluation
            .current(id)
            .map(|p| p.evaluated.clone()),
    );
    editor.poll_exact_operation(session);
    assert!(editor.sketch_evaluation.current(id).is_some());
}
fn frame(
    ctx: &egui::Context,
    editor: &mut SketchEditor,
    session: &mut Session,
    events: Vec<egui::Event>,
) {
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1360., 860.))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| editor.canvas(ui, session, true, None, false),
    )
    .drop_without_applying_deltas();
}
fn click(ctx: &egui::Context, editor: &mut SketchEditor, session: &mut Session, point: [f64; 2]) {
    let id = editor.selected.unwrap();
    let (_, plane, _) = editor
        .read_sketch(session.document(), id, session.revision())
        .unwrap();
    let p = editor.camera.project(
        plane.point(point).map(|v| v as f32),
        editor.viewport_rect.unwrap(),
    );
    let pos = Pos2::new(p[0], p[1]);
    for pressed in [true, false] {
        frame(
            ctx,
            editor,
            session,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}
fn setup() -> (review::Fixture, SketchEditor, egui::Context) {
    let mut fixture = review::seed().unwrap();
    let mut editor = SketchEditor::default();
    editor.set_query_factory(Arc::new(|| {
        Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
    }));
    editor
        .open_at(
            fixture.session.document(),
            fixture.sketch,
            fixture.session.revision(),
        )
        .unwrap();
    editor.camera.scale = 10.;
    editor.camera.target = [10., 3., 0.];
    settle(&mut editor, &fixture.session);
    let ctx = egui::Context::default();
    frame(&ctx, &mut editor, &mut fixture.session, vec![]);
    (fixture, editor, ctx)
}
fn wait_edit(editor: &mut SketchEditor, session: &Session) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while editor.trim_worker.pending() {
        editor.poll_exact_operation(session);
        assert!(Instant::now() < deadline, "Exact edit timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn pointer_spline_trim_keeps_the_click_view_and_applies_one_native_checked_transaction() {
    let (mut f, mut editor, ctx) = setup();
    let original = f.session.document().clone();
    editor.begin_operation(OperationTool::Trim).unwrap();
    let mut pick = review::point_at(&f, 0.2).unwrap();
    pick[1] += 0.2;
    click(&ctx, &mut editor, &mut f.session, pick);
    assert_eq!(editor.operations.curve_target(), Some(f.target));
    assert!(editor.operations.viewport_span_pick().is_some());
    assert!(editor.constraints.selection().is_empty());
    click(&ctx, &mut editor, &mut f.session, [10., 15.]);
    assert_eq!(
        editor.operations.curve_boundaries(),
        &std::collections::BTreeSet::from([f.boundary])
    );
    // Zooming before Preview must not change the original eight-pixel authority.
    editor.camera.scale = 200.;
    let source = review::source(&f).unwrap();
    let request = editor.operations.request(&source, &[]).unwrap();
    editor.prepare_operation(&f.session, request).unwrap();
    assert!(editor.has_pending_edits());
    assert!(!editor.operation_ready());
    assert_eq!(f.session.document(), &original);
    wait_edit(&mut editor, &f.session);
    assert!(
        editor.constraints.dirty(),
        "{}",
        editor.operation_error.as_deref().unwrap_or("No draft")
    );
    assert!(editor.operation_report.is_some());
    assert_eq!(f.session.document(), &original);
    editor.constraints.apply(&mut f.session).unwrap();
    assert_eq!(f.session.revision(), 1);
    let edited = review::source(&f).unwrap();
    assert!(edited.sketch.curves.iter().any(|c| c.id == f.target));
    let curve = multicad_kernel::NativeRegionQueries::curve(&review::current_curve(&f).unwrap());
    let [a, _] = curve.domain(Default::default()).unwrap();
    let start = curve.evaluate(a, Default::default()).unwrap().point;
    assert!((start[0] - 10.).abs() < 1e-6);
    assert_eq!(
        f.session.activity().last().unwrap().actor.channel,
        Channel::Ui
    );
    f.session.undo(1, Actor::local(Channel::Ui)).unwrap();
    assert_eq!(f.session.document(), &original);
    settle(&mut editor, &f.session);
    assert!(editor.operations.viewport_span_pick().is_none());
}

#[test]
fn a_curve_click_has_priority_over_a_coincident_control_pole() {
    let (mut f, mut editor, ctx) = setup();
    editor.begin_operation(OperationTool::Trim).unwrap();
    click(&ctx, &mut editor, &mut f.session, [0., 0.]);
    assert_eq!(editor.operations.curve_target(), Some(f.target));
    assert!(editor.constraints.selection().is_empty());
    assert!(editor.operations.viewport_span_pick().is_some());
    editor.cancel();
    assert!(!editor.has_pending_edits());
    assert_eq!(f.session.revision(), 0);
}

#[test]
fn a_source_or_session_change_before_first_preview_revokes_the_viewer_pick() {
    for foreign in [false, true] {
        let (mut f, mut editor, ctx) = setup();
        editor.begin_operation(OperationTool::Trim).unwrap();
        let point = review::point_at(&f, 0.2).unwrap();
        click(&ctx, &mut editor, &mut f.session, point);
        assert!(editor.operations.viewport_span_pick().is_some());
        if foreign {
            f.session = Session::new(f.session.document().clone()).unwrap();
        } else {
            f.session
                .apply(Request {
                    expected_revision: 0,
                    actor: Actor::local(Channel::Mcp),
                    label: "External edit before Preview".into(),
                    edits: vec![Mutation::RenameDocument("Updated externally".into())],
                })
                .unwrap();
        }
        let source = f.session.document().clone();
        editor.poll_exact_operation(&f.session);
        assert!(editor.operations.viewport_span_pick().is_none());
        assert!(editor.operation_pick_view.is_none());
        assert!(!editor.constraints.dirty());
        assert_eq!(f.session.document(), &source);
    }
}
