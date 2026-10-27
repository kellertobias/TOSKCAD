//! Original vector symbols on a consistent 24-point grid. No platform glyphs.
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
#[derive(Clone, Copy)]
pub enum Icon {
    Save,
    Undo,
    Redo,
    Sketch,
    Box,
    Cut,
    Edge,
    Measure,
    Library,
    Home,
    Fit,
    Settings,
    Page,
    Export,
    Link,
    New,
    Grid,
    Orthographic,
    Perspective,
    Wire,
    AllEdges,
    VisibleEdges,
    Filled,
}
pub fn button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    icon_button(ui, icon, label, 30.0)
}
fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 5, Color32::from_rgb(53, 64, 80));
    }
    let color = if ui.is_enabled() {
        Color32::from_rgb(201, 214, 232)
    } else {
        Color32::from_gray(91)
    };
    paint_sized(
        ui,
        icon,
        rect.center(),
        color,
        if size <= 24.0 { 16.0 } else { 24.0 },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}
pub fn compact_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    icon_button(ui, icon, label, 24.0)
}
pub fn labeled_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.into(),
        egui::FontId::proportional(12.0),
        Color32::WHITE,
    );
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(16.0 + 6.0 + galley.size().x + 12.0, 26.0),
        egui::Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 4, Color32::from_white_alpha(30));
    }
    paint_sized(
        ui,
        icon,
        Pos2::new(rect.left() + 14.0, rect.center().y),
        Color32::WHITE,
        16.0,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 28.0, rect.center().y - galley.size().y * 0.5),
        galley,
        Color32::WHITE,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}
pub fn tool(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(112.0, 64.0), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink(2.0), 5, Color32::from_rgb(42, 51, 65));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            5,
            Stroke::new(1.0, Color32::LIGHT_BLUE),
            egui::StrokeKind::Inside,
        );
    }
    let color = if ui.is_enabled() {
        Color32::from_rgb(201, 214, 232)
    } else {
        Color32::from_gray(91)
    };
    paint_sized(
        ui,
        icon,
        Pos2::new(rect.center().x, rect.top() + 18.0),
        color,
        24.0,
    );
    let galley = ui
        .painter()
        .layout(label.into(), egui::FontId::proportional(12.0), color, 100.0);
    ui.painter().galley(
        Pos2::new(rect.center().x - galley.size().x * 0.5, rect.top() + 35.0),
        galley,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}
pub(crate) fn paint_sized(ui: &egui::Ui, icon: Icon, center: Pos2, color: Color32, size: f32) {
    let p = ui.painter();
    let scale = size / 24.0;
    let stroke = Stroke::new((1.65 * scale).max(1.25), color);
    let v = |x: f32, y: f32| center + Vec2::new(x - 12.0, y - 12.0) * scale;
    let path = |points: &[[f32; 2]]| {
        p.add(egui::Shape::line(
            points.iter().map(|a| v(a[0], a[1])).collect(),
            stroke,
        ));
    };
    let line = |a: [f32; 2], b: [f32; 2]| path(&[a, b]);
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        p.rect_stroke(
            egui::Rect::from_min_size(v(x, y), Vec2::new(w, h) * scale),
            2,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    let circle = |x: f32, y: f32, r: f32| {
        p.circle_stroke(v(x, y), r * scale, stroke);
    };
    match icon {
        Icon::Save => {
            path(&[
                [4.0, 3.0],
                [17.0, 3.0],
                [21.0, 7.0],
                [21.0, 21.0],
                [3.0, 21.0],
                [3.0, 3.0],
                [4.0, 3.0],
            ]);
            rect(7.0, 3.0, 9.0, 6.0);
            rect(7.0, 14.0, 10.0, 7.0);
        }
        Icon::Undo | Icon::Redo => {
            let mirror = if matches!(icon, Icon::Redo) {
                -1.0
            } else {
                1.0
            };
            let q = |x: f32, y: f32| [12.0 + (x - 12.0) * mirror, y];
            path(&[q(8.0, 4.0), q(3.0, 9.0), q(8.0, 14.0)]);
            let points = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0;
                    let x = (1.0 - t).powi(2) * 3.0 + 2.0 * (1.0 - t) * t * 23.0 + t * t * 17.0;
                    let y = (1.0 - t).powi(2) * 9.0 + 2.0 * (1.0 - t) * t * 5.0 + t * t * 20.0;
                    let a = q(x, y);
                    v(a[0], a[1])
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
        }
        Icon::Sketch => {
            path(&[
                [4.0, 20.0],
                [5.0, 15.0],
                [16.0, 4.0],
                [20.0, 8.0],
                [9.0, 19.0],
                [4.0, 20.0],
            ]);
            line([13.0, 7.0], [17.0, 11.0]);
            line([4.0, 23.0], [21.0, 23.0]);
        }
        Icon::Box | Icon::Cut | Icon::Wire | Icon::AllEdges | Icon::VisibleEdges | Icon::Filled => {
            if matches!(icon, Icon::AllEdges | Icon::VisibleEdges | Icon::Filled) {
                p.add(egui::Shape::convex_polygon(
                    vec![
                        v(12.0, 2.0),
                        v(21.0, 7.0),
                        v(21.0, 17.0),
                        v(12.0, 22.0),
                        v(3.0, 17.0),
                        v(3.0, 7.0),
                    ],
                    Color32::from_rgb(73, 125, 167),
                    Stroke::NONE,
                ));
            }
            if !matches!(icon, Icon::Filled) {
                path(&[
                    [12.0, 2.0],
                    [21.0, 7.0],
                    [21.0, 17.0],
                    [12.0, 22.0],
                    [3.0, 17.0],
                    [3.0, 7.0],
                    [12.0, 2.0],
                ]);
                path(&[[3.0, 7.0], [12.0, 12.0], [21.0, 7.0]]);
                line([12.0, 12.0], [12.0, 22.0]);
            }
            if matches!(icon, Icon::Cut) {
                line([9.0, 6.0], [15.0, 6.0]);
                line([9.0, 6.0], [9.0, 10.0]);
            }
            if matches!(icon, Icon::Wire | Icon::AllEdges) {
                for (a, b) in [
                    ([3.0, 17.0], [12.0, 12.0]),
                    ([12.0, 12.0], [21.0, 17.0]),
                    ([12.0, 12.0], [12.0, 2.0]),
                ] {
                    let m = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                    line(a, m);
                }
            }
        }
        Icon::Edge => {
            path(&[[3.0, 3.0], [3.0, 21.0], [21.0, 21.0]]);
            let points = (0..=20)
                .map(|i| {
                    let a = std::f32::consts::PI + i as f32 / 20.0 * std::f32::consts::FRAC_PI_2;
                    v(20.0 + a.cos() * 12.0, 4.0 - a.sin() * 12.0)
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
        }
        Icon::Measure => {
            path(&[
                [3.0, 15.0],
                [17.0, 3.0],
                [22.0, 9.0],
                [8.0, 21.0],
                [3.0, 15.0],
            ]);
            for (x, y) in [(8.0, 11.0), (12.0, 7.5), (16.0, 4.0)] {
                line([x, y], [x + 2.0, y + 2.5]);
            }
        }
        Icon::Library => {
            rect(3.0, 4.0, 5.0, 17.0);
            rect(10.0, 4.0, 5.0, 17.0);
            path(&[
                [16.0, 5.0],
                [20.0, 4.0],
                [23.0, 20.0],
                [19.0, 21.0],
                [16.0, 5.0],
            ]);
        }
        Icon::Home => {
            path(&[[2.0, 11.0], [12.0, 3.0], [22.0, 11.0]]);
            path(&[
                [5.0, 9.0],
                [5.0, 21.0],
                [10.0, 21.0],
                [10.0, 15.0],
                [14.0, 15.0],
                [14.0, 21.0],
                [19.0, 21.0],
                [19.0, 9.0],
            ]);
        }
        Icon::Fit => {
            for points in [
                &[[3.0, 9.0], [3.0, 3.0], [9.0, 3.0]][..],
                &[[15.0, 3.0], [21.0, 3.0], [21.0, 9.0]],
                &[[21.0, 15.0], [21.0, 21.0], [15.0, 21.0]],
                &[[9.0, 21.0], [3.0, 21.0], [3.0, 15.0]],
            ] {
                path(points);
            }
        }
        Icon::Settings => {
            circle(12.0, 12.0, 3.0);
            let points = (0..=32)
                .map(|i| {
                    let a = i as f32 / 32.0 * std::f32::consts::TAU;
                    let r = if i % 4 == 0 || i % 4 == 1 { 10.0 } else { 7.5 };
                    v(12.0 + r * a.cos(), 12.0 + r * a.sin())
                })
                .collect();
            p.add(egui::Shape::line(points, stroke));
        }
        Icon::Page | Icon::New | Icon::Export => {
            path(&[
                [4.0, 2.0],
                [15.0, 2.0],
                [20.0, 7.0],
                [20.0, 22.0],
                [4.0, 22.0],
                [4.0, 2.0],
            ]);
            path(&[[15.0, 2.0], [15.0, 7.0], [20.0, 7.0]]);
            if matches!(icon, Icon::New) {
                line([8.0, 14.0], [16.0, 14.0]);
                line([12.0, 10.0], [12.0, 18.0]);
            } else if matches!(icon, Icon::Export) {
                path(&[[11.0, 11.0], [16.0, 16.0], [11.0, 21.0]]);
                line([7.0, 16.0], [16.0, 16.0]);
            } else {
                line([8.0, 12.0], [16.0, 12.0]);
                line([8.0, 16.0], [16.0, 16.0]);
            }
        }
        Icon::Link => {
            let a = egui::Rect::from_min_size(v(2.0, 8.0), Vec2::new(12.0, 8.0) * scale);
            let b = egui::Rect::from_min_size(v(10.0, 8.0), Vec2::new(12.0, 8.0) * scale);
            p.rect_stroke(a, 4, stroke, egui::StrokeKind::Inside);
            p.rect_stroke(b, 4, stroke, egui::StrokeKind::Inside);
            line([8.0, 12.0], [16.0, 12.0]);
        }
        Icon::Grid => {
            rect(3.0, 3.0, 18.0, 18.0);
            for n in [9.0, 15.0] {
                line([n, 3.0], [n, 21.0]);
                line([3.0, n], [21.0, n]);
            }
        }
        Icon::Orthographic => {
            rect(4.0, 4.0, 13.0, 13.0);
            path(&[[8.0, 21.0], [21.0, 21.0], [21.0, 8.0]]);
            line([17.0, 4.0], [21.0, 8.0]);
            line([17.0, 17.0], [21.0, 21.0]);
            line([4.0, 17.0], [8.0, 21.0]);
        }
        Icon::Perspective => {
            path(&[
                [3.0, 3.0],
                [21.0, 7.0],
                [21.0, 17.0],
                [3.0, 21.0],
                [3.0, 3.0],
            ]);
            path(&[[8.0, 4.0], [8.0, 20.0]]);
            line([3.0, 12.0], [21.0, 12.0]);
        }
    }
}
