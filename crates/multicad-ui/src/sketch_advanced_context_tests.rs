//! Real solved choices flow through the editor into advanced tool Inspectors.
use super::*;
use crate::{
    modeling::{AdvancedKind, ModelingTool, native_fixture::NativeBackend},
    viewport::SceneFeed,
};
use multicad_core::{
    constraint_sketch::{ConstraintKind, Curve, CurveKind, Dimension},
    document::Entity,
    parameters::BoundExpression,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

struct Host {
    session: Session,
    editor: SketchEditor,
    feed: SceneFeed,
    ctx: egui::Context,
    sections: [EntityId; 2],
    path: EntityId,
    path_curve: EntityId,
    radius: EntityId,
}
fn section(document: Document, name: &str, z: f64, expression: &str) -> (Document, EntityId) {
    let (sketch, edits) = Source::create(Datum::Xy, name).unwrap();
    let document = document.edited(&edits).unwrap();
    let (mut source, _, plane) = Source::read(&document, sketch).unwrap();
    let center = source.add_point([0., 0.]);
    let curve = EntityId::new();
    source.sketch.curves.push(Curve {
        id: curve,
        construction: false,
        kind: CurveKind::Circle {
            center,
            radius_mm: 1.,
        },
    });
    source.add_constraint(ConstraintKind::Fix { point: center });
    source.add_constraint(ConstraintKind::Radius {
        curve,
        dimension: Dimension::Expression {
            expression: BoundExpression::bind(&document, expression, "mm", None).unwrap(),
        },
    });
    let document = document
        .edited(&source.replace(&document, sketch).unwrap())
        .unwrap();
    let mut plane = document.get(plane).unwrap().clone();
    let EntityData::Plane { origin_mm, .. } = &mut plane.data else {
        panic!()
    };
    origin_mm[2] = z;
    (
        document.edited(&[Mutation::Replace(plane)]).unwrap(),
        sketch,
    )
}
impl Host {
    fn new() -> Self {
        let mut document = Document::new("Solved advanced choices").unwrap();
        let radius = Entity::new(
            "radius",
            EntityData::Parameter {
                expression: "3".into(),
                unit: "mm".into(),
                exposed: true,
                inputs: vec![],
            },
        );
        let radius_id = radius.id;
        document.insert(radius).unwrap();
        let (document, first) = section(document, "First circle", 0., "=$radius");
        let (document, second) = section(document, "Second circle", 20., "=$radius + 1");
        let (path, edits) = Source::create(Datum::Yz, "Path").unwrap();
        let document = document.edited(&edits).unwrap();
        let mut source = Source::read(&document, path).unwrap().0;
        let a = source.add_point([0., 0.]);
        let b = source.add_point([0., 30.]);
        let path_curve = source.add_line(a, b);
        let c = source.add_point([5., 0.]);
        let d = source.add_point([5., 30.]);
        let construction = source.add_line(c, d);
        source
            .sketch
            .curves
            .iter_mut()
            .find(|curve| curve.id == construction)
            .unwrap()
            .construction = true;
        for point in [a, b, c, d] {
            source.add_constraint(ConstraintKind::Fix { point });
        }
        let document = document
            .edited(&source.replace(&document, path).unwrap())
            .unwrap();
        let mut session = Session::new(document).unwrap();
        session.mark_saved(0).unwrap();
        let mut editor = SketchEditor::default();
        editor.set_model_backend(Box::new(NativeBackend::new()));
        editor.set_query_factory(Arc::new(|| {
            Ok(Box::new(multicad_kernel::NativeRegionQueries::default()))
        }));
        let mut feed = SceneFeed::default();
        feed.set_backend(Box::new(NativeBackend::new()));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        Self {
            session,
            editor,
            feed,
            ctx,
            sections: [first, second],
            path,
            path_curve,
            radius: radius_id,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.feed
            .tick(self.session.document(), self.session.revision());
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
                    false,
                    &mut self.feed,
                    &Default::default(),
                )
            },
        )
    }
    fn settle(&mut self) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            self.frame(vec![]).drop_without_applying_deltas();
            if self
                .feed
                .is_current(self.session.document().id(), self.session.revision())
                && [self.sections[0], self.sections[1], self.path]
                    .iter()
                    .all(|id| {
                        self.editor
                            .sketch_evaluation
                            .current(*id)
                            .is_some_and(|presentation| {
                                presentation
                                    .evaluated
                                    .is_current(self.session.document(), self.session.revision())
                            })
                    })
            {
                break;
            }
            assert!(
                Instant::now() < until,
                "{:?}",
                self.editor.sketch_evaluation.errors()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn begin(&mut self, kind: AdvancedKind) {
        self.editor
            .activate_model_tool(
                &self.session,
                &Default::default(),
                &self.feed,
                ModelingTool::Advanced(kind),
                Channel::Ui,
            )
            .unwrap();
        assert!(self.editor.model.advanced_active());
    }
    fn texts(&mut self) -> Vec<String> {
        let out = self.frame(vec![]);
        let texts = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text)
                    if shape
                        .clip_rect
                        .intersects(Rect::from_min_size(text.pos, text.galley.size())) =>
                {
                    Some(text.galley.text().to_owned())
                }
                _ => None,
            })
            .collect();
        out.drop_without_applying_deltas();
        texts
    }
    fn click(&mut self, label: &str) {
        self.frame(vec![]).drop_without_applying_deltas();
        let out = self.frame(vec![]);
        let point = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .or_else(|| {
                out.platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .and_then(|(_, node)| node.bounds())
                    .map(|bounds| {
                        Pos2::new(
                            ((bounds.x0 + bounds.x1) * 0.5) as f32,
                            ((bounds.y0 + bounds.y1) * 0.5) as f32,
                        )
                    })
            })
            .unwrap_or_else(|| panic!("Missing Inspector control {label}"));
        out.drop_without_applying_deltas();
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ])
            .drop_without_applying_deltas();
        }
    }
    fn dismiss_popup(&mut self) {
        for pressed in [true, false] {
            self.frame(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            }])
            .drop_without_applying_deltas();
        }
    }
    fn ready(&mut self) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            self.frame(vec![]).drop_without_applying_deltas();
            if self
                .editor
                .model
                .can_apply(&self.session, &Default::default())
            {
                break;
            }
            assert!(Instant::now() < until, "{:?}", self.editor.model.error());
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn circle_area(&self, index: usize) -> f64 {
        let presentation = self
            .editor
            .sketch_evaluation
            .current(self.sections[index])
            .unwrap();
        assert!(presentation.evaluated.solved().is_some());
        assert_eq!(presentation.stamp.revision, self.session.revision());
        presentation.regions.regions[0].area_mm2
    }
}
#[test]
fn solved_choices_reach_sweep_and_loft_inspectors_and_source_changes_reject_old_candidates() {
    let mut h = Host::new();
    h.settle();
    let original = h.session.document().clone();
    // Both authored radius guesses were 1 mm. These areas require the actual
    // production solver and prove the catalog is not using raw source geometry.
    assert!((h.circle_area(0) - 9. * std::f64::consts::PI).abs() < 1e-6);
    assert!((h.circle_area(1) - 16. * std::f64::consts::PI).abs() < 1e-6);
    h.begin(AdvancedKind::Sweep);
    h.click("Section profile");
    let choices = h.texts();
    assert!(
        choices
            .iter()
            .any(|text| text == "First circle · profile 1 (1 boundary curves)")
    );
    assert!(
        choices
            .iter()
            .any(|text| text == "Second circle · profile 1 (1 boundary curves)")
    );
    assert!(
        !choices
            .iter()
            .any(|text| text.starts_with("Path · profile"))
    );
    h.click("First circle · profile 1 (1 boundary curves)");
    h.dismiss_popup();
    h.click("Add path segment");
    let choices = h.texts();
    assert!(choices.iter().any(|text| text == "Path · Curve 1"));
    assert!(
        !choices.iter().any(|text| text == "Path · Curve 2"),
        "Construction lines cannot become path choices"
    );
    h.click("Path · Curve 1");
    h.dismiss_popup();
    h.ready();
    assert_eq!(h.session.document(), &original);
    assert_eq!(h.session.revision(), 0);
    assert!(
        h.editor
            .model
            .display_scene(&h.session, &Default::default())
            .is_some()
    );

    let mut parameter = h.session.document().get(h.radius).unwrap().clone();
    let EntityData::Parameter { expression, .. } = &mut parameter.data else {
        panic!()
    };
    *expression = "5".into();
    let mut path = Source::read(h.session.document(), h.path).unwrap().0;
    path.sketch
        .curves
        .iter_mut()
        .find(|curve| curve.id == h.path_curve)
        .unwrap()
        .construction = true;
    let mut edits = path.replace(h.session.document(), h.path).unwrap();
    edits.push(Mutation::Replace(parameter));
    h.session
        .apply(Request {
            expected_revision: 0,
            actor: Actor::local(Channel::Mcp),
            label: "Invalidate current solved inputs".into(),
            edits,
        })
        .unwrap();
    h.frame(vec![]).drop_without_applying_deltas();
    assert!(!h.editor.model.can_apply(&h.session, &Default::default()));
    assert!(
        h.editor
            .model
            .display_scene(&h.session, &Default::default())
            .is_none()
    );
    assert!(
        h.editor
            .model
            .preview(&h.session, &Default::default(), Channel::Ui)
            .is_err()
    );
    assert!(
        h.editor
            .model
            .apply(&mut h.session, &Default::default())
            .is_err()
    );
    assert_eq!(
        h.session.revision(),
        1,
        "The stale preview must not add a transaction"
    );
    h.editor.cancel_model();
    h.settle();
    assert!((h.circle_area(0) - 25. * std::f64::consts::PI).abs() < 1e-6);
    assert!((h.circle_area(1) - 36. * std::f64::consts::PI).abs() < 1e-6);
    let changed = h.session.document().clone();
    h.begin(AdvancedKind::Sweep);
    h.click("Add path segment");
    let choices = h.texts();
    assert!(
        !choices
            .iter()
            .any(|text| text == "Path · Curve 1" || text == "Path · Curve 2"),
        "Old/reclassified construction lines remained cached as candidates"
    );
    h.dismiss_popup();
    h.click("Cancel");
    h.begin(AdvancedKind::Loft);
    for label in [
        "First circle · profile 1 (1 boundary curves)",
        "Second circle · profile 1 (1 boundary curves)",
    ] {
        h.click("Add section (ordered)");
        h.click(label);
        h.dismiss_popup();
    }
    h.ready();
    assert!(
        h.editor
            .model
            .display_scene(&h.session, &Default::default())
            .unwrap()
            .world_bounds()
            .is_some()
    );
    h.click("Cancel");
    assert_eq!(h.session.document(), &changed);
    assert_eq!(h.session.revision(), 1);
    assert_eq!(h.session.activity().count(), 1);
}
