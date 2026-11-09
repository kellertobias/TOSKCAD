//! Lightweight samples of the selected definition and its current draft values.
use super::*;
use egui::{Pos2, Rect, Stroke, StrokeKind, Vec2};
use multicad_core::{
    document::Document,
    model::{ProfileSection, SectionEdge, SectionLoop},
};

const SAMPLE_BG: Color32 = Color32::from_rgb(19, 24, 33);

pub(super) fn show(ui: &mut egui::Ui, document: &Document, draft: Option<&Draft>) {
    ui.strong("Preview");
    ui.add_space(8.);
    let Some(draft) = draft else {
        ui.weak("Select a definition to see its sample.");
        return;
    };
    ui.label(egui::RichText::new(&draft.entity.name).size(16.));
    let size = ui.available_width();
    let (sample, _) = ui.allocate_exact_size(
        Vec2::new(size, (size * 0.78).clamp(84., 196.)),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(sample);
    painter.rect_filled(sample, 4., SAMPLE_BG);
    let bounds = sample.shrink(14.);
    if let Some(color) = material_color(&draft.entity) {
        let appearance = draft
            .style
            .as_ref()
            .and_then(|(_, style)| style.appearance.as_ref())
            .and_then(|link| document.get(link.id))
            .and_then(|entity| read(entity).ok().flatten());
        let color = match appearance {
            Some(LibraryDefinition::Appearance(appearance)) => appearance.color_rgba,
            _ => color,
        };
        swatch(&painter, bounds, color);
        show_color(ui, color);
        if let EntityData::Material {
            density_kg_m3: Some(density),
            ..
        } = draft.entity.data
        {
            ui.weak(format!("{density} kg/m³"));
        }
        return;
    }

    match read(&draft.entity) {
        Ok(Some(LibraryDefinition::Appearance(appearance))) => {
            swatch(&painter, bounds, appearance.color_rgba);
            show_color(ui, appearance.color_rgba);
            ui.weak(format!(
                "Roughness {:.2}  ·  Metallic {:.2}",
                appearance.roughness, appearance.metallic
            ));
        }
        Ok(Some(LibraryDefinition::Hatch(hatch))) => {
            hatch_sample(&painter, bounds, &hatch);
            ui.weak(format!(
                "{}  ·  {}°  ·  {} mm spacing",
                hatch.pattern.label(),
                hatch.angle_deg,
                hatch.spacing_mm
            ));
        }
        Ok(Some(LibraryDefinition::Stock(stock))) => {
            let material = document.get(stock.material).or_else(|| {
                draft
                    .companion
                    .as_ref()
                    .filter(|entity| entity.id == stock.material)
            });
            let color = material
                .and_then(material_color)
                .map_or(Color32::GRAY, to_color);
            stock_sample(ui, &painter, bounds, &stock, color);
            if let Some(material) = material {
                ui.weak(&material.name);
            }
        }
        _ => {
            ui.weak("Enter valid values to see the sample.");
        }
    }
}

fn material_color(entity: &Entity) -> Option<[u8; 4]> {
    match entity.data {
        EntityData::Material { color_rgba, .. } => Some(color_rgba),
        _ => None,
    }
}

fn to_color([r, g, b, a]: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn show_color(ui: &mut egui::Ui, [r, g, b, a]: [u8; 4]) {
    ui.label(egui::RichText::new(format!("#{r:02X}{g:02X}{b:02X}")).monospace());
    if a < 255 {
        ui.weak(format!("{}% opacity", (f32::from(a) * 100. / 255.).round()));
    }
}

fn swatch(painter: &egui::Painter, rect: Rect, color: [u8; 4]) {
    let offset = egui::vec2(8., 8.);
    let face = Rect::from_min_max(rect.min, rect.max - offset);
    let color = to_color(color);
    painter.rect_filled(face.translate(offset), 2., color.gamma_multiply(0.65));
    painter.rect_filled(face, 2., color);
}

pub(super) fn paint_swatch(
    painter: &egui::Painter,
    rect: Rect,
    material_override: Option<[u8; 4]>,
    entry: &Entity,
) {
    if let Some(base) = material_color(entry) {
        let color = material_override.unwrap_or(base);
        painter.rect_filled(rect, 2., to_color(color));
        painter.rect_stroke(rect, 2., Stroke::new(1., Color32::GRAY), StrokeKind::Inside);
    } else if let Ok(Some(LibraryDefinition::Hatch(hatch))) = read(entry) {
        hatch_sample(
            painter,
            rect,
            &HatchStyle {
                spacing_mm: 1.6,
                ..hatch
            },
        );
    } else if let Ok(Some(LibraryDefinition::Appearance(appearance))) = read(entry) {
        painter.rect_filled(rect, 2., to_color(appearance.color_rgba));
        painter.rect_stroke(rect, 2., Stroke::new(1., Color32::GRAY), StrokeKind::Inside);
    }
}

pub(super) fn hatch_picker(ui: &mut egui::Ui, hatch: &mut HatchStyle) {
    ui.horizontal_wrapped(|ui| {
        for pattern in HatchPattern::ALL {
            let selected = hatch.pattern == pattern;
            let response = ui.add(
                egui::Button::new("")
                    .selected(selected)
                    .min_size(Vec2::splat(38.)),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    selected,
                    pattern.label(),
                )
            });
            hatch_sample(
                ui.painter(),
                response.rect.shrink(5.),
                &HatchStyle {
                    color_rgb: hatch.color_rgb,
                    pattern,
                    angle_deg: if matches!(
                        pattern,
                        HatchPattern::Brick | HatchPattern::Concrete | HatchPattern::Grass
                    ) {
                        0.
                    } else {
                        45.
                    },
                    spacing_mm: 2.1,
                    line_width_mm: 0.28,
                },
            );
            if response.on_hover_text(pattern.label()).clicked() {
                hatch.pattern = pattern;
            }
        }
    });
    ui.weak(hatch.pattern.label());
}

fn hatch_sample(painter: &egui::Painter, rect: Rect, hatch: &HatchStyle) {
    painter.rect_filled(
        rect,
        2.,
        if hatch.color_rgb.is_some() {
            Color32::from_gray(245)
        } else {
            SAMPLE_BG
        },
    );
    let stroke = Stroke::new(
        (hatch.line_width_mm as f32 * 3.).clamp(0.75, 8.),
        hatch
            .color_rgb
            .map_or(Color32::LIGHT_GRAY, |[r, g, b]| Color32::from_rgb(r, g, b)),
    );
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    if hatch.pattern == HatchPattern::Solid {
        painter.rect_filled(rect, 2., stroke.color);
    } else if hatch.pattern != HatchPattern::None {
        let (sin, cos) = hatch.angle_deg.to_radians().sin_cos();
        let direction = egui::vec2(cos as f32, -sin as f32);
        let normal = egui::vec2(-direction.y, direction.x);
        let spacing = (hatch.spacing_mm as f32 * 3.).max(3.);
        let extent = rect.size().length();
        let count = ((extent * 0.5 / spacing).ceil() as i32 + 1).min(128);
        for i in -count..=count {
            let row = rect.center() + normal * (i as f32 * spacing);
            match hatch.pattern {
                HatchPattern::Brick => {
                    painter
                        .line_segment([row - direction * extent, row + direction * extent], stroke);
                    for j in -count..=count {
                        let joint = row + direction * ((2 * j + i.rem_euclid(2)) as f32 * spacing);
                        painter.line_segment([joint, joint + normal * spacing], stroke);
                    }
                }
                HatchPattern::Concrete | HatchPattern::Grass => {
                    for j in -count..=count {
                        let point =
                            row + direction * ((j as f32 + i.rem_euclid(2) as f32 * 0.5) * spacing);
                        let offset = |x, y| point + (direction * x - normal * y) * spacing;
                        if hatch.pattern == HatchPattern::Grass {
                            for tilt in [-0.20, 0., 0.20] {
                                painter.line_segment(
                                    [point, offset(tilt, if tilt == 0. { 0.31 } else { 0.23 })],
                                    stroke,
                                );
                            }
                        } else if (i + j) % 3 == 0 {
                            let a = offset(-0.18, -0.10);
                            let b = offset(0.14, -0.06);
                            let c = offset(0.04, 0.17);
                            painter.add(egui::Shape::closed_line(vec![a, b, c], stroke));
                        } else {
                            painter
                                .line_segment([offset(-0.035, -0.04), offset(0.035, 0.04)], stroke);
                        }
                    }
                }
                HatchPattern::Dots | HatchPattern::SmallCrosses | HatchPattern::Alternating
                    if hatch.pattern != HatchPattern::Alternating || i % 2 != 0 =>
                {
                    for j in -count..=count {
                        let point = row + direction * (j as f32 * spacing);
                        if hatch.pattern == HatchPattern::SmallCrosses {
                            let arm = spacing * 0.17;
                            for axis in [direction, normal] {
                                painter
                                    .line_segment([point - axis * arm, point + axis * arm], stroke);
                            }
                        } else {
                            painter.circle_filled(
                                point,
                                (stroke.width * 0.5).max(spacing * 0.045),
                                stroke.color,
                            );
                        }
                    }
                }
                _ => {
                    painter
                        .line_segment([row - direction * extent, row + direction * extent], stroke);
                    if hatch.pattern == HatchPattern::Cross {
                        let column = rect.center() + direction * (i as f32 * spacing);
                        painter.line_segment(
                            [column - normal * extent, column + normal * extent],
                            stroke,
                        );
                    }
                }
            }
        }
    }
    painter.rect_stroke(rect, 2., Stroke::new(1., Color32::GRAY), StrokeKind::Inside);
}

fn stock_sample(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    stock: &Stock,
    color: Color32,
) {
    let to_fit = |width: f64, height: f64| {
        let ratio = width / height;
        let size = if ratio > f64::from(rect.width() / rect.height()) {
            Vec2::new(rect.width(), (f64::from(rect.width()) / ratio) as f32)
        } else {
            Vec2::new((f64::from(rect.height()) * ratio) as f32, rect.height())
        };
        Rect::from_center_size(rect.center(), size)
    };
    match &stock.specification {
        StockKind::Panel {
            thickness_mm,
            length_mm,
            width_mm,
            grain,
        } => {
            let face = to_fit(*length_mm, *width_mm);
            painter.rect_filled(
                face.translate(egui::vec2(0., 5.)),
                1.,
                color.gamma_multiply(0.68),
            );
            painter.rect_filled(face, 1., color);
            if *grain != Grain::None {
                let along_length = *grain == Grain::AlongLength;
                let stroke = Stroke::new(1., color.gamma_multiply(0.66));
                for fraction in [0.25, 0.5, 0.75] {
                    let (start, end) = if along_length {
                        ((0.10, fraction), (0.90, fraction))
                    } else {
                        ((fraction, 0.10), (fraction, 0.90))
                    };
                    let point = |(x, y)| face.min + egui::vec2(x * face.width(), y * face.height());
                    painter.line_segment([point(start), point(end)], stroke);
                }
            }
            ui.weak(format!("{length_mm} × {width_mm} × {thickness_mm} mm"));
        }
        StockKind::Sheet {
            thickness_mm,
            length_mm,
            width_mm,
            ..
        } => {
            let face = to_fit(*length_mm, *width_mm);
            painter.rect_filled(face, 1., color);
            ui.weak(format!("{length_mm} × {width_mm} × {thickness_mm} mm"));
        }
        StockKind::Profile {
            width_mm,
            height_mm,
            length_mm,
            ..
        } => {
            if let Ok(profile) = ProfileSection::from_stock(stock) {
                section_sample(painter, to_fit(*width_mm, *height_mm), &profile);
            }
            ui.weak(format!(
                "{width_mm} × {height_mm} mm section  ·  {length_mm} mm long"
            ));
        }
        StockKind::Print {
            process,
            layer_height_mm,
            ..
        } => {
            let layer = rect.height() / 6.;
            for index in 0..6 {
                let y = rect.top() + index as f32 * layer;
                let strip = Rect::from_min_size(
                    Pos2::new(rect.left(), y),
                    egui::vec2(rect.width(), layer - 1.),
                );
                painter.rect_filled(strip, 2., color);
            }
            ui.weak(format!("{process}  ·  {layer_height_mm} mm layers"));
        }
    }
}

fn section_sample(painter: &egui::Painter, rect: Rect, profile: &ProfileSection) {
    let project = |[x, y]: [f64; 2]| {
        Pos2::new(
            rect.left() + (x / profile.width_mm) as f32 * rect.width(),
            rect.bottom() - (y / profile.height_mm) as f32 * rect.height(),
        )
    };
    let stroke = Stroke::new(1.5, Color32::LIGHT_GRAY);
    for boundary in std::iter::once(profile.outer(0.)).chain(profile.holes()) {
        match boundary {
            SectionLoop::Polygon(vertices) => {
                painter.add(egui::Shape::closed_line(
                    vertices.into_iter().map(project).collect(),
                    stroke,
                ));
            }
            SectionLoop::Circle { center, radius_mm } => {
                painter.circle_stroke(
                    project(center),
                    (radius_mm / profile.width_mm) as f32 * rect.width(),
                    stroke,
                );
            }
            SectionLoop::Curved(edges) => {
                for edge in edges {
                    match edge {
                        SectionEdge::Line { a, b } => {
                            painter.line_segment([project(a), project(b)], stroke);
                        }
                        SectionEdge::Arc {
                            center: [cx, cy],
                            a,
                            b,
                        } => {
                            let start = (a[1] - cy).atan2(a[0] - cx);
                            let end = (b[1] - cy).atan2(b[0] - cx);
                            let sweep = (end - start).rem_euclid(std::f64::consts::TAU);
                            let radius = (a[0] - cx).hypot(a[1] - cy);
                            let points = (0..=12).map(|i| {
                                let angle = start + sweep * f64::from(i) / 12.;
                                project([cx + radius * angle.cos(), cy + radius * angle.sin()])
                            });
                            painter.add(egui::Shape::line(points.collect(), stroke));
                        }
                    }
                }
            }
        }
    }
}
