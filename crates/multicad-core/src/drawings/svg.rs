//! Scalable vector page output in paper millimeters, from the shared page
//! render. Stale views and detached dimensions are refused, never drawn with
//! old values.
use super::DrawingEvaluation;
use super::render::{PageRender, render_page};
use crate::document::{Document, EntityId};
use std::fmt::Write;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
    Ok(render_svg(&render_page(
        document, revision, book, page, evaluation,
    )?))
}

pub fn render_svg(page: &PageRender) -> String {
    let [w, h] = page.size_mm;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">"#
    );
    for line in &page.polylines {
        let points: Vec<String> = line
            .points
            .iter()
            .map(|p| format!("{:.4},{:.4}", p[0], p[1]))
            .collect();
        let tag = if line.closed { "polygon" } else { "polyline" };
        let dash = if line.dashed {
            r#" stroke-dasharray="3 1.5""#
        } else {
            ""
        };
        let _ = writeln!(
            out,
            r#"<{tag} class="{}" fill="none" stroke="black" stroke-width="{}"{dash} points="{}"/>"#,
            line.layer.name(),
            line.width_mm,
            points.join(" ")
        );
    }
    for arc in &page.arcs {
        let sweep = arc.end_rad - arc.start_rad;
        let r = arc.radius_mm;
        if sweep.abs() >= std::f64::consts::TAU - 1e-9 {
            let _ = writeln!(
                out,
                r#"<circle class="{}" cx="{:.4}" cy="{:.4}" r="{r:.4}" fill="none" stroke="black" stroke-width="{}"/>"#,
                arc.layer.name(),
                arc.center[0],
                arc.center[1],
                arc.width_mm
            );
        } else {
            let p = |a: f64| [arc.center[0] + r * a.cos(), arc.center[1] + r * a.sin()];
            let (s, e) = (p(arc.start_rad), p(arc.end_rad));
            let large = u8::from(sweep.abs() > std::f64::consts::PI);
            let positive = u8::from(sweep > 0.);
            let _ = writeln!(
                out,
                r#"<path class="{}" fill="none" stroke="black" stroke-width="{}" d="M {:.4} {:.4} A {r:.4} {r:.4} 0 {large} {positive} {:.4} {:.4}"/>"#,
                arc.layer.name(),
                arc.width_mm,
                s[0],
                s[1],
                e[0],
                e[1]
            );
        }
    }
    for fill in &page.fills {
        let points: Vec<String> = fill
            .points
            .iter()
            .map(|p| format!("{:.4},{:.4}", p[0], p[1]))
            .collect();
        let _ = writeln!(
            out,
            r#"<polygon class="{}" fill="black" points="{}"/>"#,
            fill.layer.name(),
            points.join(" ")
        );
    }
    for text in &page.texts {
        let rotate = if text.angle_deg.abs() > 1e-9 {
            format!(
                r#" transform="rotate({:.4} {:.4} {:.4})""#,
                text.angle_deg, text.at[0], text.at[1]
            )
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            r#"<text class="{}" x="{:.4}" y="{:.4}" font-family="monospace" font-size="{:.3}"{rotate}>{}</text>"#,
            text.layer.name(),
            text.at[0],
            text.at[1],
            text.height_mm,
            escape(&text.text)
        );
    }
    let _ = writeln!(out, "</svg>");
    out
}
