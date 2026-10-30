//! Editor-level interaction tests for sketch snapping (CAD-136). Pointer
//! input runs through the real canvas with injected egui time; commits go
//! through the shared sketch draw transaction.
use super::*;
use snap::{DWELL_SECONDS, TargetKind};

fn session() -> Session {
    Session::new(Document::new("Snap tests").unwrap()).unwrap()
}
fn settle(editor: &mut SketchEditor, session: &Session) {
    if let Some(report) = editor
        .evaluated
        .clone()
        .filter(|r| r.revision == session.revision())
    {
        editor.sketch_evaluation.sync_creation_report(
            session.document(),
            session.revision(),
            report,
        );
    } else {
        editor
            .sketch_evaluation
            .sync(session.document(), session.revision(), None);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while editor.sketch_evaluation.pending() {
        editor.sketch_evaluation.poll();
        assert!(
            std::time::Instant::now() < deadline,
            "Sketch worker timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

struct Harness {
    ctx: egui::Context,
    editor: SketchEditor,
    session: Session,
}
impl Harness {
    /// An XY sketch with a line tool active and the viewport laid out.
    fn new(setup: impl FnOnce(&mut SketchEditor, &mut Session)) -> Self {
        let mut session = session();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        setup(&mut editor, &mut session);
        editor.set_tool(DrawTool::Line).unwrap();
        let mut h = Self {
            ctx: egui::Context::default(),
            editor,
            session,
        };
        h.frame(vec![], 0.);
        h.frame(vec![], 0.01);
        h
    }
    fn frame(&mut self, events: Vec<egui::Event>, time: f64) {
        settle(&mut self.editor, &self.session);
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000., 700.))),
            events,
            time: Some(time),
            ..Default::default()
        };
        let (editor, session) = (&mut self.editor, &mut self.session);
        self.ctx
            .run_ui(raw, |ui| editor.canvas(ui, session, true, None, false))
            .drop_without_applying_deltas();
        settle(&mut self.editor, &self.session);
    }
    fn screen(&self, p: [f64; 2]) -> Pos2 {
        let frame = Datum::Xy.frame();
        let s = self.editor.camera.project(
            frame.point(p).map(|v| v as f32),
            self.editor.viewport_rect.unwrap(),
        );
        Pos2::new(s[0], s[1])
    }
    /// Screen pixels per sketch millimeter along X.
    fn px_per_mm(&self) -> f32 {
        self.screen([10., 0.]).distance(self.screen([0., 0.])) / 10.
    }
    fn hover(&mut self, p: [f64; 2], offset: Vec2, time: f64) {
        let at = self.screen(p) + offset;
        self.frame(vec![egui::Event::PointerMoved(at)], time);
    }
    fn click(&mut self, p: [f64; 2], offset: Vec2, time: f64) {
        let at = self.screen(p) + offset;
        for (pressed, t) in [(true, time), (false, time + 0.01)] {
            self.frame(
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                t,
            );
        }
    }
    fn points(&self) -> Vec<[f64; 2]> {
        let id = self.editor.selected.unwrap();
        Source::read(self.session.document(), id)
            .unwrap()
            .0
            .sketch
            .points
            .iter()
            .map(|p| p.position_mm)
            .collect()
    }
    fn shown(&self) -> Option<TargetKind> {
        self.editor.snap_assist.shown().map(|t| t.kind)
    }
}

/// Two separate lines whose endpoints (0,0) and (20,10) are not joined, so
/// the only target at (10,5) is the inferred reference midpoint.
fn two_reference_lines(editor: &mut SketchEditor, session: &mut Session) {
    editor
        .add(session, DrawTool::Line, [0., 0.], [0., -15.], Channel::Ui)
        .unwrap();
    editor
        .add(session, DrawTool::Line, [20., 10.], [35., 10.], Channel::Ui)
        .unwrap();
}

#[test]
fn dwell_acquires_references_without_mutating_the_document() {
    let mut h = Harness::new(two_reference_lines);
    let before = h.session.document().clone();
    let revision = h.session.revision();
    let near = Vec2::new(2., -1.);
    // Briefly passing over the endpoint does not acquire it.
    h.hover([0., 0.], near, 1.);
    h.hover([60., 60.], Vec2::ZERO, 1.2);
    h.hover([0., 0.], near, 1.4);
    h.hover([0., 0.], near, 1.4 + DWELL_SECONDS - 0.05);
    assert!(h.editor.snap_assist.references().is_empty());
    assert_eq!(h.shown(), Some(TargetKind::Endpoint));
    h.hover([0., 0.], near, 1.4 + DWELL_SECONDS);
    let refs = h.editor.snap_assist.references();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].position, [0., 0.], "cross at the exact point");
    h.hover([20., 10.], near, 3.);
    h.hover([20., 10.], near, 3. + DWELL_SECONDS);
    assert_eq!(h.editor.snap_assist.references().len(), 2);
    assert_eq!(
        h.session.document(),
        &before,
        "aids never edit the document"
    );
    assert_eq!(
        h.session.revision(),
        revision,
        "no transaction, dirty state or Undo"
    );
}

#[test]
fn reference_midpoint_placement_commits_exactly_ten_five_with_grid_on_and_off() {
    for grid in [true, false] {
        let mut h = Harness::new(two_reference_lines);
        h.editor.snap = grid;
        let near = Vec2::new(1., 1.);
        h.hover([0., 0.], near, 1.);
        h.hover([0., 0.], near, 1. + DWELL_SECONDS);
        h.hover([20., 10.], near, 2.);
        h.hover([20., 10.], near, 2. + DWELL_SECONDS);
        // 3 px away from the midpoint; the grid point (10,10) is ignored.
        h.hover([10., 5.], Vec2::new(2., -2.), 3.);
        assert_eq!(h.shown(), Some(TargetKind::ReferenceMidpoint));
        h.click([10., 5.], Vec2::new(2., -2.), 3.1);
        assert_eq!(h.editor.first, Some([10., 5.]), "shown target is placed");
        let revision = h.session.revision();
        h.click([60., -40.], Vec2::ZERO, 4.);
        assert_eq!(h.session.revision(), revision + 1, "one shared transaction");
        let points = h.points();
        assert!(
            points.contains(&[10., 5.]),
            "grid {grid}: committed {points:?}"
        );
    }
}

#[test]
fn line_and_assistive_line_snaps_beat_a_nearby_grid_point() {
    for grid in [true, false] {
        let mm = Harness::new(|_, _| {}).px_per_mm() as f64;
        // A line 3 px above the grid row y=0, off every grid point.
        let y = 3. / mm;
        let mut h = Harness::new(|editor, session| {
            editor
                .add(session, DrawTool::Line, [-40., y], [40., y], Channel::Ui)
                .unwrap();
        });
        h.editor.snap = grid;
        // The pointer sits exactly on grid point (10,0).
        h.hover([10., 0.], Vec2::ZERO, 1.);
        assert_eq!(h.shown(), Some(TargetKind::OnCurve));
        h.click([10., 0.], Vec2::ZERO, 1.1);
        let first = h.editor.first.unwrap();
        assert_eq!(first[1], y, "grid {grid}: on the line, not the grid");
        assert!((first[0] - 10.).abs() < 1e-3);
        h.editor.cancel();
        h.editor.set_tool(DrawTool::Line).unwrap();
        // Acquire the line end, then follow its vertical assistive line.
        h.hover([40., y], Vec2::new(1., 1.), 2.);
        h.hover([40., y], Vec2::new(1., 1.), 2. + DWELL_SECONDS);
        assert_eq!(h.editor.snap_assist.references().len(), 1);
        let pointer = Vec2::new(-2., 0.);
        h.hover([40., 37.3], pointer, 3.);
        assert_eq!(
            h.shown(),
            Some(TargetKind::Guide(snap::Axis::Vertical)),
            "grid {grid}"
        );
        h.click([40., 37.3], pointer, 3.1);
        let first = h.editor.first.unwrap();
        assert_eq!(first[0], 40., "grid {grid}: on the assistive line");
        assert!(
            (first[1] - 37.3).abs() < 0.05,
            "grid did not round {first:?}"
        );
    }
}

#[test]
fn grid_applies_only_without_a_geometry_target() {
    let mut h = Harness::new(|_, _| {});
    h.click([23.4, 16.2], Vec2::ZERO, 1.);
    assert_eq!(h.editor.first, Some([20., 20.]));
    h.editor.cancel();
    h.editor.set_tool(DrawTool::Line).unwrap();
    h.editor.snap = false;
    h.click([23.4, 16.2], Vec2::ZERO, 2.);
    let first = h.editor.first.unwrap();
    assert!((first[0] - 23.4).abs() < 1e-3 && (first[1] - 16.2).abs() < 1e-3);
}

#[test]
fn rectangle_center_and_corner_targets_place_their_exact_positions() {
    let mut h = Harness::new(|editor, session| {
        editor
            .add(
                session,
                DrawTool::Rectangle,
                [0., 0.],
                [40., 20.],
                Channel::Ui,
            )
            .unwrap();
    });
    h.hover([20., 10.], Vec2::new(3., 2.), 1.);
    assert_eq!(
        h.shown(),
        Some(TargetKind::ShapeCenter(snap::ShapeKind::Rectangle))
    );
    h.click([20., 10.], Vec2::new(3., 2.), 1.1);
    assert_eq!(h.editor.first, Some([20., 10.]));
    assert_eq!(h.editor.first_anchor, None);
    let points = h.points().len();
    h.click([40., 20.], Vec2::new(-3., 2.), 2.);
    assert_eq!(h.points().len(), points + 1, "corner reused, center added");
    assert!(h.points().contains(&[20., 10.]));
    // The midpoint of the right edge is an exact target as well.
    h.hover([40., 10.], Vec2::new(2., 1.), 3.);
    assert_eq!(h.shown(), Some(TargetKind::LineMidpoint));
}

#[test]
fn aids_stay_anchored_through_pan_and_zoom() {
    let mut h = Harness::new(two_reference_lines);
    h.hover([20., 10.], Vec2::new(1., 1.), 1.);
    h.hover([20., 10.], Vec2::new(1., 1.), 1. + DWELL_SECONDS);
    assert_eq!(h.editor.snap_assist.references().len(), 1);
    h.editor.camera.scale *= 2.;
    h.editor.camera.pan(Vec2::new(40., -20.));
    let at = h.screen([45., 10.]);
    assert!(h.editor.viewport_rect.unwrap().contains(at), "{at:?}");
    // A 2 px offset in the new view still resolves the same alignment.
    h.hover([45., 10.], Vec2::new(0., 2.), 2.);
    let target = h.editor.snap_assist.shown().unwrap();
    assert_eq!(target.kind, TargetKind::Guide(snap::Axis::Horizontal));
    assert_eq!(target.position[1], 10.);
    assert_eq!(h.editor.snap_assist.references()[0].position, [20., 10.]);
}

#[test]
fn cancel_tool_switch_and_leaving_sketch_clear_aids_without_edits() {
    let mut h = Harness::new(two_reference_lines);
    let acquire = |h: &mut Harness, t: f64| {
        h.hover([0., 0.], Vec2::new(1., 1.), t);
        h.hover([0., 0.], Vec2::new(1., 1.), t + DWELL_SECONDS);
        assert_eq!(h.editor.snap_assist.references().len(), 1);
    };
    let before = h.session.document().clone();
    let revision = h.session.revision();
    acquire(&mut h, 1.);
    h.editor.cancel();
    assert!(h.editor.snap_assist.references().is_empty());
    h.editor.set_tool(DrawTool::Line).unwrap();
    acquire(&mut h, 2.);
    h.editor.set_tool(DrawTool::Circle).unwrap();
    assert!(h.editor.snap_assist.references().is_empty());
    acquire(&mut h, 3.);
    h.editor.clear_snap_references();
    assert!(h.editor.snap_assist.references().is_empty());
    acquire(&mut h, 4.);
    h.editor.finish().unwrap();
    h.frame(vec![], 5.);
    assert!(h.editor.snap_assist.references().is_empty());
    assert!(h.editor.snap_assist.shown().is_none());
    assert_eq!(h.session.document(), &before);
    assert_eq!(h.session.revision(), revision);
}

#[test]
fn geometry_snapping_can_be_disabled_independently_of_the_grid() {
    let mut h = Harness::new(two_reference_lines);
    h.editor.geometry_snap = false;
    h.editor.snap = false;
    h.click([0., 0.], Vec2::new(2., 1.), 1.);
    let first = h.editor.first.unwrap();
    assert_ne!(first, [0., 0.]);
    assert!(h.editor.first_anchor.is_none());
}

#[test]
fn perpendicular_snap_commits_the_exact_foot_with_grid_on_and_off() {
    let ab = [40., 30.];
    for grid in [true, false] {
        let mut h = Harness::new(|editor, session| {
            editor
                .add(session, DrawTool::Line, [0., 0.], ab, Channel::Ui)
                .unwrap();
        });
        h.editor.snap = grid;
        h.click([10., 20.], Vec2::ZERO, 1.);
        let first = h.editor.first.expect("line started");
        // Foot from (10,20) onto the line: t = 0.4 -> (16,12), off the grid.
        let offset = Vec2::new(3., -2.);
        h.hover([16., 12.], offset, 2.);
        assert_eq!(h.shown(), Some(TargetKind::Perpendicular), "grid {grid}");
        let shown = h.editor.snap_assist.shown().unwrap().position;
        let revision = h.session.revision();
        h.click([16., 12.], offset, 2.1);
        assert_eq!(h.session.revision(), revision + 1, "one shared transaction");
        let end = h
            .points()
            .into_iter()
            .find(|p| (p[0] - shown[0]).abs() < 1e-12 && (p[1] - shown[1]).abs() < 1e-12)
            .unwrap_or_else(|| panic!("grid {grid}: shown {shown:?} not committed"));
        let drawn = [end[0] - first[0], end[1] - first[1]];
        let length = drawn[0].hypot(drawn[1]);
        assert!(length > 5., "grid {grid}: {drawn:?}");
        assert!(
            (drawn[0] * ab[0] + drawn[1] * ab[1]).abs() < 1e-9 * length * 50.,
            "grid {grid}: not perpendicular {drawn:?}"
        );
        assert!(
            (end[0] * ab[1] - end[1] * ab[0]).abs() < 1e-9 * 50.,
            "grid {grid}: foot left the target line {end:?}"
        );
    }
}

#[test]
fn perpendicular_targets_only_appear_while_a_line_has_a_start() {
    let mut h = Harness::new(|editor, session| {
        editor
            .add(session, DrawTool::Line, [0., 0.], [40., 30.], Channel::Ui)
            .unwrap();
    });
    h.editor.snap = false;
    // Before the first click the same spot is just a point on the line.
    h.hover([16., 12.], Vec2::new(3., -2.), 1.);
    assert_eq!(h.shown(), Some(TargetKind::OnCurve));
    h.click([10., 20.], Vec2::ZERO, 2.);
    h.hover([16., 12.], Vec2::new(3., -2.), 3.);
    assert_eq!(h.shown(), Some(TargetKind::Perpendicular));
    h.editor.cancel();
    h.editor.set_tool(DrawTool::Circle).unwrap();
    h.click([10., 20.], Vec2::ZERO, 4.);
    h.hover([16., 12.], Vec2::new(3., -2.), 5.);
    assert_eq!(h.shown(), Some(TargetKind::OnCurve), "circles have no foot");
}
