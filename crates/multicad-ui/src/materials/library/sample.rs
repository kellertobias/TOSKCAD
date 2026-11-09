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
                "{:?}  ·  {}°  ·  {} mm spacing",
                hatch.pattern, hatch.angle_deg, hatch.spacing_mm
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

fn hatch_sample(painter: &egui::Painter, rect: Rect, hatch: &HatchStyle) {
    let boundary = [
        [0., 0.],
        [f64::from(rect.width()), 0.],
        [f64::from(rect.width()), f64::from(rect.height())],
        [0., f64::from(rect.height())],
    ];
    painter.rect_stroke(rect, 1., Stroke::new(1., Color32::GRAY), StrokeKind::Inside);
    let angle = hatch.angle_deg.rem_euclid(360.);
    let angles = [angle, angle + 90.];
    let count = match hatch.pattern {
        HatchPattern::None => 0,
        HatchPattern::Lines => 1,
        HatchPattern::Cross => 2,
    };
    let stroke = Stroke::new(
        (hatch.line_width_mm as f32 * 3.).clamp(0.75, 8.),
        Color32::LIGHT_GRAY,
    );
    for angle in angles.into_iter().take(count) {
        for line in
            multicad_core::drawings::hatch_lines(&boundary, angle, (hatch.spacing_mm * 3.).max(3.))
        {
            painter.line_segment(
                line.map(|[x, y]| Pos2::new(rect.left() + x as f32, rect.bottom() - y as f32)),
                stroke,
            );
        }
    }
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
