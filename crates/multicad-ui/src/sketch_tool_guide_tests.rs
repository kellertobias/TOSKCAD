//! Real-host Inspector tests for drawing-tool guidance (CAD-158): the
//! Inspector shown by `show_project` and pointer clicks through the canvas.
use super::*;

struct Host {
    ctx: egui::Context,
    editor: SketchEditor,
    session: Session,
    feed: crate::viewport::SceneFeed,
}

impl Host {
    fn new(tool: DrawTool) -> Self {
        let mut session = Session::new(Document::new("Tool guide").unwrap()).unwrap();
        let mut editor = SketchEditor::default();
        editor.create(&mut session, Datum::Xy, Channel::Ui).unwrap();
        editor.set_tool(tool).unwrap();
        let mut h = Self {
            ctx: egui::Context::default(),
            editor,
            session,
            feed: Default::default(),
        };
        h.ctx.enable_accesskit();
        h.settle();
        h
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1360., 1000.))),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                self.editor.show_project(
                    ui,
                    &mut self.session,
                    true,
                    &mut self.feed,
                    &Default::default(),
                )
            },
        )
    }
    fn settle(&mut self) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            self.frame(vec![]).drop_without_applying_deltas();
            if !self.editor.sketch_evaluation.pending() {
                break;
            }
            assert!(std::time::Instant::now() < until, "Sketch worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        self.frame(vec![]).drop_without_applying_deltas();
    }
    fn texts(&mut self) -> Vec<String> {
        let out = self.frame(vec![]);
        let texts = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        out.drop_without_applying_deltas();
        texts
    }
    fn shows(&mut self, wanted: &str) -> bool {
        self.texts().iter().any(|t| t == wanted)
    }
    fn screen(&self, p: [f64; 2]) -> Pos2 {
        let s = self.editor.camera.project(
            Datum::Xy.frame().point(p).map(|v| v as f32),
            self.editor.viewport_rect.unwrap(),
        );
        Pos2::new(s[0], s[1])
    }
    fn press(&mut self, at: Pos2) {
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ])
            .drop_without_applying_deltas();
        }
        self.settle();
    }
    fn click(&mut self, p: [f64; 2]) {
        let at = self.screen(p);
        self.press(at);
    }
    fn click_label(&mut self, label: &str) {
        let out = self.frame(vec![]);
        let at = out
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
        self.press(at);
    }
    fn geometry(&self) -> Vec<Geometry> {
        let id = self.editor.selected.unwrap();
        self.editor
            .read_sketch(self.session.document(), id, self.session.revision())
            .unwrap()
            .0
            .geometry
    }
}

/// A committed rectangle is four connected constrained lines.
fn assert_rectangle(geometry: &[Geometry]) {
    assert_eq!(geometry.len(), 4, "{geometry:?}");
    assert!(geometry.iter().all(|g| matches!(g, Geometry::Line { .. })));
}

#[test]
fn rectangle_inspector_prompts_corners_without_apply_and_commits_on_the_second_click() {
    let mut h = Host::new(DrawTool::Rectangle);
    let texts = h.texts();
    assert!(
        texts
            .iter()
            .any(|t| t == "Step 1 of 2 · Click the first corner")
    );
    assert!(texts.iter().any(|t| t == "No pending input"));
    for absent in ["Apply", "Finish spline", "rect <x1> <y1> <x2> <y2> (mm)"] {
        assert!(
            !texts.iter().any(|t| t == absent),
            "{absent} must not be shown for Rectangle"
        );
    }
    // Command syntax stays available as optional help.
    h.click_label("Command entry");
    assert!(h.shows("rect <x1> <y1> <x2> <y2> (mm)"));
    let before = h.session.revision();
    h.click([0., 0.]);
    assert_eq!(
        h.session.revision(),
        before,
        "The first corner is pending only"
    );
    assert!(h.shows("Step 2 of 2 · Click the opposite corner"));
    assert!(h.shows("Pending: first corner · not in the sketch yet"));
    assert!(h.editor.message.contains("opposite corner"));
    h.click([40., 20.]);
    assert_eq!(h.session.revision(), before + 1);
    assert_rectangle(&h.geometry());
    assert!(
        h.editor
            .message
            .starts_with("Rectangle committed · Undo removes it"),
        "{}",
        h.editor.message
    );
    assert!(h.shows("Step 1 of 2 · Click the first corner"));
    // Escape semantics: ending the tool keeps the committed rectangle.
    h.click([10., 10.]);
    assert!(h.shows("Pending: first corner · not in the sketch yet"));
    h.click_label("End Rectangle (Esc)");
    assert_eq!(h.editor.tool, None);
    assert_eq!(h.session.revision(), before + 1);
    assert_rectangle(&h.geometry());
}

#[test]
fn clear_pending_points_keeps_the_tool_and_committed_geometry() {
    let mut h = Host::new(DrawTool::Line);
    h.click([0., 0.]);
    h.click([30., 0.]);
    let revision = h.session.revision();
    h.click([0., 20.]);
    assert!(h.shows("Pending: start point · not in the sketch yet"));
    h.click_label("Clear pending points");
    assert_eq!(h.editor.tool, Some(DrawTool::Line));
    assert!(h.shows("Step 1 of 2 · Click the start point"));
    assert_eq!(h.session.revision(), revision);
    assert_eq!(h.geometry().len(), 1);
}

#[test]
fn circle_and_polyline_describe_their_pointer_workflows() {
    let mut h = Host::new(DrawTool::Circle);
    assert!(h.shows("Step 1 of 2 · Click the center"));
    h.click([0., 0.]);
    assert!(h.shows("Step 2 of 2 · Click the point on the circle that sets the radius"));
    h.click([10., 0.]);
    assert!(matches!(h.geometry()[..], [Geometry::Circle { .. }]));

    let mut h = Host::new(DrawTool::Polyline);
    assert!(h.shows("Step 1 · Click the first vertex"));
    h.click([0., 0.]);
    h.click([20., 0.]);
    h.click([20., 20.]);
    assert!(h.shows("2 segments already committed · the chain continues from its last vertex"));
    assert!(h.shows(
        "Esc stops the chain and ends the tool. Committed segments stay in the sketch; Undo removes them."
    ));
}

#[test]
fn spline_keeps_its_guarded_finish_action() {
    let mut h = Host::new(DrawTool::Spline);
    // The fixed action bar normalizes the finish action to Apply; it stays
    // disabled until four control poles exist.
    assert!(h.shows("Apply"));
    assert!(!h.editor.can_finish_spline());
    for p in [[0., 0.], [0., 30.], [40., 30.]] {
        h.click(p);
    }
    assert!(h.shows("Pending: 3 control poles · not in the sketch yet"));
    h.click_label("Apply");
    assert!(
        h.geometry().is_empty(),
        "Apply stays guarded below four poles"
    );
    h.click([40., 0.]);
    assert!(h.editor.can_finish_spline());
    h.click_label("Apply");
    assert!(matches!(h.geometry()[..], [Geometry::BSpline { .. }]));
}
