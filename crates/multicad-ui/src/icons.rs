//! Original vector symbols on a consistent 24-point grid. No platform glyphs.
use crate::svg_icon::SvgIcon;
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
#[derive(Clone, Copy)]
pub enum Icon {
    User,
    Hammer,
    Keyboard,
    Transform,
    Scale,
    Rotate,
    Mirror,
    LinearPattern,
    CircularPattern,
    Trim,
    Extend,
    ArrowUp,
    ArrowDown,
    Trash,
    Save,
    Undo,
    Redo,
    Sketch,
    Line,
    Rectangle,
    Circle,
    Arc,
    Polyline,
    Spline,
    Construction,
    Box,
    Cylinder,
    Extrude,
    Revolve,
    Boolean,
    Shell,
    Thickness,
    Sweep,
    Loft,
    Draft,
    Assembly,
    Joint,
    Motion,
    Interference,
    Explode,
    Transfer,
    Cut,
    Split,
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
    Section,
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
    tool_with_hint(ui, icon, label, label)
}
pub fn tool_with_hint(ui: &mut egui::Ui, icon: Icon, label: &str, hint: &str) -> egui::Response {
    tool_glyph(ui, Glyph::Vector(icon), label, hint, false)
}
/// A ribbon symbol: a retained vector motif or reviewed catalog artwork.
#[derive(Clone, Copy)]
pub enum Glyph {
    Vector(Icon),
    Art(&'static str, &'static SvgIcon),
}
/// Visual state of a ribbon tool button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Enabled,
    Disabled,
    Hovered,
    Selected,
}
pub const TOOL_COLOR: Color32 = Color32::from_rgb(201, 214, 232);
pub const TOOL_DISABLED: Color32 = Color32::from_gray(91);
pub const TOOL_HOVER: Color32 = Color32::from_rgb(238, 244, 252);
pub const TOOL_SELECTED: Color32 = Color32::WHITE;
/// Symbol tint for a state. Artwork with its own reviewed color (the green
/// Finish / Apply check) keeps it whenever it is usable.
pub fn glyph_color(glyph: Glyph, state: ToolState) -> Color32 {
    if state == ToolState::Disabled {
        return TOOL_DISABLED;
    }
    if let Glyph::Art(_, art) = glyph
        && let Some([r, g, b]) = art.color
    {
        return Color32::from_rgb(r, g, b);
    }
    match state {
        ToolState::Enabled => TOOL_COLOR,
        ToolState::Hovered => TOOL_HOVER,
        ToolState::Selected => TOOL_SELECTED,
        ToolState::Disabled => TOOL_DISABLED,
    }
}
pub fn tool_glyph(
    ui: &mut egui::Ui,
    glyph: Glyph,
    label: &str,
    hint: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(76.0, 92.0), egui::Sense::click());
    let state = if !ui.is_enabled() {
        ToolState::Disabled
    } else if selected {
        ToolState::Selected
    } else if response.hovered() {
        ToolState::Hovered
    } else {
        ToolState::Enabled
    };
    if selected {
        ui.painter()
            .rect_filled(rect.shrink(1.0), 4, Color32::from_rgb(36, 62, 96));
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            4,
            Stroke::new(1.0, Color32::from_rgb(110, 160, 220)),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 3, Color32::from_rgb(42, 51, 65));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            5,
            Stroke::new(1.0, Color32::LIGHT_BLUE),
            egui::StrokeKind::Inside,
        );
    }
    let color = glyph_color(glyph, state);
    paint_glyph(
        ui,
        glyph,
        Pos2::new(rect.center().x, rect.top() + rect.height() / 3.0 + 4.0),
        color,
        if matches!(glyph, Glyph::Art(..)) {
            30.0
        } else {
            24.0
        },
    );
    let text = if state == ToolState::Disabled {
        TOOL_DISABLED
    } else {
        TOOL_COLOR
    };
    let galley = ui
        .painter()
        .layout(label.into(), egui::FontId::proportional(11.0), text, 64.0);
    ui.painter().galley(
        Pos2::new(
            rect.center().x - galley.size().x * 0.5,
            rect.top() + rect.height() * 2.0 / 3.0,
        ),
        galley,
        text,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
    });
    response.on_hover_text(hint)
}
/// Paint a glyph centred at `center` with an edge length of `size` points.
pub fn paint_glyph(ui: &egui::Ui, glyph: Glyph, center: Pos2, color: Color32, size: f32) {
    match glyph {
        Glyph::Vector(icon) => paint_sized(ui, icon, center, color, size),
        Glyph::Art(id, art) => {
            let pixels = (size * ui.ctx().pixels_per_point())
                .round()
                .clamp(8.0, 256.0) as usize;
            let texture = artwork_texture(ui.ctx(), id, art, pixels);
            ui.painter().image(
                texture.id(),
                egui::Rect::from_center_size(center, Vec2::splat(size)),
                egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                color,
            );
        }
    }
}
#[derive(Clone, Default)]
struct ArtworkTextures(std::collections::HashMap<(&'static str, usize), egui::TextureHandle>);
/// White alpha-mask texture per artwork and pixel size; tinted when painted.
fn artwork_texture(
    ctx: &egui::Context,
    id: &'static str,
    art: &SvgIcon,
    pixels: usize,
) -> egui::TextureHandle {
    let key = egui::Id::new("multicad-artwork-textures");
    if let Some(texture) = ctx.data(|data| {
        data.get_temp::<ArtworkTextures>(key)
            .and_then(|cache| cache.0.get(&(id, pixels)).cloned())
    }) {
        return texture;
    }
    let mask = art.rasterize(pixels);
    let rgba: Vec<u8> = mask
        .alpha
        .iter()
        .flat_map(|alpha| [255, 255, 255, *alpha])
        .collect();
    let texture = ctx.load_texture(
        format!("artwork-{id}-{pixels}"),
        egui::ColorImage::from_rgba_unmultiplied([mask.size, mask.size], &rgba),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<ArtworkTextures>(key)
            .0
            .insert((id, pixels), texture.clone());
    });
    texture
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
        Icon::User => {
            circle(12., 7., 3.5);
            path(&[
                [4., 21.],
                [4., 18.],
                [6., 14.],
                [9., 12.5],
                [15., 12.5],
                [18., 14.],
                [20., 18.],
                [20., 21.],
            ]);
        }
        Icon::Section => {
            path(&[
                [4., 6.],
                [16., 6.],
                [20., 10.],
                [20., 19.],
                [8., 19.],
                [4., 15.],
                [4., 6.],
            ]);
            line([2., 15.], [22., 9.]);
            line([8., 13.], [11., 17.]);
            line([13., 11.], [16., 15.]);
        }
        Icon::Hammer => {
            path(&[[3., 19.], [12., 8.], [16., 11.], [7., 22.], [3., 19.]]);
            path(&[
                [10., 6.],
                [15., 2.],
                [22., 7.],
                [19., 12.],
                [15., 9.],
                [13., 11.],
                [9., 8.],
                [10., 6.],
            ]);
        }
        Icon::Keyboard => {
            rect(2., 5., 20., 14.);
            for y in [9., 12.] {
                for x in [6., 10., 14., 18.] {
                    line([x, y], [x + 0.5, y]);
                }
            }
            line([7., 16.], [17., 16.]);
        }

        Icon::Transform => {
            line([5., 19.], [20., 19.]);
            path(&[[16., 15.], [20., 19.], [16., 23.]]);
            line([5., 19.], [5., 3.]);
            path(&[[1., 7.], [5., 3.], [9., 7.]]);
            line([5., 19.], [16., 8.]);
            path(&[[10., 8.], [16., 8.], [16., 14.]]);
        }
        Icon::Scale => {
            rect(4., 13., 7., 7.);
            path(&[[4., 9.], [4., 4.], [20., 4.], [20., 20.], [15., 20.]]);
            path(&[[10., 14.], [18., 6.], [13., 6.]]);
            path(&[[18., 6.], [18., 11.]]);
        }
        Icon::Extrude => {
            path(&[[3., 18.], [11., 22.], [21., 17.], [13., 13.], [3., 18.]]);
            path(&[[3., 10.], [11., 14.], [21., 9.], [13., 5.], [3., 10.]]);
            for (a, b) in [
                ([3., 10.], [3., 18.]),
                ([11., 14.], [11., 22.]),
                ([21., 9.], [21., 17.]),
            ] {
                line(a, b);
            }
            line([7., 11.], [7., 2.]);
            path(&[[4., 5.], [7., 2.], [10., 5.]]);
        }
        Icon::Revolve => {
            for y in [2., 7., 12., 17.] {
                line([12., y], [12., y + 3.]);
            }
            path(&[[8., 5.], [4., 5.], [4., 18.], [8., 18.]]);
            path(&[
                [8., 8.],
                [17., 6.],
                [22., 10.],
                [19., 15.],
                [14., 17.],
                [9., 16.],
            ]);
            path(&[[13., 13.], [9., 16.], [13., 19.]]);
        }
        Icon::Boolean => {
            path(&[
                [2., 8.],
                [8., 5.],
                [14., 8.],
                [14., 15.],
                [8., 18.],
                [2., 15.],
                [2., 8.],
            ]);
            path(&[
                [10., 10.],
                [16., 7.],
                [22., 10.],
                [22., 17.],
                [16., 20.],
                [10., 17.],
                [10., 10.],
            ]);
            path(&[[2., 8.], [8., 11.], [14., 8.]]);
            line([8., 11.], [8., 18.]);
            path(&[[10., 10.], [16., 13.], [22., 10.]]);
            line([16., 13.], [16., 20.]);
        }
        Icon::Shell => {
            path(&[
                [3., 7.],
                [12., 3.],
                [21., 7.],
                [21., 18.],
                [12., 22.],
                [3., 18.],
                [3., 7.],
            ]);
            path(&[[3., 7.], [12., 11.], [21., 7.]]);
            line([12., 11.], [12., 22.]);
            path(&[[6., 7.], [12., 5.], [18., 7.], [12., 9.], [6., 7.]]);
            path(&[[6., 7.], [6., 16.], [10., 18.]]);
            path(&[[18., 7.], [18., 16.], [14., 18.]]);
        }
        Icon::Thickness => {
            path(&[[3., 15.], [13., 10.], [21., 14.], [11., 19.], [3., 15.]]);
            path(&[[3., 10.], [13., 5.], [21., 9.], [11., 14.], [3., 10.]]);
            line([3., 10.], [3., 15.]);
            line([11., 14.], [11., 19.]);
            line([21., 9.], [21., 14.]);
            line([18., 2.], [18., 6.]);
            path(&[[16., 4.], [18., 6.], [20., 4.]]);
            line([18., 22.], [18., 18.]);
            path(&[[16., 20.], [18., 18.], [20., 20.]]);
        }
        Icon::Sweep => {
            path(&[[3., 19.], [3., 11.], [5., 7.], [9., 5.], [18., 5.]]);
            path(&[[9., 19.], [9., 12.], [11., 11.], [18., 11.]]);
            path(&[[18., 5.], [21., 6.], [22., 8.], [21., 10.], [18., 11.]]);
            path(&[
                [3., 19.],
                [4., 21.],
                [6., 22.],
                [8., 21.],
                [9., 19.],
                [8., 17.],
                [6., 16.],
                [4., 17.],
                [3., 19.],
            ]);
            path(&[[5., 19.], [6., 18.], [7., 19.], [6., 20.], [5., 19.]]);
        }
        Icon::Loft => {
            path(&[[3., 18.], [10., 14.], [21., 17.], [14., 22.], [3., 18.]]);
            path(&[[7., 5.], [12., 2.], [18., 5.], [13., 8.], [7., 5.]]);
            line([3., 18.], [7., 5.]);
            line([10., 14.], [12., 2.]);
            line([21., 17.], [18., 5.]);
            line([14., 22.], [13., 8.]);
        }
        Icon::Draft => {
            path(&[[4., 20.], [8., 4.], [16., 4.], [21., 20.], [4., 20.]]);
            line([12., 4.], [12., 20.]);
            path(&[[2., 22.], [22., 22.]]);
            path(&[[16., 17.], [18., 16.], [20., 17.]]);
            path(&[[17., 13.], [18., 16.], [15., 16.]]);
        }
        Icon::Rotate => {
            path(&[
                [6., 7.],
                [11., 3.],
                [18., 5.],
                [21., 11.],
                [19., 18.],
                [13., 21.],
                [6., 18.],
            ]);
            path(&[[6., 2.], [6., 7.], [11., 7.]]);
            circle(12., 12., 1.);
        }
        Icon::Mirror => {
            for y in [2., 7., 12., 17.] {
                line([12., y], [12., y + 3.]);
            }
            path(&[[3., 18.], [3., 6.], [8., 18.], [3., 18.]]);
            path(&[[21., 18.], [21., 6.], [16., 18.], [21., 18.]]);
        }
        Icon::LinearPattern => {
            for x in [2., 10., 18.] {
                rect(x, 8., 4., 8.);
            }
            line([4., 20.], [20., 20.]);
            path(&[[17., 18.], [20., 20.], [17., 22.]]);
        }
        Icon::CircularPattern => {
            circle(12., 12., 1.);
            for angle in [
                0.,
                std::f32::consts::FRAC_PI_2,
                std::f32::consts::PI,
                3. * std::f32::consts::FRAC_PI_2,
            ] {
                circle(12. + 8. * f32::cos(angle), 12. + 8. * f32::sin(angle), 2.);
            }
        }
        Icon::Trim => {
            line([3., 8.], [21., 8.]);
            line([3., 16.], [21., 16.]);
            line([8., 3.], [8., 8.]);
            line([16., 16.], [16., 21.]);
            path(&[[10., 10.], [14., 14.]]);
            path(&[[10., 14.], [14., 10.]]);
        }
        Icon::Extend => {
            line([3., 17.], [11., 17.]);
            for x in [12., 16.] {
                line([x, 17.], [x + 2., 17.]);
            }
            line([21., 3.], [21., 21.]);
            path(&[[17., 14.], [21., 17.], [17., 20.]]);
        }
        Icon::Assembly => {
            for (x, y) in [(2., 11.), (9., 3.), (16., 11.)] {
                path(&[
                    [x, y + 2.],
                    [x + 3., y],
                    [x + 6., y + 2.],
                    [x + 6., y + 7.],
                    [x + 3., y + 9.],
                    [x, y + 7.],
                    [x, y + 2.],
                ]);
                path(&[[x, y + 2.], [x + 3., y + 4.], [x + 6., y + 2.]]);
                line([x + 3., y + 4.], [x + 3., y + 9.]);
            }
        }
        Icon::Joint => {
            path(&[[2., 4.], [7., 4.], [7., 10.], [11., 10.]]);
            path(&[[22., 20.], [17., 20.], [17., 14.], [13., 14.]]);
            circle(12., 12., 3.);
            line([12., 3.], [12., 7.]);
            line([12., 17.], [12., 21.]);
        }
        Icon::Transfer => {
            rect(3., 2., 18., 6.);
            rect(3., 16., 18., 6.);
            for x in [8., 16.] {
                circle(x, 5., 1.4);
                circle(x, 19., 1.4);
                line([x, 10.], [x, 14.]);
                path(&[[x - 1.5, 12.5], [x, 14.], [x + 1.5, 12.5]]);
            }
        }
        Icon::Motion => {
            rect(3., 10., 6., 7.);
            line([3., 21.], [21., 21.]);
            path(&[[12., 17.], [20., 17.], [17., 14.]]);
            path(&[[6., 6.], [11., 3.], [17., 4.], [21., 9.]]);
            path(&[[17., 9.], [21., 9.], [21., 5.]]);
        }
        Icon::Interference => {
            rect(2., 5., 12., 12.);
            rect(10., 9., 12., 12.);
            line([10., 10.], [14., 14.]);
            line([10., 14.], [14., 10.]);
            path(&[[16., 3.], [20., 3.], [20., 7.]]);
        }
        Icon::Explode => {
            rect(9., 9., 6., 6.);
            path(&[[7., 7.], [2., 2.], [2., 6.]]);
            line([2., 2.], [6., 2.]);
            path(&[[17., 7.], [22., 2.], [18., 2.]]);
            line([22., 2.], [22., 6.]);
            path(&[[7., 17.], [2., 22.], [6., 22.]]);
            line([2., 22.], [2., 18.]);
            path(&[[17., 17.], [22., 22.], [22., 18.]]);
            line([22., 22.], [18., 22.]);
        }
        Icon::ArrowUp => path(&[[5., 15.], [12., 8.], [19., 15.]]),
        Icon::ArrowDown => path(&[[5., 9.], [12., 16.], [19., 9.]]),
        Icon::Trash => {
            line([4., 6.], [20., 6.]);
            rect(6., 6., 12., 15.);
            path(&[[9., 6.], [9., 3.], [15., 3.], [15., 6.]]);
            line([10., 10.], [10., 17.]);
            line([14., 10.], [14., 17.]);
        }

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
        Icon::Line => {
            line([4., 20.], [20., 4.]);
            circle(4., 20., 1.5);
            circle(20., 4., 1.5);
        }
        Icon::Rectangle => {
            rect(3., 6., 18., 12.);
            circle(3., 6., 1.2);
            circle(21., 18., 1.2);
        }
        Icon::Circle => {
            circle(12., 12., 8.);
            line([10., 12.], [14., 12.]);
            line([12., 10.], [12., 14.]);
        }
        Icon::Arc => {
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let angle = std::f32::consts::PI * (0.15 + i as f32 / 24. * 1.4);
                    [12. + 8. * angle.cos(), 12. - 8. * angle.sin()]
                })
                .collect();
            path(&points);
            for point in [points[0], points[24]] {
                circle(point[0], point[1], 1.3);
            }
            line([10., 12.], [14., 12.]);
            line([12., 10.], [12., 14.]);
        }
        Icon::Polyline => {
            let points = [[3., 18.], [8., 5.], [15., 15.], [21., 7.]];
            path(&points);
            for point in points {
                circle(point[0], point[1], 1.2);
            }
        }
        Icon::Spline => {
            let points: Vec<_> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.;
                    let s = 1. - t;
                    [
                        3. * s * s * s
                            + 3. * 8. * s * s * t
                            + 3. * 15. * s * t * t
                            + 21. * t * t * t,
                        18. * s * s * s
                            + 3. * 1. * s * s * t
                            + 3. * 23. * s * t * t
                            + 6. * t * t * t,
                    ]
                })
                .collect();
            path(&points);
            circle(3., 18., 1.2);
            circle(21., 6., 1.2);
            line([3., 18.], [8., 1.]);
            line([15., 23.], [21., 6.]);
            circle(8., 1., 1.);
            circle(15., 23., 1.);
        }
        Icon::Construction => {
            for x in [2., 8., 14., 20.] {
                line([x, 12.], [x + 2., 12.]);
            }
            for y in [2., 8., 14., 20.] {
                line([12., y], [12., y + 2.]);
            }
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
        Icon::Cylinder => {
            let ellipse = |cy: f32, start: usize, end: usize| {
                let points = (start..=end)
                    .map(|step| {
                        let angle = step as f32 * std::f32::consts::TAU / 32.;
                        v(12. + 8. * angle.cos(), cy + 3. * angle.sin())
                    })
                    .collect();
                p.add(egui::Shape::line(points, stroke));
            };
            ellipse(5., 0, 32);
            line([4., 5.], [4., 19.]);
            line([20., 5.], [20., 19.]);
            ellipse(19., 0, 16);
        }
        Icon::Split => {
            path(&[[2., 7.], [10., 3.], [10., 21.], [2., 17.], [2., 7.]]);
            path(&[[14., 3.], [22., 7.], [22., 17.], [14., 21.], [14., 3.]]);
            for y in [3., 8., 13., 18.] {
                line([12., y], [12., y + 2.]);
            }
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

pub fn mini_button(ui: &mut egui::Ui, icon: Icon, label: &str) -> egui::Response {
    icon_button(ui, icon, label, 14.)
}
#[cfg(test)]
mod ribbon_tests {
    use super::*;
    #[test]
    fn two_line_labels_fit_inside_equal_height_tiles() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            for label in [
                "Apply and rebuild",
                "Fillet / chamfer",
                "Attached fastener",
                "Projection / section",
            ] {
                let response = tool(ui, Icon::Hammer, label);
                let text = ui.painter().layout(
                    label.into(),
                    egui::FontId::proportional(11.),
                    Color32::WHITE,
                    64.,
                );
                assert_eq!(response.rect.height(), 92.);
                assert!(
                    response.rect.height() * 2. / 3. + text.size().y <= response.rect.height() - 4.,
                    "{label}"
                );
            }
        })
        .drop_without_applying_deltas();
    }
}
