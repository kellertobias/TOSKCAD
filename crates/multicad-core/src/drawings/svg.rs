//! Scalable vector page output in paper millimeters. The writer consumes only
//! a checked, current evaluation, so stale views and detached dimensions are
//! refused instead of drawn with old values.
use super::{
    AnnotationEvaluation, AnnotationGraphic, AnnotationKind, DrawingEvaluation, DrawingView,
    FONT_MM, LINE_MM, VectorRole, ViewEvaluation, draft_dimension, layout_page, sheet_graphic,
};
use crate::document::{Document, EntityId};
use std::fmt::Write;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn text(out: &mut String, at: [f64; 2], size: f64, angle: f64, value: &str, class: &str) {
    for (i, line) in value.lines().enumerate() {
        let y = at[1] + i as f64 * LINE_MM;
        let rotate = if angle.abs() > 1e-9 {
            format!(r#" transform="rotate({angle:.4} {:.4} {y:.4})""#, at[0])
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            r#"<text class="{class}" x="{:.4}" y="{y:.4}" font-family="monospace" font-size="{size:.3}"{rotate}>{}</text>"#,
            at[0],
            escape(line)
        );
    }
}
fn graphic(out: &mut String, g: &AnnotationGraphic, id: EntityId, class: &str) {
    let _ = writeln!(
        out,
        r#"<g class="{class}" data-id="{id}" stroke="black" stroke-width="0.18" fill="none">"#
    );
    for [a, b] in &g.lines {
        let _ = writeln!(
            out,
            r#"<line x1="{:.4}" y1="{:.4}" x2="{:.4}" y2="{:.4}"/>"#,
            a[0], a[1], b[0], b[1]
        );
    }
    for (center, r, start, end) in &g.arcs {
        let sweep = end - start;
        if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
            let _ = writeln!(
                out,
                r#"<circle cx="{:.4}" cy="{:.4}" r="{r:.4}"/>"#,
                center[0], center[1]
            );
        } else {
            let p = |a: f64| [center[0] + r * a.cos(), center[1] + r * a.sin()];
            let (s, e) = (p(*start), p(*end));
            let large = u8::from(sweep.abs() > std::f64::consts::PI);
            let positive = u8::from(sweep > 0.);
            let _ = writeln!(
                out,
                r#"<path d="M {:.4} {:.4} A {r:.4} {r:.4} 0 {large} {positive} {:.4} {:.4}"/>"#,
                s[0], s[1], e[0], e[1]
            );
        }
    }
    for (tip, dir) in &g.arrows {
        let back = [
            tip[0] - dir[0] * super::ARROW_MM,
            tip[1] - dir[1] * super::ARROW_MM,
        ];
        let n = [-dir[1] * 0.45, dir[0] * 0.45];
        let _ = writeln!(
            out,
            r#"<polygon fill="black" points="{:.4},{:.4} {:.4},{:.4} {:.4},{:.4}"/>"#,
            tip[0],
            tip[1],
            back[0] + n[0],
            back[1] + n[1],
            back[0] - n[0],
            back[1] - n[1]
        );
    }
    let _ = writeln!(out, "</g>");
    if !g.text.is_empty() {
        text(
            out,
            g.text_mm,
            super::ANNOTATION_TEXT_MM,
            g.text_angle_deg,
            &g.text,
            class,
        );
    }
}

/// Write one page as SVG in millimeters. `evaluation` must be current for
/// `document` at `revision`.
pub fn page_svg(
    document: &Document,
    revision: u64,
    book: EntityId,
    page: EntityId,
    evaluation: &DrawingEvaluation,
) -> Result<String, String> {
    let checked = evaluation.checked_page(document, revision, book, page)?;
    let drawing = super::read(document, book).map_err(|e| e.to_string())?;
    let (index, page) = drawing
        .pages
        .iter()
        .enumerate()
        .find(|(_, p)| p.id == page)
        .ok_or("Drawing page is missing")?;
    let layout = layout_page(page, index + 1, drawing.pages.len()).map_err(|e| e.to_string())?;
    let [w, h] = layout.paper_mm;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#
    );
    let [x0, y0, x1, y1] = layout.printable_mm;
    let _ = writeln!(
        out,
        r#"<rect class="border" x="{x0}" y="{y0}" width="{}" height="{}" fill="none" stroke="black" stroke-width="0.5"/>"#,
        x1 - x0,
        y1 - y0
    );
    for cell in layout.title_cells.iter().chain(&layout.notes) {
        let [cx0, cy0, cx1, cy1] = cell.rect_mm;
        if cell.label.is_empty() {
            text(
                &mut out,
                [cx0, cy0 + FONT_MM],
                FONT_MM,
                0.,
                &cell.lines.join("\n"),
                "note",
            );
            continue;
        }
        let _ = writeln!(
            out,
            r#"<rect class="title" x="{cx0}" y="{cy0}" width="{}" height="{}" fill="none" stroke="black" stroke-width="0.35"/>"#,
            cx1 - cx0,
            cy1 - cy0
        );
        let mut lines = vec![cell.label.clone()];
        lines.extend(cell.lines.iter().cloned());
        text(
            &mut out,
            [cx0 + 1., cy0 + FONT_MM + 0.5],
            FONT_MM,
            0.,
            &lines.join("\n"),
            "title",
        );
    }
    for (result, view) in &checked {
        let ViewEvaluation::Ready { vectors, .. } = result else {
            continue;
        };
        write_view(&mut out, view, vectors);
    }
    for annotation in &page.annotations {
        let class = match &annotation.kind {
            AnnotationKind::Dimension { .. } => "dimension",
            AnnotationKind::Note { .. } => "note",
            AnnotationKind::Leader { .. } => "leader",
            AnnotationKind::Weld { .. } => "weld",
            AnnotationKind::Table { .. } => "table",
            AnnotationKind::Draft { .. } => "draft",
            AnnotationKind::DraftDimension { .. } => "dimension",
        };
        let g = match &annotation.kind {
            AnnotationKind::Dimension { .. } => match evaluation.annotations.get(&annotation.id) {
                Some(AnnotationEvaluation::Ready { graphic }) => graphic.clone(),
                _ => return Err("A dimension is not evaluated".into()),
            },
            AnnotationKind::DraftDimension {
                a,
                b,
                offset_mm,
                precision,
            } => draft_dimension(page, *a, *b, *offset_mm, *precision),
            _ => sheet_graphic(annotation),
        };
        if let AnnotationKind::Draft { line_width_mm, .. } = &annotation.kind {
            let mut s = String::new();
            graphic(&mut s, &g, annotation.id, class);
            out.push_str(&s.replacen(
                r#"stroke-width="0.18""#,
                &format!(r#"stroke-width="{line_width_mm}""#),
                1,
            ));
        } else {
            graphic(&mut out, &g, annotation.id, class);
        }
    }
    let _ = writeln!(out, "</svg>");
    Ok(out)
}

fn write_view(out: &mut String, view: &DrawingView, vectors: &[super::DrawingVector]) {
    let _ = writeln!(out, r#"<g class="view" data-id="{}">"#, view.id);
    for vector in vectors {
        let points: Vec<String> = vector
            .preview_mm
            .iter()
            .map(|p| {
                let q = view.paper_point(*p);
                format!("{:.4},{:.4}", q[0], q[1])
            })
            .collect();
        let (class, dash) = match vector.role {
            VectorRole::Visible => ("visible", ""),
            VectorRole::Hidden => ("hidden", r#" stroke-dasharray="3 1.5""#),
            VectorRole::Section => ("section", ""),
            VectorRole::Hatch => ("hatch", ""),
        };
        let _ = writeln!(
            out,
            r#"<polyline class="{class}" fill="none" stroke="black" stroke-width="{}"{dash} points="{}"/>"#,
            vector.line_width_mm,
            points.join(" ")
        );
    }
    let _ = writeln!(out, "</g>");
}
