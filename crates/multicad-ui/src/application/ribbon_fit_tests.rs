//! CAD-154: the ribbon panel must contain every tool tile label, group
//! caption and pinned session action, at the reviewed 1360 × 860 window and
//! on resize, so no content panel paints over them.
use super::*;
struct Storage;
impl DocumentStorage for Storage {
    fn save(&mut self, _: &std::path::Path, _: &Document) -> Result<(), String> {
        Ok(())
    }
    fn load(&mut self, _: &std::path::Path) -> Result<Document, String> {
        Err("unused".into())
    }
}
struct Painted {
    text: String,
    rect: egui::Rect,
    clip: egui::Rect,
    lines: usize,
}
struct Frame {
    ribbon: egui::Rect,
    texts: Vec<Painted>,
}
fn frame(app: &mut Application, ctx: &egui::Context, size: egui::Vec2) -> Frame {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    let panel = if app.mode == Mode::Export {
        "project-export-ribbon"
    } else {
        "ribbon"
    };
    let ribbon = egui::PanelState::load(ctx, egui::Id::new(panel))
        .expect("ribbon panel")
        .outer_rect;
    let texts = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if !text.galley.text().trim().is_empty() => Some(Painted {
                text: text.galley.text().replace('\n', " "),
                rect: egui::Rect::from_min_size(text.pos, text.galley.size()),
                clip: shape.clip_rect,
                lines: text.galley.rows.len(),
            }),
            _ => None,
        })
        .collect();
    output.drop_without_applying_deltas();
    Frame { ribbon, texts }
}
/// Every (mode, tab, sketch editing) combination the user can reach.
fn surfaces(app: &Application) -> Vec<(Mode, usize, bool)> {
    let mut all = vec![];
    for workspace in &app.registry.workspaces {
        if workspace.mode == Mode::Parameters {
            continue;
        }
        for (index, tab) in workspace.tabs.iter().enumerate() {
            for editing in [false, true] {
                if workspace.mode != Mode::Sketch && editing {
                    continue;
                }
                if tab.requires_active_sketch && !editing {
                    continue;
                }
                all.push((workspace.mode, index, editing));
            }
        }
    }
    all
}
fn check(size: egui::Vec2, pixels_per_point: f32) {
    let mut app = Application::new(Box::new(Storage)).unwrap();
    app.pending_files.clear();
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(pixels_per_point);
    let catalog: std::collections::BTreeSet<String> = app
        .registry
        .workspaces
        .iter()
        .flat_map(|w| w.tabs.iter().flat_map(|t| t.tools.iter()))
        .filter_map(|id| app.registry.tool(id).map(|tool| tool.label.to_string()))
        .chain(["Add Primitive", "Boolean", "Pattern", "Rectangle", "Circle"].map(String::from))
        .collect();
    for (mode, tab, editing) in surfaces(&app) {
        app.mode = mode;
        app.tab = tab;
        app.current_mut().sketch.editing = editing;
        frame(&mut app, &ctx, size);
        let painted = frame(&mut app, &ctx, size);
        let ribbon = painted.ribbon;
        let place = format!(
            "{} / {} (editing {editing}) at {size:?} × {pixels_per_point}",
            mode.title(),
            app.registry.workspace(mode).tabs[tab].title
        );
        // Tool tile labels: painted inside the ribbon's clip, below the tab strip.
        let tiles: Vec<&Painted> = painted
            .texts
            .iter()
            .filter(|t| {
                t.clip.max.y <= ribbon.max.y + 0.5
                    && t.clip.min.y >= ribbon.min.y - 0.5
                    && t.rect.min.y > ribbon.min.y + if mode == Mode::Export { 0. } else { 40. }
                    && catalog.contains(&t.text)
            })
            .collect();
        for tile in &tiles {
            assert!(
                tile.lines <= 2,
                "{place}: {} wraps to {} lines",
                tile.text,
                tile.lines
            );
            let visible = tile.rect.intersect(tile.clip);
            // Horizontally a tile may scroll out of view; vertically it must
            // always be complete with bottom padding inside the ribbon.
            if visible.width() > 0. {
                assert!(
                    tile.rect.max.y + 4. <= ribbon.max.y && tile.rect.max.y <= tile.clip.max.y,
                    "{place}: {} label {:?} crosses ribbon bottom {}",
                    tile.text,
                    tile.rect,
                    ribbon.max.y
                );
            }
        }
        // All tile labels, including pinned Finish sketch / Abort, share one top.
        if let Some(first) = tiles.first() {
            for tile in &tiles {
                assert!(
                    (tile.rect.min.y - first.rect.min.y).abs() < 0.5,
                    "{place}: {} at {} vs {} at {}",
                    tile.text,
                    tile.rect.min.y,
                    first.text,
                    first.rect.min.y
                );
            }
        }
        // No content panel draws into the ribbon band.
        for text in &painted.texts {
            if text.clip.min.y >= ribbon.max.y - 0.5 || text.clip.max.y <= ribbon.min.y + 0.5 {
                continue;
            }
            if text.clip.min.y < 0.5 && text.clip.max.y >= size.y - 0.5 {
                continue; // full-screen overlays such as the view cube
            }
            assert!(
                text.clip.max.y <= ribbon.max.y + 0.5,
                "{place}: {:?} clip {:?} overlaps ribbon {:?}",
                text.text,
                text.clip,
                ribbon
            );
        }
        if mode == Mode::Model && tab == 0 {
            for caption in ["Create", "Modify", "Transform"] {
                let shown = painted
                    .texts
                    .iter()
                    .find(|t| t.text == caption && t.clip.max.y <= ribbon.max.y + 0.5)
                    .unwrap_or_else(|| panic!("{place}: {caption} caption missing"));
                if shown.rect.intersect(shown.clip).width() > 0. {
                    assert!(shown.rect.max.y <= ribbon.max.y, "{place}: {caption}");
                }
            }
        }
        assert!(
            !tiles.is_empty() || mode == Mode::Sketch,
            "{place}: no tiles"
        );
    }
}
#[test]
fn ribbon_labels_fit_inside_the_ribbon_at_the_reviewed_window() {
    check(egui::vec2(1360., 860.), 1.);
}
#[test]
fn ribbon_labels_fit_inside_the_ribbon_on_resize_and_retina_scaling() {
    for size in [
        egui::vec2(900., 640.),
        egui::vec2(1024., 600.),
        egui::vec2(1920., 1080.),
    ] {
        check(size, 1.);
    }
    check(egui::vec2(1360., 860.), 2.);
}
#[test]
fn reviewed_model_ribbon_ends_below_two_line_labels_and_captions() {
    let mut app = Application::new(Box::new(Storage)).unwrap();
    app.pending_files.clear();
    app.mode = Mode::Model;
    let ctx = egui::Context::default();
    let size = egui::vec2(1360., 860.);
    frame(&mut app, &ctx, size);
    let painted = frame(&mut app, &ctx, size);
    let find = |label: &str| {
        painted
            .texts
            .iter()
            .find(|t| t.text == label && t.clip.max.y <= painted.ribbon.max.y + 0.5)
            .unwrap_or_else(|| panic!("{label} missing"))
    };
    let treatment = find("Fillet / Chamfer");
    assert_eq!(treatment.lines, 2, "representative two-line label");
    let caption = find("Modify");
    assert!(treatment.rect.max.y < caption.rect.min.y);
    assert!(caption.rect.max.y < painted.ribbon.max.y);
    let content = painted
        .texts
        .iter()
        .find(|t| t.text == "Choose Sketch, or open a sketch from the document.")
        .expect("content guidance");
    assert!(content.clip.min.y >= painted.ribbon.max.y);
}

#[test]
fn compact_ribbon_uses_only_its_tool_and_caption_rows() {
    let mut app = Application::new(Box::new(Storage)).unwrap();
    app.pending_files.clear();
    let ctx = egui::Context::default();
    let size = egui::vec2(1024., 600.);
    app.mode = Mode::Model;
    app.tab = app
        .registry
        .workspace(Mode::Model)
        .tabs
        .iter()
        .position(|tab| tab.title == "Mechanical")
        .unwrap();
    frame(&mut app, &ctx, size);
    let mechanical = frame(&mut app, &ctx, size);
    assert!(
        mechanical.ribbon.height() <= 132.,
        "{:?}",
        mechanical.ribbon
    );
    app.tab = 0;
    frame(&mut app, &ctx, size);
    let common = frame(&mut app, &ctx, size);
    assert!(common.ribbon.height() <= 132., "{:?}", common.ribbon);
    assert!((common.ribbon.height() - mechanical.ribbon.height()).abs() < 0.5);
    assert!(
        !common
            .texts
            .iter()
            .any(|text| text.text == "Transfer features")
    );
    app.tab = app
        .registry
        .workspace(Mode::Model)
        .tabs
        .iter()
        .position(|tab| tab.title == "Relations")
        .unwrap();
    let relations = frame(&mut app, &ctx, size);
    assert!(relations.texts.iter().any(|text| {
        text.text == "Transfer features" && text.clip.expand(0.5).contains_rect(text.rect)
    }));
    app.mode = Mode::Sketch;
    app.tab = 0;
    frame(&mut app, &ctx, size);
    let inactive_sketch = frame(&mut app, &ctx, size);
    assert!(
        inactive_sketch.ribbon.height() <= 70.,
        "{:?}",
        inactive_sketch.ribbon
    );
}
